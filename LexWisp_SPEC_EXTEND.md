# LexWisp SPEC EXTEND

**版本：1.0｜日期：2026-09-18｜性质：`LexWisp_IMPLEMENTATION_SPEC.md` 的补充规范**

> 本文不是新的总 SPEC，也不替换原 SPEC。  
> 本文仅补足原 SPEC 中已经存在、但实现约束尚可进一步明确的五个工程点：
>
> 1. `SurfaceFactory / Window Helper`
> 2. `HostHandles`
> 3. `ChatController / Chat Service`
> 4. `ExecutionStore + SQLite` 批量 checkpoint
> 5. 类型化局部事件
>
> 这些扩展吸收 AgentX 当前工程中已经出现的可取模式，但不复制其整体架构。LexWisp 原 SPEC 中关于 **Host＋插件、单一资源所有权、唯一 Tokio Runtime、Invocation 监管、插件权限、窗口生命周期、GPUI 主线程边界** 的约束继续有效。

---

## 0. 文档优先级与使用方式

### 0.1 与原 SPEC 的关系

本文按以下优先级使用：

1. 用户已经确认的产品行为与边界
2. `LexWisp_IMPLEMENTATION_SPEC.md`
3. 本 `SPEC EXTEND`
4. 实现阶段中的局部技术决定

如果本文与原 SPEC 的产品行为冲突，以原 SPEC 为准。本文只允许：

- 将原有概念进一步具体化；
- 补充缺失的所有权、接口和生命周期说明；
- 降低 Stage 0–4 的返工概率；
- 为 Coding Agent 提供更直接的实现边界。

本文不允许借“扩展”为名改变：

- Host＋插件结构；
- Native / Declarative / Script 三类插件；
- 唯一 Host Tokio Runtime；
- GPUI 主线程只负责 UI；
- Chat 收起继续、Translate / Polish 默认收起取消；
- OpenAI Compatible Provider；
- 手动确认替换原文；
- Windows 绿色版目标；
- 权限、任务和 Invocation 监管模型。

### 0.2 AgentX 的借鉴原则

AgentX 只作为**实现案例库**。

可以借：

- 统一创建 GPUI Window 与 `Root` 的方式；
- 后台任务持有可 clone 的 Service Handle，而不是依赖 UI 全局状态；
- Chat 的 Controller / Service 层；
- Streaming chunk 合并与批量持久化；
- 用明确领域事件连接少数模块。

不直接借：

- 巨型全局 `AppState`；
- 动态 Service Locator；
- 全应用通用 EventBus；
- 任意 `spawn(...).detach()`；
- Service 自己兜底创建新的 Tokio Runtime；
- UI、业务、持久化之间通过无界事件不断转发；
- Agent / Dock / IDE 产品结构。

---

# 1. 扩展后的核心数据流

五项扩展合并后，LexWisp 第一阶段的主要数据流应收敛为：

```text
Windows / User Intent
        │
        ▼
   UiCommand / Action
        │
        ▼
┌──────────────────────┐
│     HostHandles      │
│  强类型共享服务句柄   │
└──────────────────────┘
        │
        ├──────────────► ActionRegistry / PluginContext
        │
        ├──────────────► InvocationSupervisor
        │
        ├──────────────► AIService / ProviderRegistry
        │
        └──────────────► Storage / Settings / Capability
                              │
                              ▼
                       ExecutionStore
                              │
                  ┌───────────┴───────────┐
                  │                       │
                  ▼                       ▼
          UI Projection             CheckpointProjector
          ~33 ms 合并               ~500 ms / 16 KiB
                  │                       │
                  ▼                       ▼
             GPUI Entity                 SQLite
```

Chat 在这条链路中不是特殊旁路：

```text
Quick Shell / Chat Panel
          │
          ▼
    ChatController
          │
          ▼
      ChatHostPort
          │
          ▼
  InvocationSupervisor
          │
          ▼
      ExecutionStore
```

Chat 的两个 Surface 共用同一个 Controller 和同一份 Conversation 状态。

---

# 2. SurfaceFactory / Window Helper

## 2.1 目的

原 SPEC 已经定义三个主要 Surface：

- Quick Shell
- Chat Panel
- Control Center

并要求：

- 每个真实窗口只有一个顶层 Kit `Root`；
- Popup 支持 `NotCreated → Visible → HiddenWarm → Destroyed`；
- Chat Panel / Control Center 全局至多一个；
- Surface handoff 不得导致请求重发或错误取消。

需要补足的问题是：

> **窗口本身由谁创建、公共 WindowOptions 如何复用、Root / Theme / Overlay 如何统一、WindowRegistry 如何与各 Surface 生命周期衔接。**

因此引入 `SurfaceFactory`。

---

## 2.2 定位

`SurfaceFactory` 是 **UI Shell 的窗口创建入口**。

它负责：

- 把逻辑 Surface 请求变成真实 GPUI Window；
- 统一窗口级基础配置；
- 统一 Kit `Root` 与 Overlay 初始化；
- 连接 `WindowRegistry`；
- 创建或复用正确的 View；
- 返回可以被 UI Shell 管理的 Surface Handle。

它不负责：

- AI；
- Chat 业务；
- Action 路由；
- Provider；
- 数据库存取；
- 插件权限；
- Windows 选区捕获；
- Invocation 生命周期；
- 持久化。

---

## 2.3 所在层

建议位于：

```text
lexwisp-ui
└── surface/
    ├── mod.rs
    ├── factory.rs
    ├── registry.rs
    ├── spec.rs
    └── root.rs
```

第一阶段不要求真的拆成以上所有文件。

只在代码已经形成真实职责后拆分。

### 依赖方向

```text
lexwisp-ui
    ↓
lexwisp-core
```

`SurfaceFactory` 可以接触 GPUI / GPUI-Kit。

`lexwisp-core` 不得出现：

- `Window`
- `Entity`
- `AnyView`
- `Root`
- GPUI Context
- GPUI Element

