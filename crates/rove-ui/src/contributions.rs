//! Built-in contribution registry (#16), not a plugin system.
use rove_base::*;
use rove_i18n::Message;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub enum Placement {
    ActivityBar,
    Sidebar { container: Arc<str> },
    Panel,
    StatusLeft,
    StatusRight,
    EditorTitle,
}
/// Assembly connects factory_key to a UI-owned factory. Visibility uses a when-expression.
/// Fixed positions are not draggable in M1. Status content is supplied by its factory.
#[derive(Clone, Debug)]
pub struct Contribution {
    pub title: Message,
    pub placement: Placement,
    pub order: i32,
    pub icon: Option<Arc<str>>,
    pub command: Option<Arc<str>>,
    pub when: Option<rove_commands::When>,
    pub factory_key: Arc<str>,
}
/// UI-thread assembly, O(log entries) registration/lookup; duplicates fail. Freeze before render;
/// factories are lazy (#2/#16), so registration never loads documents, WebView2 or filesystem data.
pub type ContributionRegistry = Registry<Contribution>;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workbench_registers_view_and_status() {
        let mut r = ContributionRegistry::default();
        for (id, placement) in [
            ("workbench.view.explorer", Placement::ActivityBar),
            ("rove.status.index", Placement::StatusLeft),
        ] {
            r.register(
                id,
                Contribution {
                    title: Message::key("common.loading"),
                    placement,
                    order: 0,
                    icon: None,
                    command: None,
                    when: None,
                    factory_key: id.into(),
                },
            )
            .unwrap();
        }
        assert_eq!(r.entries().count(), 2);
    }
}
