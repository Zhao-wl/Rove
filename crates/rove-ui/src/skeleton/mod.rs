//! Replaceable #2 implementations. Frozen contracts remain in their owning crates.
pub mod appearance;
pub(crate) mod events;
mod text;
mod workbench;
pub use workbench::{launch, MarkdownSnapshot, Workbench};