Host 通过 `HostUiCommand` 请求窗口操作，由 `app` / UI bridge 在 GPUI 主线程转给 `SurfaceFactory`。

---

## 2.4 最小类型

以下是语义示意，不要求逐字照抄：

```rust
pub enum SurfaceKind {
    QuickShell,
    ChatPanel,
    ControlCenter,
}

pub struct SurfaceKey {
    pub kind: SurfaceKind,
    pub instance: Option<SurfaceInstanceId>,
}

pub struct SurfaceOpenRequest {
    pub key: SurfaceKey,
    pub activation: ActivationPolicy,
    pub placement: PlacementHint,
}

pub enum SurfaceState {
    NotCreated,
    Visible,
    HiddenWarm,
    Destroyed,
}
```

首版三个 Surface 都是单实例，因此：

```text
QuickShell      → 全局 1 个
ChatPanel       → 全局 1 个
ControlCenter   → 全局 1 个
```

`SurfaceInstanceId` 仍保留在内部模型中，以便未来插件受控 Surface 扩展，但 Stage 0–4 不建立任意多窗口系统。

---

## 2.5 SurfaceFactory 的职责

建议接口语义：

```rust
pub struct SurfaceFactory {
    registry: WindowRegistry,
    views: SurfaceViewRegistry,
}

impl SurfaceFactory {
    pub fn show(
        &mut self,
        request: SurfaceOpenRequest,
        cx: &mut App,
    ) -> Result<SurfaceHandle>;

    pub fn hide(
        &mut self,
        key: &SurfaceKey,
        cx: &mut App,
    ) -> Result<()>;

    pub fn destroy(
        &mut self,
        key: &SurfaceKey,
        cx: &mut App,
    ) -> Result<()>;
}
```

这里的 `show()` 应执行：

```text
查 WindowRegistry
    │
    ├─ Visible
    │    └─ activate / focus
    │
    ├─ HiddenWarm
    │    └─ show existing window
    │
    └─ NotCreated / Destroyed
         └─ create new GPUI Window
              ├─ WindowOptions
              ├─ Root
              ├─ Theme
              ├─ Overlay
              ├─ View Factory
              └─ Registry entry
```

---

## 2.6 WindowOptions 统一构建

不要让三个 Surface 各自复制：

```rust
WindowOptions { ... }
```

建立小型 Window helper：

```rust
fn build_window_options(
    kind: SurfaceKind,
    display: DisplaySnapshot,
    placement: PlacementHint,
) -> WindowOptions;
```

内部允许按 Surface 选择：

- WindowKind；
- min size；
- initial size；
- decoration；
- titlebar；
- resize policy；
- focus policy；
- transparency；
- initial bounds。

但共同处理：

- DPI；
- 当前显示器；
- work area；
- 负坐标；
- 不越过任务栏；
- 默认最小尺寸；
- 窗口激活。

### 不要做成通用 UI DSL

禁止引入：

```text
WindowBuilder<T>
SurfaceBuilder<T>
DynamicWindowSchema
```

除非已经出现真实需要。

Stage 0 只需要一个最小函数，Stage 1 再自然收敛为 `SurfaceFactory`。

---

## 2.7 Root 统一

每个真实 Window 必须只建立一个 GPUI-Kit `Root`。

建议：

```text
GPUI Window
   │
   ▼
LexWispWindowRoot
   ├── Surface Content
   ├── Dialog Layer
   ├── Sheet Layer
   └── Notification Layer
```

`LexWispWindowRoot` 只处理窗口级框架，不处理业务。

例如：

```rust
struct LexWispWindowRoot {
    content: AnyView,
}
```

不要为：

- Quick Shell
- Chat Panel
- Control Center

分别维护三套 Root / Overlay 初始化代码。

---

## 2.8 View 注册

Host 仍然不能直接持有 GPUI View。

建议：

```text
SurfaceDescriptor     core 数据
        │
        ▼
SurfaceViewRegistry   ui 层
        │
        ▼
ViewFactory           GPUI 主线程调用
```

Native Plugin 的 Surface：

```text
PluginId + SurfaceId
        │
        ├─ Descriptor
        └─ ViewFactory
```

必须在注册期完成配对。

缺少 ViewFactory 的 Surface 不得等到用户打开窗口时才 panic。

---

## 2.9 Surface 与业务状态分离

窗口被销毁：

```text
Chat Panel Window Destroyed
```

不等于：

```text
Conversation Destroyed
ChatController Destroyed
Invocation Cancelled
```

Quick Shell 的 View 也不能成为 Invocation 的唯一所有者。

正确关系：

```text
Surface
   │ observes
   ▼
Controller / ExecutionStore
```

不是：

```text
Surface
   │ owns
   ▼
Network Task
```

---

## 2.10 HiddenWarm

Quick Shell：

```text
Visible
  │
  ▼
HiddenWarm
  │ 30 s default
  ├──────────────► Visible
  │
  ▼
Destroyed
```

`HiddenWarm` 允许保留：

- InputState；
- 当前轻量 draft；
- scroll state；
- 当前 Surface binding；
- 必要 Entity。

禁止保留：

- 仅为了“以后可能用”的大列表；
- 历史全文缓存；
- 独立 HTTP Client；
- 独立 Runtime；
- 不可见时持续动画；
- 不可见时固定刷新 ticker。

---

## 2.11 Surface generation

每次 Window 真正 Destroy 后重新创建：

```text
surface_generation += 1
```

来自旧窗口的异步 UI 回调必须同时验证：

```text
SurfaceKey
SurfaceGeneration
```

旧 Window 的：

- timer；
- delayed callback；
- weak update；
- focus restore；
- menu callback

不得作用到新 Surface。

---

## 2.12 Stage 对应

### Stage 0

只完成：

- 一个最小 `SurfaceFactory` 骨架；
- `WindowOptions` helper；
- `LexWispWindowRoot`；
- Quick Shell test surface；
- Root / Input / Button / selectable text；
- Release 启动。

不要提前做完整 registry。

### Stage 1

补：

