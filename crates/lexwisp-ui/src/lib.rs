//! Native LexWisp surfaces. Platform and host behavior arrive through narrow ports.

mod control_center;
mod surface;

pub mod theme;
pub mod ui_metrics;

pub use surface::{
    ShellContentViewFactory, ShellSession, SurfaceController, SurfaceFactory, SurfaceServices,
    SurfaceWindowPlatform, WindowRegistry,
};
