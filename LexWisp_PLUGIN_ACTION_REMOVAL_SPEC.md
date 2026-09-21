# LexWisp Plugin / Legacy Action Removal SPEC

> Status: Ready for implementation  
> Scope: Architectural simplification before future built-in Agent/Tool work  
> Product direction: Native Windows floating AI chat, comparable in product boundary to PopChat, with possible future **built-in** Agent tools such as web search.  
> Non-goal: General-purpose plugin platform, arbitrary user extensions, selection-first text actions, or a speculative agent framework.

---

## 1. Goal

LexWisp is no longer a selection-first extensible text utility.

The product is now:

> **A native Windows floating AI chat client with global hotkey, persistent conversations, configurable OpenAI-compatible providers, and room for a small set of first-party built-in Agent tools in the future.**

This change removes two obsolete architectural layers:

1. **Runtime Plugin System**
2. **Legacy selection/action system**

The implementation must retain the valuable infrastructure underneath them:

- Chat controller and conversation state
- Model/provider infrastructure
- Streaming execution lifecycle
- Cancellation
- Persistence/history
- Background task ownership
- Windows hotkey/tray/window lifecycle
- GPUI/GPUI-Kit UI architecture

The implementation must **not** replace the old plugin/action framework with a new speculative abstraction layer in this change.

---

# 2. Architectural Decision

## 2.1 Remove Plugin as a first-class concept

The following concepts must cease to exist in product architecture:

```text
Plugin
PluginId
PluginDescriptor
PluginRegistry
PluginManager
PluginManagementUiPort
ManagedPlugin*
PluginImport*
PluginKind
Plugin lifecycle
Plugin generation
Plugin capability grants
Plugin KV
Declarative Plugin
Script Plugin
Plugin ZIP/package installation
Plugin reload/enable/disable/uninstall
```

LexWisp must no longer load executable or declarative behavior at runtime from user-installed packages.

There is no plugin marketplace, plugin folder, plugin manifest, plugin sandbox, plugin VM, or plugin permission model.

---

## 2.2 Remove legacy Action as a product model

The old Action model came from the previous LexWisp product direction:

```text
capture selection
    ↓
choose action
    ↓
translate / polish / chat / ...
    ↓
render action result
    ↓
optionally replace source text
```

That model is obsolete.

The following concepts must therefore also cease to be first-class architecture:

```text
ActionRegistry
ActionDescriptor
ActionHandler
ActionKind
ActionId
QualifiedActionId
ActionInputSource
ActionOutputPolicy
ActionParameter
DeclarativeActionDefinition
ScriptActionDefinition
TextActionUiPort
TextRunPort
ActionUiPort
configured default action
action palette
replace-source action semantics
selection-driven action dispatch
```

Do not rename these types and keep their semantics. Remove the model.

---

# 3. Target Product Model

After this refactor the primary runtime flow is:

```text
Global Hotkey / Tray
        ↓
SurfaceController
        ↓
Floating Chat UI
        ↓
ChatController
        ↓
ChatRunPort / RunSupervisor
        ↓
Provider / Model request
        ↓
Streaming result
        ↓
ExecutionStore
        ↓
ChatController / UI / Persistence
```

Future built-in Agent tools may later extend the run loop:

```text
ChatController
      ↓
RunSupervisor
      ↓
Model
      ↓
optional built-in tool call
      ↓
ToolRunner
      ↓
Model
      ↓
final answer
```

**This future tool layer is not part of this deletion task.**

Do not introduce `ToolRegistry`, MCP, skill frameworks, generic function buses, reflection-based dispatch, arbitrary JSON RPC, or agent planners unless strictly required to keep existing Chat working.

---

# 4. Core Design Principles

## 4.1 Chat is a first-party product capability

Chat must not register itself as a plugin.

Remove flows such as:

```rust
let plugin = chat_plugin();
let action = chat_action();

handles
    .plugins()
    .register_package(...);

handles.capabilities().replace_grants(...);
```

Chat should be constructed directly from first-party services.

Preferred conceptual composition:

```rust
let chat_controller = ChatController::new(
    handles.chat_run_port(),
    handles.chat_history(),
)?;
```