- `WindowRegistry`；
- Visible / HiddenWarm / Destroyed；
- Quick Shell 30 秒 warm；
- Chat Panel / Control Center 单实例策略；
- `HostUiCommand → SurfaceFactory` bridge。

### Stage 4

补：

- Quick Shell → Chat Panel handoff；
- Chat Surface ViewFactory；
- 两个 Surface 观察同一个 ChatController。

---

## 2.13 验收

至少验证：

1. 三种窗口不复制 Root 初始化逻辑；
2. Quick Shell HiddenWarm 后恢复同一个轻量 UI 状态；
3. Destroy 后重建 generation 改变；
4. 旧窗口 callback 不更新新窗口；
5. 关闭 Chat Panel 不取消继续中的 Chat；
6. WindowFactory 不包含 Chat / Translate / Polish 业务分支；
7. 无可见 Window 时不需要隐藏 GPUI 主窗口维持进程。

---

# 3. HostHandles

## 3.1 目的

AgentX 中一个有价值的模式是：

> 后台任务取得可 clone 的服务集合，而不是回到 GPUI 全局 `AppState` 中寻找业务 Service。

LexWisp 需要吸收这个思想，但不能变成动态 Service Locator。

因此引入：

```text
HostHandles
```

---

## 3.2 定义

`HostHandles` 是：

> **Composition Root 完成 Host 构建后形成的一组强类型、可 clone、不可随意替换的服务句柄。**

它的目的：

- 后台任务不访问 GPUI global state；
- Service 共享唯一资源；
- 异步 closure 可以安全持有所需能力；
- 所有权明确；
- 避免到处传十几个参数。

它不是：

- DI Container；
- `HashMap<TypeId, Box<dyn Any>>`；
- Service Locator；
- 动态注册框架；
- 插件可以任意访问的 Host 对象。

---

## 3.3 建议结构

示意：

```rust
#[derive(Clone)]
pub struct HostHandles {
    pub invocations: Arc<InvocationSupervisor>,
    pub executions: Arc<ExecutionStore>,
    pub ai: Arc<AIService>,
    pub providers: Arc<ProviderRegistry>,
    pub settings: Arc<SettingsService>,
    pub storage: Arc<StorageService>,
    pub capabilities: Arc<CapabilityAuthority>,
    pub plugins: Arc<PluginRegistry>,
    pub actions: Arc<ActionRegistry>,
    pub ui: HostUiCommandPort,
    pub tasks: HostTaskPort,
}
```

是否全部公开字段由实现决定。

更推荐：

```rust
impl HostHandles {
    pub fn ai(&self) -> &Arc<AIService>;
    pub fn executions(&self) -> &Arc<ExecutionStore>;
}
```

避免外部任意替换内部成员。

---

## 3.4 不使用 Option Service

完成 Composition Root 后：

```text
HostHandles
```

中的必要服务必须已经可用。

不要大量出现：

```rust
Option<Arc<AIService>>
Option<Arc<Storage>>
Option<Arc<Settings>>
```

然后运行时再判断：

```text
not initialized
```

如果某服务属于阶段性未实现功能：

- 不注册对应 Action；
- UI 不展示对应入口；
- 或单独使用明确的 Feature State。

不要把“程序尚未初始化完成”长期编码成所有 Service 都是 `Option`。

---

## 3.5 Composition Root

建议：

```text
lexwisp-app
   │
   ▼
CompositionRoot::build()
   │
   ├─ Runtime
   ├─ Storage
   ├─ Settings
   ├─ Credentials
   ├─ HTTP
   ├─ ProviderRegistry
   ├─ AIService
   ├─ ExecutionStore
   ├─ InvocationSupervisor
   ├─ CapabilityAuthority
   ├─ PluginRegistry
   ├─ ActionRegistry
   └─ HostUiCommandPort
          │
          ▼
     HostHandles
```

构建顺序必须显式。

不通过隐式全局 lazy singleton 拼装 Host。

---

## 3.6 与唯一 Tokio Runtime 的关系

原 SPEC 的“唯一 Host Runtime”保持不变。

`HostHandles` 不直接暴露：

```rust
&tokio::runtime::Runtime
```

给业务。

建议暴露：

```text
HostTaskPort
```

或：

```rust
struct TaskSpawner {
    handle: tokio::runtime::Handle,
    supervisor: ...
}
```

所有新任务必须带：

```text
owner
scope
cancellation token
```

例如：

```rust
TaskOwner::Process
TaskOwner::Plugin(plugin_id, generation)
TaskOwner::Invocation(invocation_id)
```

禁止：

```rust
tokio::spawn(...)
```

散落业务代码。

禁止 Service 自己：

```text
Handle::try_current()
失败
→ new Runtime()
```

---

## 3.7 HostHandles 与 PluginContext

这是必须明确的边界：

> **插件不能拿到 HostHandles。**

Host 内部：

```text
HostHandles
    │
    ▼
CapabilityAuthority
    │
    ▼
derive scoped ports
    │
    ▼
PluginContext
```

插件得到：

```text
PluginContext
├── AI Port
├── Storage Namespace Port
├── Context Port
├── Output Port
├── UI Request Port
└── Cancellation
```

这些 Port 已绑定：

- PluginId；
- Plugin generation；
- grant generation；
- capability scope。

Script 更不能接触 HostHandles。

---

## 3.8 HostHandles 与 InvocationContext

创建 Invocation 时：

```text
HostHandles
    │
    ▼
InvocationSupervisor
    │
    ▼
InvocationContext
```

`InvocationContext` 固定：

- InvocationId；
- PluginId；
- PluginGeneration；
- request snapshot；
- provider/model snapshot；
- cancellation token；
- output sink；
- budget；
- retention generation。

这样设置变化不会改变已经运行中的请求。

---

## 3.9 HostHandles 与 UI

UI 可以拿到**只够用的 facade**。

不要：

```text
GPUI View
  ↓
HostHandles 全部能力
```

建议：

```text
QuickShellModel
ChatUiPort
SettingsUiPort
HistoryUiPort
```

