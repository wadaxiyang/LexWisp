#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    QuickShell,
    ChatPanel,
    ControlCenter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleQuickShell {
        launch_generation: u64,
    },
    ApplyLaunchContext {
        launch_generation: u64,
        snapshot: crate::ContextSnapshot,
    },
    ShowQuickShell,
    ShowChatPanel,
    ShowControlCenter,
    RefreshPlugins,
    Quit,
}
