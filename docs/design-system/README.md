# LexWisp design system

`DESIGN.md` is the normative product UI contract. `crates/lexwisp-ui/src/theme.rs` owns semantic light/dark colors; `ui_metrics.rs` owns shared geometry. The independent `examples/lexwisp-ui-lab/src/lexwisp_main_window.rs` supplies the one-popup composition reference. Main product code adapts the reference without changing or depending on the lab.

Read the installed `gpui-kit` and `gpui-kit-design-guides` skills, the locked Kit source, these files, and the relevant lab sections before changing UI. `INTEGRATION.md` maps current product components to the contract. `AGENT_UI_RULES.md` is a short implementation checklist.