这些 facade 内部持有需要的 Handle。

只有 app / UI bridge 可以持有完整 HostHandles，用来构造各 Controller / Model。

---

## 3.10 生命周期

HostHandles 的生命周期：

```text
Process Start
    │
    ▼
Build once
    │
    ▼
Clone lightweight Arc handles
    │
    ▼
Reject new work during shutdown
    │
    ▼
Drop after supervised cleanup
```

`HostHandles.clone()` 必须是轻量 clone。

不允许 clone 时：

- 新建 Client；
- 新建 Runtime；
- 打开 SQLite；
- 新建线程；
- 复制大配置；
- 启动监听器。

---

## 3.11 Stage 对应

### Stage 1

正式建立 HostHandles。

Stage 1 的第一版只包含真实存在的服务。

例如：

```text
Settings
TaskSupervisor
PluginRegistry
ActionRegistry
CapabilityAuthority
UiCommandPort
```

Stage 2 再加入：

```text
AIService
ProviderRegistry
ExecutionStore
Storage
```

结构可以扩展，但不要提前放空字段。

---

## 3.12 验收

1. 后台任务不通过 GPUI `AppState::global()` 获得 Host Service；
2. 程序中只有一个 Host Tokio Runtime；
3. `HostHandles.clone()` 不创建新资源；
4. 插件无法获得 HostHandles；
5. Script 无法获得裸 reqwest / SQLite / Credential；
6. GPUI View 不直接持有数据库连接；
7. Service 之间不依赖动态类型查找。

---

# 4. ChatController / Chat Service

## 4.1 为什么提前到 Stage 2

原 SPEC 将真正的多会话 Chat Panel 放在 Stage 4，这是正确的。

但如果 Stage 2 的第一版 Chat 直接写成：

```text
Quick Shell View
    ├─ send request
    ├─ append text
    ├─ stop
    └─ save DB
```

Stage 4 增加多会话时几乎一定要重构。

因此：

> **Stage 2 就建立 ChatController，但 Stage 2 只要求一个活动 Conversation。**

Stage 4 再扩展会话数量和独立 Panel。

---

## 4.2 所有权

ChatController 属于：

```text
lexwisp-plugins-builtin
```

它负责 Chat 的**产品语义**。

Host 不知道：

- 什么叫“最后一条 assistant 回复”；
- 什么叫“重新生成”；
- 如何选择 Chat 上下文轮次；
- Quick Ask 当前绑定哪个 Conversation；
- Chat title 如何生成。

这些属于 Chat Plugin。

---

## 4.3 ChatController 不直接依赖 Host 实现

根据原 SPEC 的依赖方向：

```text
plugins-builtin
    ↓
core + ui
```

不能：

```text
plugins-builtin
    ↓
lexwisp-host
```

因此 ChatController 使用 core 中定义的最小端口。

例如：

```rust
pub trait ChatHostPort {
    fn start_chat_invocation(...);
    fn cancel_invocation(...);
    fn load_conversation(...);
    fn save_conversation_metadata(...);
    fn subscribe_execution(...);
}
```

实际实现由 Host 提供。

也可以拆成更小端口，但不要机械过拆。

---

## 4.4 ChatController 职责

至少负责：

```text
Conversation identity
Message ordering
Draft
Current generation
Send
Stop
Regenerate
Context assembly
Attempt policy
Conversation title
Model preference
Execution binding
Surface observers
```

不负责：

```text
HTTP
OpenAI payload
API Key
SQLite SQL
GPUI Window creation
Credential Manager
Global hotkey
UI Automation
```

---

## 4.5 Stage 2 最小模型

即使只有一个会话，也直接使用稳定 ID：

```rust
struct ConversationId(...);
struct MessageId(...);
struct AttemptId(...);
```

不要 Stage 2 使用：

```text
Vec<Message>
```

到 Stage 4 再补 ID。

Stage 2：

```text
ChatController
└── active_conversation: ConversationId
```

Stage 4：

```text
ChatController
├── active_conversation
├── conversation index
└── multiple conversations
```

这样核心发送链路不变。

---

## 4.6 消息状态

建议：

```text
Draft
Submitted
Generating
Completed
CancelledPartial
FailedPartial
```

真正终态仍来自 InvocationSupervisor。

ChatController 只把 Execution 结果解释为 Chat 语义。

例如：

```text
Execution Completed
    ↓
Assistant attempt = Completed

Execution Cancelled + partial text
    ↓
Assistant attempt = CancelledPartial
```

---

## 4.7 Send 流程

```text
User presses Send
      │
      ▼
ChatController.validate()
      │
      ▼
Create user Message
      │
      ▼
Immediately expose local state to UI
      │
      ▼
Assemble context
      │
      ▼
ChatHostPort.start_chat_invocation()
      │
      ▼
InvocationId
      │
      ▼
Bind assistant attempt ↔ Invocation
      │
      ▼
Observe ExecutionStore
```

必须做到：

> 用户点击 Send 后立即看见本地提交状态，不等待网络首字。

---

## 4.8 同会话并发

保持原 SPEC：

```text
one conversation
    └── max 1 active generation
```

如果正在生成：

- Send 默认被阻止；
- UI 提供 Stop；
- 不暗中创建第二条 assistant generation。

不同 Conversation 在 Stage 4 可以并行。

---

## 4.9 Regenerate

`regenerate_last_reply()`：

```text
Conversation
   │
   ├─ old assistant attempt
   └─ new assistant attempt
```

不能：

- 覆盖旧 Message ID；
- 重用旧 InvocationId；
- 删除旧结果后假装从未发生。

默认 UI 展示最新成功 attempt。

---

## 4.10 上下文组装

ChatController 负责决定：

- system 指令；
- 当前 conversation；
- 最近完整轮次；
- 本次 user message；
- 被选中文字形成的显式 context block；
- 预算裁剪。

Host 只执行已经确定的 AI Request Snapshot。

禁止 Host：

