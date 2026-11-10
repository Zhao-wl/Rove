//! SKELETON(commands): two registered placeholder commands; replaced by #14.
use rove_base::*;
use rove_commands::*;

pub struct Commands {
    pub registry: CommandRegistry,
}
impl Commands {
    pub fn new() -> Result<Self> {
        let mut r = CommandRegistry::default();
        for (id, title) in [
            ("workbench.action.quickOpen", "commands.quickOpen"),
            ("workbench.action.showCommands", "commands.palette"),
        ] {
            r.register(
                id,
                Command {
                    title_key: title.into(),
                    category_key: None,
                },
            )?;
        }
        crate::editors::actions::register(&mut r)?;
        Ok(Self { registry: r })
    }
}
impl CommandExecutor for Commands {
    fn execute(&self, i: Invocation, c: Cancellation) -> Task<Value> {
        let key = self.registry.get(&i.id).map(|v| v.title_key.clone());
        Box::pin(async move {
            c.check()?;
            match key {
                Some(k) => Ok(Value::String(k)),
                None => Err(Error::NotFound),
            }
        })
    }
}
/// SKELETON(workbench): status-only feedback, replaced by #16.
#[derive(Default)]
pub struct StatusFeedback {
    key: Option<std::sync::Arc<str>>,
    pub generation: u64,
    pub message: Option<rove_i18n::Message>,
}
impl StatusFeedback {
    pub fn clear(&mut self) {
        self.generation += 1;
        self.message = None;
        self.key = None;
    }
}
impl crate::feedback::FeedbackService for StatusFeedback {
    fn show(&mut self, f: crate::feedback::Feedback) -> Result<()> {
        if !matches!(f.level, crate::feedback::FeedbackLevel::Status { .. }) {
            return Err(Error::Unsupported);
        }
        self.generation += 1;
        self.key = Some(f.key);
        self.message = Some(f.message);
        Ok(())
    }
    fn dismiss(&mut self, key: &str) -> Result<()> {
        if self.key.as_deref() != Some(key) {
            return Err(Error::NotFound);
        }
        self.clear();
        Ok(())
    }
    fn events(&self) -> &dyn Events<crate::feedback::FeedbackEvent> {
        &NoEvents
    }
}