No `PluginId` must be required to construct Chat.

---

## 4.2 First-party components do not request plugin capabilities

Remove the current plugin capability authorization path.

A first-party ChatController does not need to ask LexWisp for permission to use LexWisp.

Future dangerous Agent operations may receive a dedicated **user approval policy**, but this must not be implemented by preserving `CapabilityAuthority`.

---

## 4.3 Keep typed ports

Do **not** collapse the application into a global state object.

Retain typed service boundaries such as:

```text
ChatRunPort
ChatHistoryPort
ProviderUiPort
SettingsUiPort
HistoryUiPort
Context-specific ports that remain truly needed
```

Keep `HostHandles` as a typed composition surface if it remains useful.

Do not turn it into:

```rust
Arc<Mutex<AppState>>
```

Do not add:

```text
ServiceLocator
global EventBus
invoke("name", json)
dynamic command bus
```

---

## 4.4 Keep execution supervision

The current execution infrastructure contains useful product behavior independent of plugins:

- cancellation
- terminal state ownership
- streaming accumulation
- persistence checkpoints
- unsaved/error state
- task ownership
- bounded shutdown

These behaviors must survive.

Remove plugin/action identity fields from execution state where they no longer have product meaning.

---

# 5. Target Workspace Shape

The exact crate split may remain close to the current workspace.

A reasonable target is:

```text
crates/
├── lexwisp-app
├── lexwisp-core
├── lexwisp-host
├── lexwisp-platform-windows
├── lexwisp-storage
└── lexwisp-ui
```

The current crates:

```text
lexwisp-plugins-builtin
lexwisp-plugins-script
```

must no longer exist under those names or semantics.

### Preferred treatment

- Move `ChatController` and Chat-specific first-party code out of `lexwisp-plugins-builtin`.
- Delete `lexwisp-plugins-script`.
- Either:
  - place Chat controller/business code in `lexwisp-host` or another clearly first-party crate, and Chat views in `lexwisp-ui`, or
  - create a narrowly named first-party `lexwisp-chat` crate **only if the existing dependency graph clearly benefits from it**.

Do not create a new crate merely to preserve the old plugin crate under another name.

---

# 6. Required Deletions

## 6.1 Workspace dependencies

Remove plugin-only workspace members and dependencies.

Expected removals include, where no longer used elsewhere:

```toml
lexwisp-plugins-builtin
lexwisp-plugins-script
rquickjs
semver
sha2
hex
zip
```

Do not remove a dependency if it still has a real non-plugin use.

Regenerate `Cargo.lock` only through Cargo.

---

## 6.2 Delete Script Plugin runtime

Delete the entire Script Plugin execution path, including:

```text
crates/lexwisp-plugins-script/
```

and all associated:

- QuickJS runtime creation
- JS module loading
- JS lifecycle
- script manifests
- script package definitions
- script host bridge
- script HTTP bridge
- script network scopes
- JS interrupt/budget machinery
- plugin-local JS context
- plugin package-local imports

Delete tests that exist only to validate QuickJS/plugin behavior.

Example:

```text
crates/lexwisp-app/tests/quickjs_probe.rs
```

must be deleted if no longer relevant.

---

## 6.3 Delete Declarative Plugin runtime

Delete:

- declarative package loading
- plugin manifest parsing for declarative actions
- template expansion designed around installed plugin packages
- declarative package activation
- package version checks
- package hashing

If prompt shortcuts or slash commands are desired later, they will be implemented as a separate first-party feature.

Do not preserve Declarative Plugin as hidden infrastructure.

---

## 6.4 Delete PluginManager

Remove:

```text
crates/lexwisp-host/src/plugin_manager.rs
```

and all responsibilities owned only by that manager:

- managed plugin directory
- `.staging`
- ZIP import
- path validation for plugin packages
- plugin hash checks
- plugin preview
- activation
- reload
- removal
- enable/disable
- install state
- plugin status
- plugin errors
- plugin lifecycle notifications

The application must not create a `plugins/` directory.

---

## 6.5 Delete plugin management UI

Remove plugin-specific fields from:

