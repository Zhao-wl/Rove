//! Multi-chord key state (#14), matching VS Code `AbstractKeybindingService`:
//!
//! * a key whose best rule needs more chords enters chord mode (`Pending`);
//! * the next key completes the chord (`Command`) or, if nothing matches, aborts it (`Aborted`) —
//!   `Escape` is not special: unless it is bound as a second chord it aborts like any other key;
//! * chord mode ends after 5 seconds without a key (`CHORD_TIMEOUT`) or when the window loses focus
//!   (`cancel`);
//! * bare modifier presses never change the state.
//!
//! Time is passed in by the caller so the logic stays deterministic and testable.
use crate::{
    keys::is_modifier_key, CommandRegistry, ContextKeys, Invocation, KeyResolution, KeyStroke,
    Keybindings,
};
use rove_base::Result;
use std::time::{Duration, Instant};

/// VS Code leaves chord mode after 5 seconds without interaction.
pub const CHORD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum KeyOutcome {
    /// Not a keybinding: let the key reach the focused control (typing, navigation...).
    Unhandled,
    /// First chord(s) of a multi-chord binding; the key is consumed. Carries the keys so far.
    Pending(Vec<KeyStroke>),
    /// Run this command; the key is consumed.
    Command(Invocation),
    /// The sequence pressed in chord mode is not bound; the key is consumed and chord mode ends.
    Aborted(Vec<KeyStroke>),
}

#[derive(Default)]
pub struct ChordState {
    pending: Vec<KeyStroke>,
    last: Option<Instant>,
}
impl ChordState {
    pub fn pending(&self) -> &[KeyStroke] {
        &self.pending
    }
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    /// Leave chord mode (focus lost). Returns whether chord mode was active.
    pub fn cancel(&mut self) -> bool {
        let was = self.is_pending();
        self.pending.clear();
        self.last = None;
        was
    }
    /// Leave chord mode if it timed out. Returns whether it just ended.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if self.is_pending() && now.duration_since(last) >= CHORD_TIMEOUT => {
                self.cancel()
            }
            _ => false,
        }
    }
    pub fn press(
        &mut self,
        key: KeyStroke,
        now: Instant,
        bindings: &dyn Keybindings,
        context: &dyn ContextKeys,
        commands: &CommandRegistry,
    ) -> Result<KeyOutcome> {
        if is_modifier_key(&key.key) {
            return Ok(KeyOutcome::Unhandled);
        }
        self.tick(now);
        let mut sequence = std::mem::take(&mut self.pending);
        let in_chord = !sequence.is_empty();
        sequence.push(key);
        self.last = None;
        Ok(match bindings.resolve(&sequence, context, commands)? {
            KeyResolution::Command(invocation) => KeyOutcome::Command(invocation),
            KeyResolution::PendingChord => {
                self.pending = sequence.clone();
                self.last = Some(now);
                KeyOutcome::Pending(sequence)
            }
            KeyResolution::NoMatch if in_chord => KeyOutcome::Aborted(sequence),
            KeyResolution::NoMatch => KeyOutcome::Unhandled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        keybindings::{KeybindingService, RawRule},
        keys::parse_chord,
        Command, ContextSnapshot,
    };
    use rove_base::Value;

    fn setup() -> (KeybindingService, CommandRegistry) {
        let rule = |key: &str, command: &str| RawRule {
            key: Some(key.into()),
            command: command.into(),
            when: None,
            args: Value::Null,
        };
        let s = KeybindingService::new(vec![
            rule("ctrl+k ctrl+s", "workbench.action.openGlobalKeybindings"),
            rule("ctrl+k ctrl+k escape", "three.chords"),
            rule("ctrl+p", "workbench.action.quickOpen"),
        ]);
        let mut r = CommandRegistry::default();
        for id in [
            "workbench.action.openGlobalKeybindings",
            "three.chords",
            "workbench.action.quickOpen",
        ] {
            r.register(
                id,
                Command {
                    title_key: "t".into(),
                    category_key: None,
                },
            )
            .unwrap();
        }
        (s, r)
    }
    fn press(state: &mut ChordState, k: &str, at: Instant) -> KeyOutcome {
        let (s, r) = setup();
        state
            .press(parse_chord(k).unwrap(), at, &s, &ContextSnapshot::new(), &r)
            .unwrap()
    }
    fn id(o: &KeyOutcome) -> Option<&str> {
        match o {
            KeyOutcome::Command(i) => Some(&i.id),
            _ => None,
        }
    }

    #[test]
    fn chord_completes() {
        let t0 = Instant::now();
        let mut st = ChordState::default();
        assert!(matches!(press(&mut st, "ctrl+k", t0), KeyOutcome::Pending(ref k) if k.len() == 1));
        assert!(st.is_pending());
        // Modifiers alone do not disturb chord mode.
        assert!(matches!(press(&mut st, "ctrl", t0), KeyOutcome::Unhandled));
        assert!(st.is_pending());
        let o = press(&mut st, "ctrl+s", t0 + Duration::from_secs(1));
        assert_eq!(id(&o), Some("workbench.action.openGlobalKeybindings"));
        assert!(!st.is_pending());
        // Three chords, with escape as a bound third chord.
        assert!(matches!(
            press(&mut st, "ctrl+k", t0),
            KeyOutcome::Pending(_)
        ));
        assert!(matches!(press(&mut st, "ctrl+k", t0), KeyOutcome::Pending(ref k) if k.len() == 2));
        assert_eq!(id(&press(&mut st, "escape", t0)), Some("three.chords"));
    }

    #[test]
    fn escape_and_unbound_keys_abort() {
        let t0 = Instant::now();
        let mut st = ChordState::default();
        press(&mut st, "ctrl+k", t0);
        assert!(matches!(press(&mut st, "escape", t0), KeyOutcome::Aborted(ref k) if k.len() == 2));
        assert!(!st.is_pending());
        // After aborting, keys behave normally again.
        assert_eq!(
            id(&press(&mut st, "ctrl+p", t0)),
            Some("workbench.action.quickOpen")
        );
        press(&mut st, "ctrl+k", t0);
        assert!(matches!(press(&mut st, "x", t0), KeyOutcome::Aborted(_)));
        // Outside chord mode an unbound key is simply unhandled (typing).
        assert!(matches!(press(&mut st, "x", t0), KeyOutcome::Unhandled));
        // ctrl+p in chord mode is `ctrl+k ctrl+p`: unbound, so it aborts instead of quick open.
        press(&mut st, "ctrl+k", t0);
        assert!(matches!(
            press(&mut st, "ctrl+p", t0),
            KeyOutcome::Aborted(_)
        ));
    }

    #[test]
    fn timeout_and_focus_loss() {
        let t0 = Instant::now();
        let mut st = ChordState::default();
        press(&mut st, "ctrl+k", t0);
        assert!(!st.tick(t0 + Duration::from_millis(4999)));
        assert!(st.is_pending());
        assert!(st.tick(t0 + CHORD_TIMEOUT));
        assert!(!st.is_pending());
        // A key arriving after the timeout starts fresh instead of completing the chord.
        press(&mut st, "ctrl+k", t0);
        assert!(matches!(
            press(&mut st, "ctrl+s", t0 + Duration::from_secs(6)),
            KeyOutcome::Unhandled
        ));
        press(&mut st, "ctrl+k", t0);
        assert!(st.cancel());
        assert!(!st.cancel());
        assert_eq!(
            id(&press(&mut st, "ctrl+p", t0)),
            Some("workbench.action.quickOpen")
        );
    }
}