```text
看到这是 Chat
→ 偷偷加窗口标题
→ 偷偷加剪贴板
→ 偷偷加历史
```

---

## 4.11 Quick Shell 与 Chat Panel

两个 Surface：

```text
QuickShellChatView
ChatPanelView
```

必须观察：

```text
same ChatController
```

而不是：

```text
QuickShellChatController
ChatPanelChatController
```

### Handoff

```text
Quick Shell
   │
   │ attach Chat Panel
   ▼
Chat Panel observing
   │
   │ detach Quick Shell
   ▼
same Invocation continues
```

顺序必须：

```text
attach target
→ confirm target observer
→ detach source
```

---

## 4.12 ChatController 与 GPUI Entity

Controller 的业务状态不能只存在于 Window Entity 中。

可以采用：

```text
ChatController core state
       │
       ▼
ChatViewModel / Projection
       │
       ▼
GPUI Entity
```

窗口销毁时丢掉 ViewModel 也不能丢：

- Conversation；
- Message；
- Invocation binding；
- generation；
- partial result。

---

## 4.13 Stage 对应

### Stage 2

完成：

- 一个默认 Conversation；
- send；
- stop；
- retry / regenerate 最小语义；
- streaming；
- copy；
- persistence；
- Quick Shell Chat View。

### Stage 4

扩展：

- conversation create；
- list；
- rename；
- switch；
- delete；
- restore；
- Chat Panel；
- handoff；
- long list virtualization。

---

## 4.14 验收

1. Stage 2 就存在稳定 `ConversationId`；
2. Quick Shell 不直接调用 reqwest；
3. ChatController 不直接执行 SQL；
4. 关闭 Quick Shell 后 Chat 继续；
5. 重新打开能看到同一 Invocation；
6. Stage 4 增加 Chat Panel 不需要重写发送链路；
7. Handoff 不产生第二次 API 请求；
8. regenerate 使用新 InvocationId；
9. 同会话不能同时两次生成。

---

# 5. ExecutionStore 与 SQLite 批量 Checkpoint

## 5.1 目的

Streaming 有三个不同频率：

```text
Network chunk        高频、不可预测
UI refresh           约 33 ms
Persistence          约 500 ms / 16 KiB
```

不能把三者绑定成：

```text
收到 token
→ 更新整个 UI
→ 执行一次 SQL
```

因此补充：

```text
ExecutionAccumulator
CheckpointProjector
```

---

## 5.2 ExecutionStore 仍是唯一运行时权威

保持原 SPEC：

> ExecutionStore 是运行中结果的唯一权威。

因此：

```text
Network
   │
   ▼
InvocationSupervisor
   │
   ▼
ExecutionStore
```

所有：

- TextDelta；
- Progress；
- cancel；
- failure；
- completion

先进入 ExecutionStore。

UI 和 SQLite 都只是 Projection。

---

## 5.3 ExecutionAccumulator

每个活动 Invocation 对应一个 accumulator。

示意：

```rust
struct ExecutionAccumulator {
    invocation_id: InvocationId,
    plugin_id: PluginId,
    plugin_generation: u64,

    sequence: u64,

    text: String,
    text_version: u64,

    status: ExecutionStatus,

    dirty_since_ui: bool,
    dirty_since_checkpoint: bool,
    dirty_bytes: usize,

    last_ui_flush: Instant,
    last_checkpoint: Instant,
}
```

真实实现可以避免单个 `String` 反复复制，例如内部使用增量缓冲。

关键语义不变。

---

## 5.4 网络输入

网络 SSE：

```text
raw network chunk
      │
      ▼
SSE decoder
      │
      ▼
semantic provider event
      │
      ▼
ExecutionStore.apply(event)
```

不要把原始 TCP chunk 直接写数据库。

不要假定：

```text
1 HTTP chunk = 1 JSON = 1 token
```

---

## 5.5 sequence

每个 Invocation：

```text
sequence = strictly monotonic
```

每次业务增量推进 sequence。

UI / persistence 投影携带：

```text
InvocationId
PluginGeneration
Sequence
TextVersion
Status
```

晚到旧写入不得覆盖新状态。

---

## 5.6 UI Flush

默认：

```text
~33 ms
```

如果 33 ms 内收到：

```text
10
20
50
100
```

个 token，只需要：

```text
1 次 UI projection notify
```

终态：

```text
Completed
Failed
Cancelled
```

立即 flush，不等待 timer。

后台不可见窗口：

- 不跑 UI ticker；
- 下次显示时直接读取最新 Snapshot。

---

## 5.7 Persistence Checkpoint

触发条件：

```text
elapsed >= 500 ms
OR
dirty_bytes >= 16 KiB
OR
terminal state
```

这些值继续作为原 SPEC 中的工程初值。

Checkpoint 包含：

```text
InvocationId
Sequence
Status
Provider snapshot ref
Model snapshot
Input / Output projection
ConversationId optional
MessageId optional
AttemptId optional
UpdatedAt
RetentionGeneration
```

---

## 5.8 Chat 与非 Chat 的正文唯一真源

保持原 SPEC：

### Chat

```text
messages
```

保存权威消息正文。

`executions` 只保存：

- Invocation metadata；
- provider / model snapshot；
- timing；
- status；
- message relation；
- error；
- attempt relation。

不要在：

```text
messages.content
executions.output
```

同时永久保存同一份 Chat assistant 正文。

### 非 Chat Action

例如：

- Translate
- Polish
- Declarative Action
- Script Action

可以由 `executions` 保存实际输入与输出。

---

## 5.9 CheckpointProjector

建议形成：

```text
ExecutionStore
      │
      ▼
CheckpointProjector
      │
      ▼
StorageCommand
      │
      ▼
SQLite worker
```

`CheckpointProjector` 的工作：

- coalesce 多次更新；
- 决定何时 flush；
- 生成不可变 checkpoint；
- 丢弃明显过时 checkpoint；
- 终态强制 flush。

它不执行 SQL。

---

## 5.10 StorageCommand 必须有界

SQLite worker 使用原 SPEC 的：

