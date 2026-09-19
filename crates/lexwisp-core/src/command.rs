#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    QuickShell,
    ChatPanel,
    ControlCenter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleQuickShell(crate::ContextSnapshot),
    ShowQuickShell,
    ShowChatPanel,
    ShowControlCenter,
    Quit,
}
