# LexWisp BUGFIX SPEC

**版本：1.0｜日期：2026-09-20｜目标代码：`wadaxiyang/LexWisp` `main`，基于 `f735d23`（v0.1.0）审查结果**

> 本文只处理当前实现中已经确认的逻辑、生命周期与性能隐患，不负责新的 UI 设计。  
> 若本文与旧 SPEC 中相同实现点冲突，以本文为准。  
> 修改时应遵守现有 `AGENTS.md`：不引入通用 EventBus，不创建额外 Tokio Runtime，不把阻塞 I/O 放到 GPUI 主线程，不为修复局部问题做无关重构。

---

## 1. 修复目标

本轮必须解决以下问题：

1. **Declarative / Script Action 的最新 Snapshot 可能丢失，终态可能无法送达 UI**
2. **`ExecutionStore` 的 terminal execution 不释放，常驻进程内存会随调用次数持续增长**
3. **全局快捷键路径先阻塞执行 UIA capture 再显示窗口，影响主入口响应速度，并存在 UI command 被静默丢弃的问题**
4. **Execution 在等待全局并发 semaphore 时已经被标记为 `Running`，状态语义不准确**
5. 为第 2 项补齐 **Favorite 对已落库但已从内存淘汰的 execution 的兼容路径**

本轮不处理：

- 新 UI 视觉设计
- Cowork 等新体验
- Translate / Polish 产品功能重做
- 通用插件 UI 框架
- 多 Provider 并发路由
- 大规模数据库重构

---

# 2. P0：修复 Text Action 最新 Snapshot 丢失

## 2.1 当前问题

Chat 已经实现“latest snapshot”语义：

- channel 有界
- channel 满时先弹出一个旧 snapshot
- 再尝试写入最新 snapshot
- 因而 terminal snapshot 不会被 streaming 中间状态永久挡住

但以下两个实现目前只保存 `Sender<TextActionSnapshot>`：

```text
crates/lexwisp-host/src/declarative.rs
crates/lexwisp-plugins-script/src/controller.rs
```

它们在 `TrySendError::Full(_)` 时直接保留 subscriber 并丢弃本次 snapshot。

在连续 streaming 时可能出现：

```text
Running snapshot
Running snapshot
Running snapshot
channel full
        ↓
Completed snapshot 被丢弃
        ↓
之后没有新 snapshot
        ↓
UI 永久停在 Generating / Stop 可用状态
```

这是逻辑正确性问题，不是单纯性能问题。

---

## 2.2 修改要求

### 2.2.1 DeclarativeController

将：

```rust
subscribers: Vec<Sender<TextActionSnapshot>>
```

改为等价于 Chat 当前做法的 subscriber 结构：

```rust
struct TextActionSubscriber {
    sender: Sender<TextActionSnapshot>,
    stale_receiver: Receiver<TextActionSnapshot>,
}
```

`subscribe(capacity)` 时：

1. 创建 bounded channel
2. 发送一次当前 snapshot
3. 保存：
   - sender
   - `receiver.clone()` 作为 stale receiver
4. 返回原 receiver

`publish()` 时：

```text
try_send(latest)
    │
    ├─ Ok
    │    保留 subscriber
    │
    ├─ Closed
    │    移除 subscriber
    │
    └─ Full
         stale_receiver.try_recv()
         再 try_send(latest)
         若 Closed 则移除
```

`set_surface_visible(true)` 中发送当前 snapshot 时必须走**同一 latest-snapshot helper**，不得重新写一套不同语义。

---

### 2.2.2 ScriptController

`ScriptController` 与 DeclarativeController 使用完全相同的 latest-snapshot 规则。

本轮允许两个模块各自保留一个很小的私有 helper，**不要为了这次修复创建全应用通用事件总线或复杂泛型发布系统**。

如果确实要抽取，只允许抽取纯粹的：

```text
bounded latest snapshot delivery
```

不得承载领域路由、订阅发现、动态 topic 等职责。

---

## 2.3 验收测试

至少增加以下测试：

### Case A：capacity = 1，终态必须到达

1. subscribe capacity 设为 1
2. 连续发布多个 Running snapshot，不消费
3. 最后发布 Completed snapshot
4. 消费 receiver
5. 最终获得的 snapshot 必须是 Completed

