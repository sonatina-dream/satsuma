//! Platform-independent logic for Satsuma, a drag-gesture file converter.
//!
//! See `SPEC.md` at the repository root for the behavior this crate implements.

pub mod actions;
pub mod edit;
pub mod format;
pub mod geometry;
pub mod gesture;
pub mod matrix;
pub mod naming;
pub mod settings;
pub mod tools;
pub mod wheel;

pub use actions::Action;
pub use format::{Category, Format};
pub use gesture::{DragTracker, Effect, Mods, Sample, WheelCombos, WheelMode};
pub use settings::Settings;
pub use tools::Tool;
pub use wheel::WheelItem;