```text
single worker
single main connection
bounded queue
```

Checkpoint 不得通过：

```text
unbounded_channel
```

无限堆积。

当 Storage 慢时：

1. 内存 ExecutionStore 继续保留最新结果；
2. 多个未写 checkpoint 合并为较新的一个；
3. 不因为磁盘慢而无限阻塞网络读取；
4. 终态进入高优先级可靠队列；
5. 队列持续饱和时向用户报告“未保存”。

---

## 5.11 Coalescing

若队列中已有：

```text
Invocation A
sequence 100
```

新的：

```text
Invocation A
sequence 140
```

尚未写入时，可以用 140 替换 100。

不要依次写：

```text
100
101
102
...
140
```

但不能合并掉具有独立业务意义的：

- user message submit；
- terminal state；
- delete barrier；
- retention generation change。

---

## 5.12 SQLite 侧 sequence guard

数据库写入同样检查：

```text
incoming_sequence > persisted_sequence
```

旧 checkpoint 即使晚到，也不能覆盖新 checkpoint。

可以采用：

```text
UPDATE ... WHERE persisted_sequence < ?
```

或事务内显式比较。

具体 SQL 由 storage crate 决定。

---

## 5.13 Terminal flush

终态顺序：

```text
ExecutionStore commit terminal
        │
        ├─ UI immediate notify
        │
        └─ terminal checkpoint
```

终态持久化失败：

- UI 状态仍然是实际终态；
- 记录标记未保存；
- 不把业务结果改成 Failed；
- 允许存储重试。

“数据库写失败”和“模型生成失败”是两种不同错误域。

---

## 5.14 Cancel

取消：

```text
Running
→ Cancelling
→ Cancelled
```

已有 partial text：

- 保留；
- UI 可复制；
- 按记录设置 checkpoint；
- 不因为取消删除。

---

## 5.15 Delete barrier

正在运行时删除 Execution / Conversation：

```text
retention_generation += 1
```

后续旧 generation checkpoint：

```text
reject
```

这样避免：

```text
用户删除
→ 晚到 checkpoint
→ 内容重新出现
```

---

## 5.16 Storage failure

数据库：

- locked；
- disk full；
- corrupt；
- migration failure；
- permission denied；

都不能阻止用户复制当前结果。

ExecutionStore 保留：

```text
bounded in-memory result
```

UI 明确：

```text
生成成功
但未保存
```

而不是：

```text
回答消失
```

---

## 5.17 Stage 对应

### Stage 2

必须完成：

- ExecutionAccumulator；
- UI coalescing；
- CheckpointProjector；
- SQLite worker；
- partial checkpoint；
- terminal immediate flush；
- sequence guard。

### Stage 5

补：

- history delete barrier；
- clear history retention generation；
- favorites relation；
- search / pagination；
- backup consistency。

---

## 5.18 验收

1. 1000 个 streaming delta 不产生 1000 次 SQL；
2. UI 不每 token 重建完整消息树；
3. terminal state 立即显示；
4. 终态 checkpoint 不被旧 checkpoint 覆盖；
5. 数据库慢时网络流不无限阻塞；
6. 数据库失败时结果仍可复制；
7. 删除后晚到写入不能复活内容；
8. Chat 正文不在 messages / executions 永久重复两份。

---

# 6. 类型化局部事件，不复制通用 EventBus

## 6.1 目的

AgentX 展示了事件化解耦的价值，但 LexWisp 不复制全局：

```text
AppEvent
EventHub
EventBus<T>
```

原因：

LexWisp 已经有非常明确的领域边界：

- Invocation
- Execution
- Plugin
- Surface
- Settings
- Storage

不需要所有东西通过一个“应用事件总线”互相广播。

---

## 6.2 总原则

优先级：

```text
直接函数调用
    ↓
强类型 Port
    ↓
领域状态订阅
    ↓
局部类型化事件
```

只有当：

- 一对多；
- 生命周期不同；
- 消费者不应互相依赖；

才使用事件。

---

## 6.3 禁止形式

不建立：

```rust
enum AppEvent {
    Execution(...),
    Plugin(...),
    Settings(...),
    Window(...),
    History(...),
    EverythingElse(...),
}
```

不建立：

```rust
EventBus<T>
subscribe<T>()
publish<T>()
```

作为全局基础设施。

不允许：

```text
模块 A
→ global bus
→ 模块 B
→ global bus
→ 模块 C
```

把清晰调用链变成事件迷宫。

---

## 6.4 推荐的局部事件

### Execution

```rust
enum ExecutionEvent {
    Started { ... },
    TextDelta { ... },
    Progress { ... },
    Completed { ... },
    Failed { ... },
    Cancelled { ... },
}
```

只存在于：

```text
InvocationSupervisor
ExecutionStore
Execution subscribers
```

### Plugin

```rust
enum PluginLifecycleEvent {
    Registered { ... },
    Activated { ... },
    Disabled { ... },
    Reloaded { ... },
    Faulted { ... },
}
```

消费者：

- Plugin UI；
- Action Registry projection；
- diagnostics。

### Settings

设置更适合：

```text
immutable snapshot + generation
```

而不是每个字段都发 Event。

必要时：

```rust
struct SettingsChanged {
    generation: u64,
    domains: SettingsDomainMask,
}
```

通知消费者重新读取 Snapshot。

### Surface

Surface 不需要全局 EventBus。

由：

```text
WindowRegistry
SurfaceFactory
HostUiCommandPort
```

直接处理。

---

## 6.5 Snapshot 优先于事件重放

对于：

- Execution；
- Settings；
- Plugin state；

订阅者必须能：

```text
read current snapshot
```

事件只是：

```text
something changed
```

的通知。

这样订阅者 lag 时可以：

```text
version mismatch
→ reload snapshot
```

而不是丢事件后永久错乱。

---

## 6.6 有界通道

跨线程：

```text
bounded channel
```

优先。

UI notification 可以 coalesce。

不可恢复的业务数据不能放进：

```text
best effort broadcast only
```