### Case B：Failed / Cancelled 同样不能丢

分别验证：

- `Failed`
- `Cancelled`

### Case C：Closed subscriber 被清理

receiver drop 后再次 publish，不得持续保存无效 sender。

### Case D：重新显示 Surface 后立即得到最新状态

在：

```text
visible = false
streaming continues
terminal reached
visible = true
```

之后 subscriber 必须能得到当前 terminal snapshot。

---

# 3. P0：限制 ExecutionStore terminal state 的内存增长

## 3.1 当前问题

当前：

```rust
ExecutionStore {
    entries: HashMap<InvocationId, ExecutionAccumulator>
}
```

execution 从 `start()` 插入以后，在 terminal commit 后仍然保留在 `entries`。

`ExecutionAccumulator` 持有：

- input
- 完整 output
- snapshot
- chat checkpoint
- observer
- 其他运行状态

因此 LexWisp 作为长期常驻 Windows 应用时，内存会随 invocation 数量单调增长。

`ExecutionStore` 的职责应是：

```text
Active Execution
+
有限数量的近期 Terminal Execution
```

而不是进程生命周期内保存所有 execution。

---

## 3.2 目标模型

保持当前单一 `ExecutionStore`，不要求本轮拆成多个 public service，但内部语义调整为：

```text
entries
├── Active
│   └── 永不被淘汰
└── Recent Terminal
    └── 有界保留
```

推荐初始上限：

```rust
const RECENT_TERMINAL_LIMIT: usize = 128;
```

可同时增加较宽松 TTL，例如 30–60 分钟；**上限是硬约束，TTL 是辅助约束**。

不要因为 TTL 再创建后台定时线程。清理采用 opportunistic pruning：

- 新 execution `start()` 前
- terminal commit 完成后
- favorite persistence 操作前后
- 其他自然进入 `ExecutionStore` 的低频点

---

## 3.3 数据结构修改

给 `ExecutionAccumulator` 增加：

```rust
finished_at: Option<Instant>
```

运行中：

```rust
finished_at = None
```

terminal persistence 完成、最终 `StorageState` 已确定后：

```rust
finished_at = Some(Instant::now())
```

增加私有方法，语义类似：

```rust
fn prune_terminal_entries_locked(
    entries: &mut HashMap<InvocationId, ExecutionAccumulator>,
    now: Instant,
    preserve: Option<&InvocationId>,
)
```

要求：

1. **绝不移除非 terminal entry**
2. 优先移除最老 terminal entry
3. 若存在 TTL，先清理超时 terminal
4. 再保证 terminal 数量不超过 `RECENT_TERMINAL_LIMIT`
5. 当前刚完成的 invocation 可通过 `preserve` 防止在同一次 commit 中立刻被淘汰
6. observer 不得因为 entry 淘汰再次收到伪造状态更新

---

## 3.4 `persist_for_favorite` 必须兼容已淘汰的 execution

直接淘汰以后，当前 `FavoriteService` 仍可能调用：

```rust
executions.persist_for_favorite(&invocation_id)
```

因此必须补 storage fallback。

### 3.4.1 ContentStore 新增 persisted-execution existence query

新增一个低成本查询，例如：

```rust
pub fn contains_execution(&self, invocation_id: &str) -> Result<bool, StorageError>
```

同时检查：

```text
executions
action_executions
```

语义是：

> 对该 invocation，持久层已经存在足以被 favorite 引用的 execution 记录。

不得只查 favorites 表。

---

### 3.4.2 `persist_for_favorite` 新流程

```text
查 ExecutionStore
    │
    ├─ 找到 active
    │    → 拒绝，要求先完成
    │
    ├─ 找到 terminal 且 StorageState::Saved
    │    → Ok，无需重复 checkpoint
    │
    ├─ 找到 terminal 且 Unsaved / NotRecorded
    │    → 按当前 checkpoint 逻辑尝试持久化
    │
    └─ 内存中不存在
         ↓
      ContentStore.contains_execution()
         │
         ├─ true  → Ok
         └─ false → 返回明确错误：
                    result is no longer retained and was not persisted
```

这样允许：

- 已正常落库的 execution 从内存释放后仍可 favorite
- recording disabled 下的未记录结果仍只在 recent terminal cache 中短期可 favorite
- 内存不会因为“以后可能收藏”而无限增长