```text
SurfaceController
SurfaceServices
ControlCenter
```

Remove:

```rust
plugin_management: Arc<dyn PluginManagementUiPort>
```

Remove plugin settings/control-center pages and controls:

- import plugin
- install preview
- plugin list
- plugin enable/disable
- reload
- uninstall
- capability display
- package source
- package version/hash
- plugin error status

The Control Center should focus on actual product configuration:

```text
General
Appearance
Providers / Models
Hotkey
History / Data
About
```

Exact information architecture may remain aligned with the current design system.

---

# 7. Remove Legacy Selection/Action Architecture

This is a required part of the task, not optional cleanup.

## 7.1 Remove Action Registry

Delete or fully eliminate the semantics of:

```text
ActionRegistry
ActionUiService
ActionUiPort
ActionHandler
ActionDescriptor
ActionKind
ActionId
QualifiedActionId
```

Do not retain a one-action registry containing only Chat.

Chat is not an Action.

---

## 7.2 Remove action dispatch from startup

The app composition root must no longer perform:

```text
chat_plugin()
chat_action()
register_package()
register action handler
collect action descriptors
activate installed plugins
grant capabilities
```

There should be no Chat registration step.

Construct Chat directly.

---

## 7.3 Remove Action descriptors from surfaces

Remove fields such as:

```rust
action_descriptors: Vec<ActionDescriptor>
```

from:

```text
SurfaceController
SurfaceServices
ControlCenter
other UI projections
```

Delete UI logic that assumes multiple actions.

---

## 7.4 Remove action palette / configured default action

Delete any product behavior whose purpose is:

```text
choose an action after opening the shell
configure a default action
dispatch captured text to a selected action
```

The global hotkey should open/toggle the Chat surface.

---

# 8. Selection and Context Capture

The previous LexWisp architecture treated foreground selection as a central input source.

That is no longer the product model.

## 8.1 Remove selection-first launch behavior

The hotkey path must not require successful selection capture before showing Chat.

Opening Chat must work independently of:

- selected text
- clipboard text
- foreground target
- replacement target

The normal flow is:

```text
hotkey
  ↓
show chat
  ↓
focus composer
```

---

## 8.2 Remove action-specific selection types

Remove action-only selection concepts such as:

```text
ActionInputSource::Selection
ActionInputSource::Clipboard
ActionInputSource::Manual
```

Do not preserve them under renamed enums.

---

## 8.3 ContextSnapshot review

`ContextSnapshot`, `CaptureStatus`, foreground selection capture, clipboard/UIA capture, and launch-generation logic must be audited.

### Required rule

If a context-capture type exists **only** to feed the old action system, delete it.

If a context-capture capability is still needed for another current product behavior, keep only the smallest surviving part.

### Product default

The Chat panel must not implicitly attach selected text on launch unless current UI/product requirements explicitly depend on it.

Do not silently send selection/clipboard content to the model.

---

## 8.4 Source replacement

Delete the old “replace selected source text after action result” workflow unless there is an independent current product requirement for it.

That includes:

- replace-source action policy
- target revalidation used only by action output replacement
- action-level replace permission
- output policy fields whose sole purpose is replacement

Do not delete general Windows clipboard or UIA utilities if still needed elsewhere, but remove product wiring that exists solely for action replacement.

---

# 9. Host Refactor

## 9.1 Host module list

Review current host modules:

```text
ai
declarative
execution
favorites
history
invocation
plugin_manager
providers
registry
script
settings
tasks
```

Target:

```text
ai
execution
favorites
history
invocation
providers
settings
tasks
```

`registry` may remain only if it contains non-plugin runtime infrastructure that genuinely survives. Prefer splitting surviving code into appropriately named modules instead of keeping a misleading registry module.

Delete:

```text
declarative
plugin_manager
script
```

---

## 9.2 HostHandles

Current plugin-related fields must be removed:

```text
plugins
actions
capabilities
plugin_manager
```

A likely target shape:

```rust
pub struct HostHandles {
    tasks: HostTaskPort,
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    context: Option<Arc<dyn ContextUiPort>>, // only if still truly required
    favorites: Arc<dyn FavoriteUiPort>,
    history: Arc<dyn HistoryUiPort>,
    chat_history: Arc<dyn ChatHistoryPort>,
    executions: Arc<ExecutionStore>,
    supervisor: Arc<RunSupervisor>,
    ui_commands: HostUiCommandPort,
}
```

Exact names may differ.

Do not keep plugin-shaped getters returning dead wrappers.

---

# 10. InvocationSupervisor → RunSupervisor

The current `InvocationSupervisor` contains useful behavior but is polluted by plugin/action identity.

Refactor it into a first-party Chat execution supervisor.

Renaming to **`RunSupervisor`** is preferred if it improves clarity and can be done without unnecessary churn.

## 10.1 Keep

Preserve:

- provider invocation
- streaming
- cancellation
- terminal transition guarantees
- execution state updates
- persistence interaction
- task ownership
- timeout/error handling
- provider/model selection
- partial result preservation

---

## 10.2 Remove

Remove dependencies on:

```text
PluginId
Plugin generation
CapabilityAuthority
ActionRegistry
QualifiedActionId
ActionHandler lookup
```

The Chat run path should call the supervisor directly through a typed port.

---

# 11. Execution Model Refactor

Execution state must become Chat/Run-oriented instead of Plugin/Action-oriented.

Audit structures such as:

```text
ExecutionSnapshot
ExecutionCheckpoint
ExecutionStart
HistoryItem
HistoryDetail
```

Remove fields whose only purpose is to identify plugin/action execution.

Examples likely to remove or replace:

```rust
plugin_id
plugin_generation
QualifiedActionId
action
```

Use first-party semantic identifiers instead.

Possible target:

```rust
RunId
ConversationId
MessageId
AttemptId
```

If `InvocationId` is already stable and useful, it may remain.

Do not rename identifiers solely for aesthetics.

---

# 12. Task Ownership

Current task ownership includes:

```rust
TaskOwner::Plugin(String)
TaskOwner::Invocation(String)
TaskOwner::Surface(String)
TaskOwner::Process
```

Remove:

```rust
TaskOwner::Plugin(...)
```

Keep useful ownership concepts such as:

```text
Process
Run / Invocation
Surface
```

If `Invocation` remains the best term internally, it may remain.

Do not introduce tool/task ownership until tools exist.

---

# 13. Storage Refactor

## 13.1 Remove plugin storage APIs

Delete storage commands and methods such as:

```text
ListPlugins
SavePlugin
RemovePlugin
PluginKvGet
PluginKvSet
PluginKvDelete
StoredPlugin
```

Delete code paths that read/write installed plugin metadata.

---

## 13.2 Database migration policy

Do not destroy user conversation/history data.

Plugin data is obsolete and may be safely retired.

### Preferred migration behavior

- New databases must no longer create plugin/plugin-KV tables.
- Existing databases must continue opening successfully.
- A migration may drop obsolete plugin-only tables if doing so is deterministic and isolated.
- If dropping old tables creates disproportionate migration complexity, leaving unused legacy tables in existing DBs is acceptable **temporarily**, provided:
  - no product code reads them,
  - no new DB creates them,
  - migration/state versioning is correct,
  - the implementation log records the choice.

Conversation/history/settings data must not be reset.

---

# 14. Core Refactor

Audit:

```text
crates/lexwisp-core/src/plugin.rs
```

Most or all of this module should disappear.

Remove exports from:

```text
crates/lexwisp-core/src/lib.rs
```

for obsolete plugin/action types.

Do not leave:

```rust
#[allow(dead_code)]
```

as a substitute for removal.

Do not create compatibility aliases such as:

```rust
type ToolId = PluginId;
type RunAction = ActionDescriptor;
```

The obsolete concepts must actually leave the architecture.

---

# 15. Chat Code Relocation

The current Chat implementation lives under:

```text
crates/lexwisp-plugins-builtin/
```

That location becomes semantically wrong.

Move the code.

## Required outcomes

These concepts remain:

```text
ChatController
ChatExperience
ChatUiPort
ChatHistoryPort
ChatRunPort
conversation/message state
send/stop/retry semantics
```

