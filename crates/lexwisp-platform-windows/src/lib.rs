mod atomic_file;
mod credential;
mod shell;
mod single_instance;
mod window;

pub use atomic_file::WindowsAtomicFileWriter;
pub use credential::WindowsCredentialStore;
pub use shell::{PlatformError, WindowsShell, WindowsShellHandle};
pub use single_instance::{SingleInstance, SingleInstanceGuard};
pub use window::{hide_native_window, show_native_window, show_startup_error};