不要为了保留 recording-disabled 的无限期收藏能力而继续无界持有所有输出。

---

## 3.5 验收测试

### Case A：大量完成请求后 entries 有界

构造至少 500 个 terminal execution：

```text
active = 0
terminal <= RECENT_TERMINAL_LIMIT
```

### Case B：active invocation 不被清理

即使 recent terminal 已超过 limit，也不得清理 active。

### Case C：已落库但已淘汰的 execution 可以 favorite

1. 完成并持久化 execution
2. 强制 prune
3. 确认内存无该 invocation
4. `persist_for_favorite` 返回 Ok
5. favorite 正常写入

### Case D：未记录且已淘汰结果给出明确错误

不得 panic，不得静默 favorite 一个不存在的 execution。

---

# 4. P0：重构 Hotkey 为 Surface-First、Context-Async

## 4.1 当前问题

当前 `WM_HOTKEY` 路径大致为：

```text
WM_HOTKEY
    ↓
capture_foreground_blocking()
    ↓
UIA worker
    ↓
最长等待约 800 ms
    ↓
ToggleQuickShell(snapshot)
    ↓
GPUI 显示窗口
```

这会把主入口的首帧延迟绑定到 UI Automation。

新的主入口必须遵守：

> **快捷键触发后先让主 Surface 出现；selection / foreground context 在后台完成后再投影到 UI。**

但不能简单地“先显示窗口，再调用现有 `capture()`”，因为一旦 LexWisp 获得焦点，`GetForegroundWindow` / focused element 已经变成 LexWisp 自己。

因此必须把：

```text
捕获 foreground target
```

与：

```text
执行 UIA capture
```

拆开。

前者很轻量，必须在窗口激活前完成；后者异步执行。

---

## 4.2 推荐数据流

```text
WM_HOTKEY
   │
   ├─ 1. 立即抓取轻量 ForegroundCaptureSeed
   │     - HWND
   │     - process id
   │     - title
   │
   ├─ 2. 分配 launch_generation
   │
   ├─ 3. 发送 ToggleMainSurface / ToggleQuickShell
   │     UI 立即显示
   │
   └─ 4. 将 seed 投递给 UIA worker
             │
             ▼
         ContextSnapshot
             │
             ▼
      HostUiCommand::ApplyLaunchContext {
          launch_generation,
          snapshot
      }
             │
             ▼
      GPUI 仅在 generation 仍有效时应用
```

---

## 4.3 ContextService 修改

不要暴露 Windows HWND 到 `lexwisp-core`。

在 `lexwisp-platform-windows` 内部增加私有或 platform-local：

```rust
struct ForegroundCaptureSeed {
    ...
}
```

增加一个入口，语义类似：

```rust
fn prepare_foreground_capture(&self) -> PreparedCapture
```

或：

```rust
fn capture_for_launch_async(
    &self,
    launch_generation: u64,
    sink: ...
)
```

要求：

1. foreground target 在 Hotkey callback 中立即获取
2. UIA 仍只在既有 MTA worker 上执行
3. 不为每次快捷键新建 Tokio Runtime
4. 不把 COM object 移出 UIA worker
5. 最终 snapshot 必须带对应 `launch_generation`

具体命名可按现有代码风格调整。

---

## 4.4 防止旧 capture 覆盖新 launch

必须处理：

```text
Hotkey #1
  capture 很慢

Hotkey #2
  capture 很快
  先返回

Hotkey #1
  后返回
```

因此 `SurfaceController` / Main Surface 状态中保存：

```rust
current_launch_generation: u64
```

收到：

```rust
ApplyLaunchContext { launch_generation, snapshot }
```

时：

```text
generation != current_launch_generation
→ 丢弃
```

不得让旧窗口上下文覆盖新一次唤起。

---

# 5. P0：Hotkey / 主 Surface UI Intent 不得静默丢弃

## 5.1 当前问题

Windows Shell 中多处使用：

```rust
ui_commands.try_send(...)
```

且忽略返回值。

bounded channel 满时，一次用户快捷键或 tray 点击可能直接消失。

对低优先级刷新事件可以容忍 coalescing，但主 Surface intent 不应静默丢弃。

---

## 5.2 修改要求

为用户可见的高优先级 Surface intent 建立**有界、最新意图语义**。