These concepts disappear:

```text
chat_plugin()
chat_action()
PluginDescriptor for Chat
ActionDescriptor for Chat
Chat registration through registry
PluginId passed to ChatRunPort
```

---

# 16. UI Behavior After Refactor

## 16.1 Hotkey

Global hotkey:

```text
toggle/show floating chat
```

It must not:

```text
capture selection
choose action
run default action
```

unless a currently implemented Chat UX explicitly and independently requires context capture.

---

## 16.2 Main surface

The primary surface remains Chat.

Preserve:

- compact floating presentation
- expanded presentation if currently supported
- conversation continuity
- streaming answer
- send
- stop
- retry
- selectable Markdown
- provider/model controls currently part of Chat UX
- hidden/warm surface behavior
- correct window generation/lifecycle handling

---

## 16.3 Control Center

After refactor there must be no Plugins page or Plugin state.

Do not replace it with an empty “Extensions” page.

---

# 17. HostUiCommand Cleanup

Audit:

```text
HostUiCommand
```

Delete plugin commands such as:

```text
RefreshPlugins
```

and all handlers for them.

Keep only commands with current product meaning.

---

# 18. Security Simplification

Deleting the plugin platform should remove a significant amount of attack surface.

Delete plugin-only security mechanisms together with plugin execution:

- archive validation
- plugin path traversal protection
- plugin redirect authorization
- plugin capability manifests
- script network allowlists
- JS sandbox claims
- plugin package identity binding
- plugin generation revocation

Do **not** weaken security of real surviving functionality:

- Provider credentials remain in Windows Credential Manager
- no secret logging
- TLS remains enabled
- no silent credential forwarding
- no automatic paid POST retry unless existing policy explicitly allows it
- settings persistence remains atomic
- Windows unsafe code remains encapsulated

---

# 19. Documentation Cleanup

Update all architecture/product docs.

At minimum audit:

```text
AGENTS.md
README.md
docs/implementation-log.md
docs/plugin-schema.md
docs/script-plugin-api.md
LexWisp_UI_REDESIGN_SPEC.md
docs/design-system/*
examples/lexwisp-ui-lab/*
```

## 19.1 Delete obsolete docs

Delete documentation whose only purpose is the removed plugin system:

```text
docs/plugin-schema.md
docs/script-plugin-api.md
```

and equivalent plugin-only examples.

Do not edit the standalone UI lab unless the task explicitly allows it. If the UI lab contains old plugin wording but is governed by an independent boundary, leave it untouched and note it.

---

## 19.2 Rewrite AGENTS.md product boundary

The new product boundary must state clearly:

```text
LexWisp is a native Windows floating AI chat client.
Chat is a first-party product capability, not a plugin or action.
LexWisp does not support installable runtime plugins.
LexWisp does not use a selection-first Action architecture.
Future Agent abilities should be implemented as a small set of first-party built-in tools, not as arbitrary plugins.
Do not build speculative plugin, tool, agent, MCP, workflow, or generic command frameworks without a concrete requirement.
```

Retain valid existing engineering rules:

- Rust + GPUI + GPUI-Kit + windows-rs
- Host-owned shared runtime/resources
- typed boundaries
- no global AppState mutex
- correct GPUI state ownership
- streaming coalescing
- persistence semantics
- Windows window lifecycle
- performance discipline

---

# 20. Code Search Gate

Before considering the task complete, search the repository for obsolete vocabulary.

The main product workspace should have no active code references to:

```text
PluginId
PluginDescriptor
PluginManager
PluginRegistry
PluginManagement
ManagedPlugin
PluginImport
PluginKind
CapabilityAuthority
chat_plugin
chat_action
ActionRegistry
ActionDescriptor
ActionHandler
ActionKind
QualifiedActionId
DeclarativeActionDefinition
ScriptActionDefinition
lexwisp-plugins-script
rquickjs
RefreshPlugins
```

Some words may legitimately remain in:

- historical implementation logs
- archived migration notes
- third-party dependency notices

Prefer removing misleading active documentation references.

Do not mechanically rename `plugin` to another word while keeping the old architecture.

