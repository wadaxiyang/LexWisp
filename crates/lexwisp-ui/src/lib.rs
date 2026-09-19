//! Native LexWisp surfaces. Platform and host behavior arrive through narrow ports.

mod control_center;
mod quick_shell;
mod surface;

pub use surface::{SurfaceController, SurfaceFactory, SurfaceWindowPlatform, WindowRegistry};