推荐建立一个很小的 command port，例如：

```text
SurfaceIntentPort
```

内部可采用与 Chat latest snapshot 类似的：

```text
capacity = 1
Full → evict stale intent → send newest intent
```

允许连续快速 Hotkey 被合并，但最后一次用户意图必须留下。

至少覆盖：

- Hotkey Toggle
- Tray Open Main Surface
- Open Workspace / Expand
- Show Settings 可继续走普通 command channel

不要使用无界 channel。

不要在 Win32 message thread 上为了等 UI 主线程而无限 `send_blocking()`。

---

# 6. P1：修正 Queued / Running 状态语义

## 6.1 当前问题

`InvocationSupervisor` 中 execution 在取得 concurrency permit 前已经：

```text
ExecutionStore::start(...)
status = Running
```

随后才：

```rust
self.concurrency.acquire_owned().await
```

因此实际排队等待 semaphore 的请求被错误显示为 Running。

---

## 6.2 修改要求

`ExecutionStore::start()` 初始状态改为：

```rust
ExecutionStatus::Queued
```

新增：

```rust
mark_running(invocation_id)
```

取得 semaphore permit 后、真正调用 Provider 之前：

```text
Queued
  ↓ permit acquired
Running
  ↓
Provider request / stream
```

`mark_running` 要：

- 拒绝 terminal entry
- sequence + 1
- 发送 observer snapshot
- 必要时按当前 checkpoint 策略持久化，但不得为每次简单状态变化引入额外高频写盘

---

## 6.3 验收测试

在 concurrency limit 可控的测试中：

1. 第一个 invocation 占住 permit
2. 第二个 invocation 开始
3. 第二个状态必须为 `Queued`
4. 第一个释放 permit
5. 第二个变为 `Running`
6. 再进入 terminal

---

# 7. 性能约束

本轮修复不得引入以下退化：

- GPUI 主线程执行 SQLite / Win32 UIA / Credential I/O
- streaming 每个 token 新建任务
- 每个 plugin 新建 Tokio Runtime
- terminal cleanup 后仍保留 observer 的额外强引用
- 为 Context capture 每次启动新的长期线程
- 将 bounded queue 改成 unbounded queue

现有以下策略保留：

```text
UI flush       ≈ 33 ms
checkpoint     ≈ 500 ms 或 16 KiB
SQLite         独立 worker
Script         独立 worker
Host           单一 Tokio Runtime
```

---

# 8. 建议修改文件

核心预计涉及：

```text
crates/lexwisp-host/src/declarative.rs
crates/lexwisp-plugins-script/src/controller.rs

crates/lexwisp-host/src/execution.rs
crates/lexwisp-host/src/favorites.rs
crates/lexwisp-storage/src/content.rs

crates/lexwisp-platform-windows/src/context.rs
crates/lexwisp-platform-windows/src/shell.rs
crates/lexwisp-core/src/command.rs
crates/lexwisp-app/src/main.rs
crates/lexwisp-ui/src/surface.rs

crates/lexwisp-host/src/invocation.rs
```

如果实际职责允许，可少改文件；不要为了和列表一致制造无意义改动。

---

# 9. 实施顺序

```text
Step 1
TextAction latest snapshot 修复
        ↓
Step 2
ExecutionStore terminal retention + favorite fallback
        ↓
Step 3
Queued / Running 语义修复
        ↓
Step 4
Hotkey surface-first + capture generation
        ↓
Step 5
Surface intent coalescing
        ↓
Step 6
cargo fmt / clippy / tests / Windows Release smoke test
```

---

# 10. 最终验收

本 SPEC 完成后必须满足：

- Declarative / Script terminal 状态不会因为 bounded subscriber 满而永久丢失
- 长时间运行后 `ExecutionStore` 不随历史 invocation 无界增长
- 已落库 execution 在内存淘汰后仍可 favorite
- Hotkey 不再等待完整 UIA capture 才显示主 Surface
- 旧 context capture 不会覆盖新一次 Hotkey
- 高优先级 Hotkey intent 不会因 `try_send` Full 被静默丢弃
- 排队请求显示 `Queued`，真正开始 Provider 调用后才显示 `Running`
- 不改变现有单 Host Runtime、SQLite worker、Script worker、插件权限与持久化基本架构