---

# 21. No Speculative Replacement Framework

This requirement is strict.

During this task, do **not** add:

```text
ToolRegistry
AgentRegistry
SkillRegistry
MCP host
generic function calling framework
workflow engine
planner
reflection bus
dynamic JSON command router
plugin compatibility shim
extension marketplace abstractions
```

The deletion task should make the system **smaller**.

A future Web Search implementation will introduce the minimal Tool abstraction when there is a real tool to execute.

---

# 22. Expected Composition Root After Refactor

The exact code may differ, but startup should conceptually resemble:

```rust
let (host, handles) = Host::build(...)?;

let settings = handles.settings();
let providers = handles.providers();
let history = handles.history();

let chat_controller = ChatController::new(
    handles.chat_run_port(),
    handles.chat_history(),
)?;

let chat: Arc<dyn ChatUiPort> = chat_controller.clone();

let shell_content_factory = {
    let chat = chat.clone();
    let providers = providers.clone();

    Rc::new(move |controller, session, window, cx| {
        cx.new(|cx| {
            ChatExperience::new(
                controller,
                session,
                chat.clone(),
                providers.clone(),
                window,
                cx,
            )
        })
        .into()
    })
};
```

There should be no:

```text
plugin construction
action construction
registry registration
capability grants
installed-plugin activation
action descriptor list
```

---

# 23. Implementation Order

Implement in this order to keep the tree buildable as much as practical.

## Phase 1 — Decouple Chat from Plugin/Action identity

1. Remove `PluginId` from `ChatRunPort`.
2. Refactor `InvocationSupervisor` so Chat can invoke directly.
3. Remove Chat registration through PluginRegistry/ActionRegistry.
4. Make application startup construct Chat directly.
5. Verify Chat send/stream/stop/retry still works.

Do not delete the registries before Chat no longer depends on them.

---

## Phase 2 — Remove UI Action/Plugin wiring

1. Remove `action_descriptors`.
2. Remove `plugin_management`.
3. Remove Plugins controls/pages.
4. Remove action palette/default-action behavior.
5. Remove `RefreshPlugins`.
6. Make hotkey open Chat directly.

---

## Phase 3 — Remove selection-first Action flow

1. Audit hotkey launch context.
2. Remove selection capture from the required launch path.
3. Remove action input-source concepts.
4. Remove source replacement workflow.
5. Keep only Windows context utilities that still serve a concrete surviving use.

---

## Phase 4 — Remove plugin runtime

Delete:

```text
PluginManager
Declarative runtime
Script runtime
Plugin Registry
CapabilityAuthority
Plugin lifecycle
Plugin package IO
Plugin storage
```

---

## Phase 5 — Remove dead core/storage types

1. Delete plugin/action structs/enums/traits.
2. Remove obsolete DB commands.
3. Update execution/history schemas.
4. Remove dead exports.
5. Remove dead dependencies.

---

## Phase 6 — Relocate Chat and collapse crates

1. Move Chat code out of `lexwisp-plugins-builtin`.
2. Remove `lexwisp-plugins-builtin` if empty.
3. Delete `lexwisp-plugins-script`.
4. Update workspace members/dependencies.

---

## Phase 7 — Documentation and final cleanup

1. Rewrite AGENTS.md.
2. Update README.
3. Delete plugin API docs.
4. Record the architectural migration in implementation log.
5. Run repository-wide dead vocabulary search.

---

# 24. Functional Acceptance Criteria

All conditions below must pass.

## 24.1 Startup

- LexWisp starts normally.
- No plugin directory is required.
- No PluginManager is created.
- No Script runtime is initialized.
- No plugin activation runs.
- No action registration runs.

---

## 24.2 Chat

- Global hotkey opens/toggles Chat.
- Composer receives focus correctly.
- User can send a message.
- Streaming output works.
- Stop works.
- Retry works.
- Conversation state remains stable.
- History persists.
- Provider/model configuration still works.
- Closing/hiding/reopening the surface preserves intended state.

---

## 24.3 Windows lifecycle

