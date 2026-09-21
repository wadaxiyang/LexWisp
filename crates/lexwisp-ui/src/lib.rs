//! Native LexWisp surfaces. Platform and host behavior arrive through narrow ports.

mod chat;
mod settings_view;
mod surface;

pub mod theme;
pub mod ui_metrics;

pub use chat::{ChatExperience, register_shortcuts};
pub use settings_view::SettingsView;
pub use surface::{SurfaceController, SurfaceServices, SurfaceWindowPlatform};
