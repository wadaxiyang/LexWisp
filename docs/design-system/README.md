# LexWisp Design System

This directory contains the design contract and integration record imported from `LexWisp_Design_System.zip` for the current LexWisp GPUI-Kit UI.

## Files

```text
DESIGN.md
    Complete visual and interaction design contract.

../../crates/lexwisp-ui/src/theme.rs
    Installed GPUI-Kit 0.6.1-oriented LexWisp light/dark theme implementation.

../../crates/lexwisp-ui/src/ui_metrics.rs
    Installed spacing, typography, radius, Shell, and content metrics.

INTEGRATION.md
    Exact integration guidance for the current LexWisp Main Shell code.

AGENT_UI_RULES.md
    Condensed rules intended for Coding Agents / AGENTS.md.
```

## Intended use

The pack assumes the current product model:

```text
Main Shell
├── Compact
├── Expanded
└── Workspace

Current Experience
└── Chat
```

The Shell visual system is intentionally Experience-neutral so another Experience can reuse it later.

## Theme character

Dark appearance:

- graphite canvas
- layered low-chroma surfaces
- restrained cool accent
- compact workbench density
- assistant content flowing directly on the page

Light appearance:

- cool-neutral canvas
- clean white working surfaces
- subtle gray navigation
- hairline structure
- quiet blue-gray selections

The two appearances share geometry and hierarchy.

## Project status

The theme and metric modules are installed in `lexwisp-ui`, the Main Shell uses the shared sizes and theme projection, and the current Chat presentation applies the composer, message, status, header, sidebar, and workspace-column corrections from `INTEGRATION.md`.

`DESIGN.md` is the normative product UI contract. `INTEGRATION.md` and `AGENT_UI_RULES.md` are retained as source guidance and migration history; active agent enforcement also lives in the repository-root `AGENTS.md`.
