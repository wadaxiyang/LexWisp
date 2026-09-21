mod atomic_file;
mod context;
mod credential;
mod shell;
mod single_instance;
mod window;

pub use atomic_file::WindowsAtomicFileWriter;
pub use context::{WindowsContextHandle, WindowsContextService};
pub use credential::WindowsCredentialStore;
pub use shell::{PlatformError, WindowsShell, WindowsShellHandle};
pub use single_instance::{SingleInstance, SingleInstanceGuard};
pub use window::{
    display_id_under_cursor, hide_native_window, set_native_window_bounds, show_native_window,
    show_startup_error,
};
