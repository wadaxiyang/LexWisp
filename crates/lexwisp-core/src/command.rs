#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    QuickShell,
    ControlCenter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleQuickShell(crate::ContextSnapshot),
    ShowQuickShell,
    ShowControlCenter,
    Quit,
}
