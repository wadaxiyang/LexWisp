#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceKind {
    MainShell,
    ControlCenter,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShellPresentation {
    #[default]
    Compact,
    Expanded,
    Workspace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostUiCommand {
    ToggleMainShell {
        launch_generation: u64,
    },
    ApplyLaunchContext {
        launch_generation: u64,
        snapshot: crate::ContextSnapshot,
    },
    ShowMainShell,
    SetMainShellPresentation(ShellPresentation),
    ShowControlCenter,
    RefreshPlugins,
    Quit,
}
