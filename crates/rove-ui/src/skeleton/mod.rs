//! Replaceable #2 implementations. Frozen contracts remain in their owning crates.
pub mod appearance;
mod commands;
pub(crate) mod events;
mod text;
mod workbench;
mod workspace;
pub use workbench::{launch, Workbench};
