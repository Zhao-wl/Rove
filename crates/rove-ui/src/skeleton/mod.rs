//! Replaceable #2 implementations. Frozen contracts remain in their owning crates.
pub mod appearance;
mod commands;
mod editor;
mod events;
mod text;
mod workbench;
pub use workbench::{launch, Workbench};
