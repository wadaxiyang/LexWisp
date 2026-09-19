# LexWisp declarative plugin schema (Stage 6)

LexWisp Stage 6 accepts a local directory or ZIP containing a declarative plugin. Import always shows a source, action, version, hash, and permission preview before the managed copy is installed. Dropping a package on the Plugins page follows the same preview; it never installs or executes automatically.

## Package layout

```text
plugin-root/
├── manifest.toml
├── prompt.md
└── icon.svg        optional
```

`schema_version` is `1`, `plugin.kind` is `declarative`, `plugin.version` is SemVer, and `plugin.host_api` must include Host API `1.0`. Stage 6 declarative packages require only `ai.invoke`; script packages and `main.js` are introduced separately in Stage 7.

The executable example is in `examples/plugins/academic-polish`. Its manifest is compiled into a parser fixture, so documentation drift fails the test suite.

## Manifest fields

- `[plugin]`: `id` is a lowercase reverse-DNS identifier; `name`, `version`, `kind`, and `host_api` are required.
- `[capabilities]`: `required` and `optional` arrays are required. Unknown capabilities are rejected. The manifest requests permissions; the confirmation creates the actual hash- and generation-bound grant.
- `[[actions]]`: `id`, `name`, `input_kind = "text"`, `allowed_sources`, package-relative `prompt`, `model_profile = "Fast"`, and `dismiss_policy` are required.
- `[[actions.parameters]]`: `key`, `label`, `kind`, `required`, optional `default`, and `choices` are supported. Kinds are `text`, `enum`, `boolean`, and `number`.
- `[actions.output]`: `format = "text"` and the three `allow_copy`, `allow_favorite`, and `allow_replace` booleans are required.

Unknown manifest fields, action sources, parameter kinds, template variables, and parameters are rejected. Prompt templates may only substitute declared `{{params.key}}` values. Submitted text is sent as a separate user message and is never interpreted as template syntax.

## Package policy

ZIPs are limited to 10 MiB compressed, 32 MiB extracted, and 256 entries. LexWisp rejects absolute, drive, UNC, parent, ADS, case-colliding, reserved Windows, link, and reparse-point paths. Manifest files are limited to 256 KiB, prompt files to 2 MiB, and icons to 1 MiB. Installation scripts and dependency downloads are not supported.

Same-ID packages require an explicit replacement confirmation. Reload validates a new package and asks for confirmation before switching generations. Expanding permissions is highlighted. Disabling or uninstalling revokes grants and cancels active work; uninstall removes only the managed copy, not the original import source or saved history.