例如：

- 终态；
- 用户提交的消息；
- permission decision；
- delete barrier。

这些必须有权威状态或可靠命令路径。

---

## 6.7 Event ownership

每条事件流都必须回答：

```text
谁创建？
谁关闭？
谁取消？
谁保存？
谁处理 lag？
```

例如：

```text
ExecutionSubscription
owner = Surface / ChatController
drop = unsubscribe
source = ExecutionStore
recovery = get_snapshot(version)
```

而不是全局 subscription 永久存在。

---

## 6.8 GPUI 回调

后台事件到 GPUI：

```text
background
   │
   ▼
typed notification
   │
   ▼
UiCommand / projection
   │
   ▼
Entity.update()
```

事件中不携带：

- GPUI Entity；
- Window；
- Context；
- COM pointer；
- HWND ownership object。

---

## 6.9 Chat 的事件模型

ChatController 不需要“Chat EventBus”。

它可以直接：

```text
ChatController
   ├─ observe ExecutionStore
   └─ publish ChatSnapshot generation
```

UI：

```text
ChatView
   └─ subscribe ChatController snapshot
```

消息业务不需要：

```text
Chat → AppEvent → MessageService → AppEvent → UI
```

---

## 6.10 Stage 对应

### Stage 1

建立：

- Plugin lifecycle 的最小通知；
- Settings snapshot generation；
- WindowRegistry 直接命令。

### Stage 2

建立：

- Execution event / snapshot；
- ChatController projection。

### Stage 5+

按真实调用方再增加事件。

不要提前建立“未来统一事件平台”。

---

## 6.11 验收

1. 项目中没有通用全局 `AppEvent` 巨型 enum；
2. 没有动态类型 EventBus；
3. 关键业务链可以从代码直接追踪；
4. 每个 event stream 有明确 owner；
5. 事件 lag 后可以靠 Snapshot 恢复；
6. 终态不依赖 best-effort broadcast；
7. Surface 关闭后相关 subscription 被释放。

---

# 7. 五项扩展之间的组合关系

五项不是彼此独立的“小技巧”。

它们应组合成：

```text
                    ┌──────────────────┐
                    │  SurfaceFactory  │
                    └────────┬─────────┘
                             │
                             ▼
                     UI / Controller
                             │
               ┌─────────────┴─────────────┐
               │                           │
               ▼                           ▼
      ChatController                Other Action UI
               │                           │
               └─────────────┬─────────────┘
                             ▼
                       Host Handles
                             │
                             ▼
                 InvocationSupervisor
                             │
                             ▼
                    ExecutionStore
                             │
               ┌─────────────┴─────────────┐
               ▼                           ▼
        Typed Subscription          CheckpointProjector
               │                           │
               ▼                           ▼
           UI Snapshot                    SQLite
```

这形成一个非常明确的规则：

> **Window 不拥有业务，Controller 不拥有基础设施，Host Handle 不绕过权限，ExecutionStore 不等于数据库，事件不等于状态。**

---

# 8. 对原 Stage 0–4 的具体补充

## Stage 0 补充

在原任务基础上加入：

### 0-A Window Helper

实现：

```text
build_window_options()
LexWispWindowRoot
open_surface_window()
```

能够被后续 `SurfaceFactory` 接管。

### 0-B 禁止事项

Stage 0 不建立：

- 完整 Window Registry；
- 通用 Surface DSL；
- 全局 EventBus；
- Service Container；
- Chat Controller；
- 空 Provider。

### 0-C 验证

额外确认：

- Root overlay 正常；
- Window helper 可第二次创建窗口；
- Window close 后资源释放；
- helper 中无业务分支。

---

## Stage 1 补充

### 1-A HostHandles

Composition Root 创建真实服务后形成 HostHandles。

### 1-B SurfaceFactory

将 Stage 0 helper 收敛为：

```text
SurfaceFactory + WindowRegistry
```

### 1-C 类型化通知

只加入当前需要的：

- Plugin lifecycle；
- Settings generation；
- Window state。

### 1-D Task discipline

后台任务必须：

```text
HostTaskPort
→ TaskScope
→ owner
```

不允许复制 AgentX 式 fire-and-forget。

---

## Stage 2 补充

### 2-A ChatController

虽然 Stage 2 只有一个活动 Conversation，也使用稳定 ConversationId。

### 2-B Host Handles 扩展

加入：

- AIService；
- ProviderRegistry；
- ExecutionStore；
- Storage。

### 2-C ExecutionAccumulator

SSE semantic delta 统一进入 ExecutionStore。

### 2-D 双 Projection

```text
UI      ~33 ms
SQLite  ~500 ms / 16 KiB
```

### 2-E 局部订阅

ChatController 订阅 Execution，不通过全局 EventBus。

---

## Stage 3 补充

Stage 3 不增加新的基础设施。

Translate / Polish 必须直接复用：

- HostHandles 形成的受控端口；
- InvocationSupervisor；
- ExecutionStore；
- CheckpointProjector；
- Result Surface；
- 类型化 Execution subscription。

这也是检验 Stage 1–2 基础设计是否真正可复用的阶段。

如果 Translate / Polish 需要再复制：

```text
HTTP
streaming
stop
persistence
result state
```

说明 Stage 2 抽象失败，应修正现有路径，而不是再写一套。

---

## Stage 4 补充

Stage 4 的多会话扩展只允许新增：

- Conversation index；
- create / rename / switch / delete；
- Chat Panel View；
- Surface handoff；
- list virtualization。

不允许重写：

- send；
- stop；
- provider request；
- streaming；
- ExecutionStore；
- persistence checkpoint。

如果 Stage 4 必须重写这些，说明 ChatController 没有在 Stage 2 正确建立。

---

# 9. 建议的第一批代码骨架

这是推荐的**概念骨架**，不是要求一次生成所有文件。

