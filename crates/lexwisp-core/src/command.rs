#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    MainShell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleMainShell,
    ShowMainShell,
    ShowSettings,
    ShowAbout,
    Quit,
}
