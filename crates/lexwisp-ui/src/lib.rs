//! Native LexWisp surfaces. Platform and host behavior arrive through narrow ports.

mod control_center;
mod surface;

pub use surface::{
    ChatPanelViewFactory, QuickShellViewFactory, SurfaceController, SurfaceFactory,
    SurfaceServices, SurfaceWindowPlatform, WindowRegistry,
};