```text
crates/
├── lexwisp-app/
│   └── composition_root.rs
│
├── lexwisp-core/
│   ├── ids.rs
│   ├── execution.rs
│   ├── plugin.rs
│   └── ports/
│       ├── ui.rs
│       └── chat.rs
│
├── lexwisp-host/
│   ├── handles.rs
│   ├── invocation/
│   │   ├── supervisor.rs
│   │   └── store.rs
│   ├── task/
│   │   └── scope.rs
│   └── projection/
│       └── checkpoint.rs
│
├── lexwisp-ui/
│   ├── surface/
│   │   ├── factory.rs
│   │   ├── registry.rs
│   │   └── root.rs
│   └── chat/
│       └── view_model.rs
│
├── lexwisp-storage/
│   └── worker.rs
│
└── lexwisp-plugins-builtin/
    └── chat/
        ├── controller.rs
        ├── conversation.rs
        └── context.rs
```

注意：

> 这不是要求 Stage 0 立即生成这些空文件。

仍遵循原 SPEC：

- 有真实调用方再拆；
- 每阶段只建立实际需要的文件；
- 不生成大批空 trait、空目录和 TODO scaffold。

---

# 10. 关键反模式清单

Coding Agent 实现以上五项时，发现以下形式应停下来检查。

## 10.1 Surface

错误：

```text
QuickShell::open_window()
ChatPanel::open_window()
Settings::open_window()
```

三处复制 Root / overlay / WindowOptions。

正确：

```text
SurfaceFactory
```

统一窗口基础机制。

---

## 10.2 Host

错误：

```rust
AppState::global(cx)
    .services
    .get(...)
```

后台任务依赖 GPUI global。

正确：

```text
clone required typed Host Handle
```

---

## 10.3 Runtime

错误：

```rust
if no runtime {
    create new tokio runtime
}
```

正确：

```text
唯一 Host Runtime
```

---

## 10.4 Chat

错误：

```text
QuickShellView
→ reqwest
→ update text
→ SQL
```

正确：

```text
QuickShellView
→ ChatController
→ ChatHostPort
→ InvocationSupervisor
→ ExecutionStore
```

---

## 10.5 Streaming

错误：

```text
token
→ UI render
→ SQL write
```

正确：

```text
delta
→ ExecutionAccumulator
→ UI coalesce
→ checkpoint coalesce
```

---

## 10.6 Event

错误：

```text
AppEvent::Everything
```

正确：

```text
领域级状态 + 局部强类型事件
```

---

## 10.7 Persistence

错误：

```text
收到旧 checkpoint
→ 覆盖新状态
```

正确：

```text
sequence / generation guard
```

---

# 11. 第一批实施顺序

建议 Coding Agent 在真正开始时按以下顺序执行。

## Batch A — Stage 0

1. 锁定 GPUI-Kit。
2. 建立最小 Release。
3. 建立 Window helper。
4. 建立统一 Root。
5. 验证输入、IME、selectable text。
6. 验证窗口关闭与重建。

## Batch B — Stage 1

1. Composition Root。
2. HostTaskPort / TaskScope。
3. 第一版 HostHandles。
4. SurfaceFactory。
5. WindowRegistry。
6. HostUiCommand bridge。
7. Hotkey / Tray / Single Instance。
8. Settings snapshot generation。

## Batch C — Stage 2

1. Storage worker。
2. Provider / Credential。
3. ExecutionStore。
4. InvocationSupervisor。
5. ExecutionAccumulator。
6. CheckpointProjector。
7. ChatController。
8. Quick Shell Chat。
9. Streaming。
10. Stop。
11. Partial persistence。
12. terminal flush。

## Batch D — Stage 3

直接复用已有能力完成：

1. Selection。
2. Action launch modes。
3. Translate。
4. Polish。
5. Replace。
6. dismiss policy。

Stage 3 不再创建第二套 AI / Streaming / Persistence 基础设施。

---

# 12. 最终补充验收表

| 条件 | 必须满足 |
|---|---|
| Window 创建 | 三个 Surface 共用 SurfaceFactory / Window helper |
| Root | 每个 Window 只有一个 Kit Root |
| 生命周期 | Window 销毁不等于业务状态销毁 |
| Host | 后台任务不依赖 GPUI Global Service lookup |
| Runtime | 只有一个 Host Tokio Runtime |
| Handle | HostHandles clone 不产生新资源 |
| Plugin | 插件拿 scoped PluginContext，不拿 HostHandles |
| Chat | Stage 2 就建立 ChatController |
| Conversation | Stage 2 就使用稳定 ConversationId |
| Handoff | Stage 4 小窗转大窗不重发请求 |
| Execution | ExecutionStore 是运行时唯一权威 |
| UI | Streaming UI 合并刷新 |
| SQLite | Streaming 使用批量 checkpoint |
| Terminal | 终态立即 flush |
| Sequence | 旧 checkpoint 不能覆盖新 checkpoint |
| Delete | 删除后晚到 checkpoint 不能复活记录 |
| Event | 不建立全局通用 EventBus |
| Recovery | 事件 lag 可以读取 Snapshot 恢复 |
| Storage failure | AI 结果仍可复制，明确显示未保存 |
| Reuse | Translate / Polish 复用 Stage 2 的 Invocation / Execution / Persistence 路径 |

---

# 13. 最终原则

本扩展的目标不是让 LexWisp 更“企业级”，而是避免第一批代码写出来之后，在 Stage 3–4 因为窗口、Chat、任务和 Streaming 所有权不清而整体返工。

最终应形成以下五条明确纪律：

1. **SurfaceFactory 管窗口，不管业务。**
2. **HostHandles 提供强类型共享能力，不成为 Service Locator。**
3. **ChatController 管 Chat 语义，不直接拥有 HTTP、SQL 或 Window。**
4. **ExecutionStore 管运行时真相，SQLite 通过批量 checkpoint 做持久化投影。**
5. **事件只在明确领域内使用，状态可以读取，事件不能代替状态。**

这五项应作为原 `LexWisp_IMPLEMENTATION_SPEC.md` 在 Stage 0–4 的实施补充，与原 SPEC 一并交给 Coding Agent。
