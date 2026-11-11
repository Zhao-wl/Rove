//! Command registration and dispatch (#14).
//!
//! Registration and implementation are separate steps so that a command can be "registered but
//! not implemented" (its VS Code ID is known, keybindings resolve to it, but it must not appear in
//! the command palette — UX_SPEC §1.7). Each feature registers its own commands from its own
//! module; there is no central list to edit.
use crate::{Command, CommandExecutor, CommandRegistry, Invocation};
use rove_base::{Cancellation, Error, Result, Task, Value};
use std::{collections::BTreeMap, sync::Arc};

/// A command implementation. Must return promptly; long work lives inside the returned task.
/// Handlers that touch UI state enqueue onto the owning window and complete when applied.
pub type Handler = Arc<dyn Fn(Invocation, Cancellation) -> Task<Value> + Send + Sync>;

/// Command table: metadata (`CommandRegistry`) plus optional handlers.
#[derive(Default)]
pub struct CommandCenter {
    registry: CommandRegistry,
    handlers: BTreeMap<String, Handler>,
}
impl CommandCenter {
    /// Register a VS Code (or `rove.`) command ID with its title/category string keys.
    /// Duplicate IDs fail with `Duplicate` and do not overwrite.
    pub fn register(&mut self, id: &str, command: Command) -> Result<()> {
        self.registry.register(id, command)
    }
    /// Attach the implementation of a registered command. Unknown ID => NotFound,
    /// already implemented => Duplicate.
    pub fn implement(&mut self, id: &str, handler: Handler) -> Result<()> {
        if self.registry.get(id).is_none() {
            return Err(Error::NotFound);
        }
        if self.handlers.contains_key(id) {
            return Err(Error::Duplicate(id.to_owned()));
        }
        self.handlers.insert(id.to_owned(), handler);
        Ok(())
    }
    /// Register and implement in one call.
    pub fn register_implemented(
        &mut self,
        id: &str,
        command: Command,
        handler: Handler,
    ) -> Result<()> {
        self.register(id, command)?;
        self.implement(id, handler)
    }
    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }
    pub fn is_registered(&self, id: &str) -> bool {
        self.registry.get(id).is_some()
    }
    pub fn is_implemented(&self, id: &str) -> bool {
        self.handlers.contains_key(id)
    }
    /// Implemented commands in ID order: what the command palette may list.
    pub fn implemented(&self) -> impl Iterator<Item = (&str, &Command)> {
        self.registry
            .entries()
            .filter(|(id, _)| self.handlers.contains_key(*id))
    }
}
/// Unknown command => NotFound; registered but not implemented => Unsupported (never pretends to
/// run). Cancellation before dispatch has no effect on the handler.
impl CommandExecutor for CommandCenter {
    fn execute(&self, invocation: Invocation, cancel: Cancellation) -> Task<Value> {
        let outcome = if !self.is_registered(&invocation.id) {
            Err(Error::NotFound)
        } else {
            self.handlers
                .get(invocation.id.as_ref())
                .cloned()
                .ok_or(Error::Unsupported)
        };
        match outcome {
            Ok(handler) => {
                if let Err(e) = cancel.check() {
                    return Box::pin(async move { Err(e) });
                }
                handler(invocation, cancel)
            }
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }
}

/// Build a handler from a synchronous closure (for commands whose effect is immediate).
pub fn sync_handler(f: impl Fn(&Invocation) -> Result<Value> + Send + Sync + 'static) -> Handler {
    Arc::new(move |invocation, cancel| {
        let result = cancel.check().and_then(|_| f(&invocation));
        Box::pin(async move { result })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block<T>(task: Task<T>) -> Result<T> {
        // The test handlers complete immediately; poll once with a no-op waker.
        use std::task::{Context, Poll, Waker};
        let mut task = task;
        match task.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(r) => r,
            Poll::Pending => panic!("test task should be ready"),
        }
    }
    fn cmd(title: &str) -> Command {
        Command {
            title_key: title.into(),
            category_key: None,
        }
    }
    fn inv(id: &str, args: Value) -> Invocation {
        Invocation {
            id: id.into(),
            args,
        }
    }
    #[test]
    fn registered_vs_implemented_and_dispatch_with_args() {
        let mut c = CommandCenter::default();
        c.register("workbench.action.quickOpen", cmd("commands.quickOpen"))
            .unwrap();
        c.register_implemented(
            "editor.action.revealDefinition",
            cmd("commands.revealDefinition"),
            sync_handler(|i| Ok(i.args.clone())),
        )
        .unwrap();
        assert!(c.is_registered("workbench.action.quickOpen"));
        assert!(!c.is_implemented("workbench.action.quickOpen"));
        assert_eq!(
            c.implemented().map(|(id, _)| id).collect::<Vec<_>>(),
            ["editor.action.revealDefinition"]
        );
        let args = Value::String("payload".into());
        assert_eq!(
            block(c.execute(
                inv("editor.action.revealDefinition", args.clone()),
                Cancellation::default()
            )),
            Ok(args)
        );
        assert_eq!(
            block(c.execute(
                inv("workbench.action.quickOpen", Value::Null),
                Cancellation::default()
            )),
            Err(Error::Unsupported)
        );
        assert_eq!(
            block(c.execute(inv("no.such.command", Value::Null), Cancellation::default())),
            Err(Error::NotFound)
        );
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert_eq!(
            block(c.execute(
                inv("editor.action.revealDefinition", Value::Null),
                cancelled
            )),
            Err(Error::Cancelled)
        );
    }
    #[test]
    fn registration_errors() {
        let mut c = CommandCenter::default();
        assert_eq!(
            c.implement("x", sync_handler(|_| Ok(Value::Null)))
                .unwrap_err(),
            Error::NotFound
        );
        c.register("x", cmd("t")).unwrap();
        assert!(matches!(
            c.register("x", cmd("t")),
            Err(Error::Duplicate(_))
        ));
        c.implement("x", sync_handler(|_| Ok(Value::Null))).unwrap();
        assert!(matches!(
            c.implement("x", sync_handler(|_| Ok(Value::Null))),
            Err(Error::Duplicate(_))
        ));
        assert_eq!(c.register("", cmd("t")), Err(Error::InvalidInput));
    }
}