- Tray behavior still works.
- Startup registration still works.
- Hotkey replacement still works.
- Explorer restart handling still works where previously implemented.
- Explicit quit performs bounded shutdown.
- No hidden GUI keeper window is introduced.

---

## 24.4 Persistence

- Existing conversations/history remain readable.
- New conversations persist.
- Favorites/history behavior that still has product meaning remains functional.
- Settings are not reset.
- Provider credentials are preserved.

---

## 24.5 Removed behavior

The product must no longer expose:

- plugin installation
- plugin import
- plugin enable/disable
- plugin reload
- plugin uninstall
- script plugins
- declarative plugins
- action palette
- default action
- selection-based action dispatch
- action-based replace-source flow

---

# 25. Build / Quality Gates

Run from the real workspace using the locked Windows target.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
```

Also run repository searches for deleted concepts.

Example:

```powershell
rg -n "PluginId|PluginDescriptor|PluginManager|PluginRegistry|PluginManagement|ManagedPlugin|PluginImport|CapabilityAuthority|chat_plugin|chat_action|ActionRegistry|ActionDescriptor|ActionHandler|ActionKind|QualifiedActionId|DeclarativeActionDefinition|ScriptActionDefinition|rquickjs|RefreshPlugins" .
```

Review every remaining match manually.

Historical text may remain only when clearly historical and not normative.

---

# 26. Runtime Verification

Launch the actual Release build.

Verify manually:

1. Start LexWisp.
2. Open Chat with hotkey.
3. Send a normal message.
4. Receive streaming text.
5. Stop a streaming request.
6. Retry a message.
7. Hide Chat.
8. Reopen Chat.
9. Switch/configure Provider where currently supported.
10. Open history.
11. Restart LexWisp and verify conversation persistence.
12. Verify tray and Quit behavior.
13. Verify no Plugins UI exists.
14. Verify no action chooser appears.
15. Verify hotkey does not require selected text.

Record actual results in `docs/implementation-log.md`.

Do not fabricate unavailable Windows/IME/native verification.

---

# 27. Performance / Dependency Acceptance

The resulting application should be simpler than before.

Record:

- Release binary size
- dependency delta
- whether QuickJS disappeared from `Cargo.lock`
- whether ZIP/plugin-specific dependencies disappeared
- startup behavior
- process thread count if already part of project measurements
- memory measurements only if existing measurement tooling is available

Do not add artificial optimization work solely to improve these numbers.

---

# 28. Migration Safety

This is an architectural deletion, not a rewrite.

Preserve user-visible behavior that belongs to the new product:

```text
Chat
Provider configuration
History
Persistence
Settings
Windows shell integration
UI design system
```

Do not opportunistically rewrite:

- GPUI component architecture
- theme system
- SQLite architecture
- Provider HTTP implementation
- Markdown rendering
- conversation semantics

unless a change is directly required by removing Plugin/Action dependencies.

---

# 29. Definition of Done

This task is complete only when:

1. **Chat no longer exists as a Plugin.**
2. **Chat no longer exists as an Action.**
3. **Plugin System is absent from active product architecture.**
4. **Legacy selection-first Action architecture is absent from active product architecture.**
5. **QuickJS/Script Plugin infrastructure is removed.**
6. **Plugin management UI is removed.**
7. **Plugin/action storage APIs are removed or fully retired.**
8. **Execution supervision continues to support Chat streaming/cancel/persistence.**
9. **Global hotkey opens Chat directly.**
10. **No speculative Agent/Tool framework is introduced.**
11. **The workspace passes format/check/clippy/test/release-build gates.**
12. **The Release build is manually verified on Windows.**
13. **AGENTS.md and README describe the new product truthfully.**
14. **Repository-wide searches show no accidental active dependency on deleted concepts.**

---

# 30. Final Architecture Constraint

After this refactor, maintain this rule:

> LexWisp is a native Windows floating AI chat client. First-party Chat behavior is compiled into the product. Future Agent abilities must be added as narrowly scoped built-in tools only when concrete functionality requires them. Runtime-installable plugins and selection-first Actions are not part of the product architecture.

This rule should be treated as a product boundary, not a temporary implementation shortcut.
