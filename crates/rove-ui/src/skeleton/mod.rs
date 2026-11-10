//! Replaceable #2 implementations. Frozen contracts remain in their owning crates.
pub mod appearance;
mod commands;
mod editor;
mod events;
mod syntax;
mod text;
mod workbench;
mod workspace;
pub use workbench::{launch, Workbench};
