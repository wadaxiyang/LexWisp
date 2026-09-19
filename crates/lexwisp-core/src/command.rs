#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    QuickShell,
    ControlCenter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleQuickShell,
    ShowQuickShell,
    ShowControlCenter,
    Quit,
}
