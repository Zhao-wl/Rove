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
        // #16 工作台命令：标题与分类登记在同一张命令表里，执行由工作台分发。
        crate::shell::catalog::register_commands(&mut r)?;
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
