//! Command and keybinding contracts (#14). No GPUI actions or input types cross this boundary.
use rove_base::*;
use std::{collections::BTreeMap, sync::Arc};
/// Command title is an i18n key. Args use JSON-compatible values, not a UI-specific object.
#[derive(Clone, Debug)]
pub struct Command {
    pub title_key: Arc<str>,
    pub category_key: Option<Arc<str>>,
}
pub type CommandRegistry = Registry<Command>;
#[derive(Clone, Debug)]
pub struct Invocation {
    pub id: Arc<str>,
    pub args: Value,
}
/// Dispatch is O(log commands) and must return promptly; background work stays in returned task.
/// Unknown command => NotFound, invalid args => InvalidInput. Send + Sync bridge queues UI actions
/// onto the owning UI thread; cancellation before dispatch has no effects. Completion means applied.
pub trait CommandExecutor: Send + Sync {
    fn execute(&self, invocation: Invocation, cancel: Cancellation) -> Task<Value>;
}
pub struct EmptyCommands;
impl CommandExecutor for EmptyCommands {
    fn execute(&self, _: Invocation, c: Cancellation) -> Task<Value> {
        Box::pin(async move {
            c.check()?;
            Err(Error::NotFound)
        })
    }
}
#[derive(Clone, Debug)]
pub struct ContextChange {
    pub keys: Arc<[Arc<str>]>,
}
/// Cached context snapshots, UI-safe reads O(log keys), Send + Sync. Set publishes after unlocking;
/// None removes a key. Missing keys are unknown and evaluate false according to FEATURES §13.7.
pub trait ContextKeys: Send + Sync {
    fn get(&self, key: &str) -> Option<Value>;
    fn set(&self, key: Arc<str>, value: Option<Value>) -> Result<()>;
    fn changes(&self) -> &dyn Events<ContextChange>;
}
pub struct EmptyContext;
impl ContextKeys for EmptyContext {
    fn get(&self, _: &str) -> Option<Value> {
        None
    }
    fn set(&self, _: Arc<str>, _: Option<Value>) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn changes(&self) -> &dyn Events<ContextChange> {
        &NoEvents
    }
}
/// VS Code when-expression source kept losslessly. #14 owns parsing/precedence/operator semantics.
#[derive(Clone, Debug)]
pub struct When(pub Arc<str>);
/// Evaluation of prepared expression on cached values only; no I/O. Invalid expression =>
/// InvalidInput, unsupported syntax => Unsupported. Unknown keys follow the shared false policy.
pub trait WhenEvaluator: Send + Sync {
    fn evaluate(&self, expression: &When, context: &dyn ContextKeys) -> Result<bool>;
}
/// Empty expression means unconditional; nonempty expressions are unsupported, never guessed.
pub struct EmptyWhen;
impl WhenEvaluator for EmptyWhen {
    fn evaluate(&self, e: &When, _: &dyn ContextKeys) -> Result<bool> {
        if e.0.is_empty() {
            Ok(true)
        } else {
            Err(Error::Unsupported)
        }
    }
}
/// Normalized physical/logical key chord representation; resolver implements keyboard.dispatch.
#[derive(Clone, Debug)]
pub struct KeyStroke {
    pub key: Arc<str>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}
#[derive(Clone, Debug)]
pub struct Binding {
    pub chord: Arc<[KeyStroke]>,
    pub invocation: Invocation,
    pub when: Option<When>,
}
#[derive(Clone, Debug)]
pub enum KeyResolution {
    NoMatch,
    PendingChord,
    Command(Invocation),
}
/// UI-safe cached resolution, O(candidate bindings); unknown command IDs are ignored. Prefix is
/// the current chord sequence, not hidden global state. Unsupported expressions cannot activate a
/// binding. Parsing/reload occurs off-thread, and failures preserve the last valid binding table.
pub trait Keybindings: Send + Sync {
    fn resolve(
        &self,
        prefix: &[KeyStroke],
        context: &dyn ContextKeys,
        commands: &CommandRegistry,
    ) -> Result<KeyResolution>;
}
pub struct EmptyKeybindings;
impl Keybindings for EmptyKeybindings {
    fn resolve(
        &self,
        _: &[KeyStroke],
        _: &dyn ContextKeys,
        _: &CommandRegistry,
    ) -> Result<KeyResolution> {
        Ok(KeyResolution::NoMatch)
    }
}
/// Detached context data for background evaluation; capture before leaving the UI event handler.
pub type ContextSnapshot = BTreeMap<Arc<str>, Value>;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_registers_vscode_command() {
        let mut r = CommandRegistry::default();
        r.register(
            "workbench.action.quickOpen",
            Command {
                title_key: "commands.quickOpen".into(),
                category_key: None,
            },
        )
        .unwrap();
        assert_eq!(r.entries().count(), 1);
        assert!(matches!(
            EmptyKeybindings.resolve(&[], &EmptyContext, &r).unwrap(),
            KeyResolution::NoMatch
        ));
    }
    #[test]
    fn context_and_when_consumer() {
        let context: &dyn ContextKeys = &EmptyContext;
        assert!(context.get("editorFocus").is_none());
        assert!(context
            .set("editorFocus".into(), Some(Value::Bool(true)))
            .is_err());
        let _sub = context.changes().subscribe(Arc::new(|_| {}));
        assert!(EmptyWhen.evaluate(&When("".into()), context).unwrap());
        assert!(EmptyWhen
            .evaluate(&When("editorFocus".into()), context)
            .is_err());
    }
}

// #14 implementation modules (contracts above stay unchanged).
pub mod builtin;
pub mod center;
pub mod chord;
pub mod context;
pub mod keybindings;
pub mod keys;
pub mod when;
