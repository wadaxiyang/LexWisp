# LexWisp 工程 SPEC

**文档版本：1.0**  
**研究核查日期：2026-09-16**  
**目标产物：可发布的 Windows 桌面应用 `LexWisp.exe`，以及按需运行的 `LexWisp.Updater.exe`**  
**实施方式：Coding Agent 按 Stage 0 → Stage 13 顺序实施；每阶段必须留下代码、测试和验收证据。**

> **事实与设计的界限**：本文对框架版本、接口行为和平台限制的判断来自文末的一手资料，正文以 `[Sxx]` 标注。架构、默认值、资源限制与性能数字是本项目的明确设计要求，不是已经完成的实现或实测结果。本文编写过程中未在 Windows 上构建或测量 LexWisp；必须在 Stage 0 建立真实基线，后续阶段持续验证。不得把文档中的预算写成产品已经达到的性能。

## 执行总则

**产品定位**：面向 Windows 的轻量划词翻译与文本查询工具。用户主动触发，应用只读取本次选中文字，将其发送到用户明确配置的 Provider，在鼠标附近展示可选择、可复制的结果，并支持历史与收藏。默认不持续监听剪贴板内容，不后台扫描屏幕，不自动上传文档。

**固定架构**：主程序是一个进程，GPUI、业务、网络、数据存储、原生 Windows 能力均在进程内组织。Updater 仅在安装更新或恢复更新时启动，完成后退出。不得自行改变为 UI/Daemon 双进程、WebView 主界面、Windows 服务、浏览器扩展或内嵌脚本运行时。

**固定 UI 栈**：Rust + GPUI + longbridge/gpui-kit。优先使用 Kit 的组件、主题、文本选择、列表、输入、对话框与测试设施。只允许开发 LexWisp 业务视图及窄范围平台适配，不开发另一套通用组件库。

**规范强度**：文中的“必须”是阶段门禁；“默认”是新安装配置；“目标”是优化方向；“发布门槛”是必须提供实测证据的上限。发现约束不可同时成立时，先给出最小复现、测量和局部修复，不得隐瞒问题或未经记录放宽要求。

**阶段纪律**：每阶段先读取此前代码、ADR 和测试报告，完成本阶段任务，再运行相关回归。禁止以空实现、永久 Mock、关闭错误处理、禁用安全验证或未执行的测试报告宣布完成。所有修改必须可通过版本控制审阅。跨阶段的局部修复可以提前做，但不得跳过前置门禁。

**范围边界**：首发支持 Windows 11 x64，Windows 10 22H2 x64 作为经过实测后发布的兼容目标；这是应用兼容范围，不代表替操作系统提供安全维护。ARM64、OCR、离线大模型、账号云同步、插件市场、TTS、悬浮球、全文文档翻译、鼠标自动划词触发、多结果固定窗口、企业集中部署均不进入 v1。保留扩展接口，不预先实现这些系统。

### 阶段总览

| 阶段 | 必须交付的增量 | 关键门禁 |
|---|---|---|
| Stage 0 | 框架核验、架构决策、Windows 最小探针、性能基线 | Release 能构建；零 GPUI 窗口仍能驻留；窗口可真正销毁 |
| Stage 1 | Workspace、应用装配、运行时、原生宿主、托盘与退出 | 一个主进程；线程和资源有所有者；无空闲轮询 |
| Stage 2 | 数据库、配置、Repository、凭据、迁移 | 数据与密钥分离；写入可恢复；UI 不阻塞 |
| Stage 3 | 快捷键、UIA 取词、受控剪贴板回退 | 不错取旧剪贴板；不抢先改变来源焦点 |
| Stage 4 | Provider、流式协议、请求状态、取消 | 最新请求生效；旧流绝不污染新结果 |
| Stage 5 | Popup 业务 UI 与完整生命周期 | 首绘、选择复制、默认 30 秒隐藏后销毁 |
| Stage 6 | 主窗口、手动查询、历史与收藏 | 分页与虚拟列表；收藏独立于历史 |
| Stage 7 | 完整设置、主题、启动项与配置联动 | 所有用户配置可在设置页修改并持久化 |
| Stage 8 | 错误体系、诊断、隐私、故障降级 | 不泄露正文与密钥；故障不破坏已保存数据 |
| Stage 9 | 更新发现、验签、下载、预检 | 未经授权的载荷不得进入可安装状态 |
| Stage 10 | 独立 Updater、替换、恢复、健康握手 | 中断可恢复；二进制与数据库回退一致 |
| Stage 11 | 系统测试、性能优化、长期运行验证 | Windows 真机、故障注入、资源趋势全部合格 |
| Stage 12 | Release 构建、安装包、签名、CI 与文档 | 干净机器可安装；构建与分发链可审计 |
| Stage 13 | 发布候选全流程验收 | 功能、安全、升级、性能证据齐全 |

---

## Stage 0　研究核验、架构冻结与 Windows 可行性探针

### 0.1 目标

将框架的实际能力与产品约束对齐，先消除可能导致后续架构返工的问题。交付可构建的 Windows 探针、依赖锁定、ADR、初始性能报告，而不是只有文字调研。

### 0.2 涉及模块

`docs/research/`、`docs/adr/`、`tools/probes/`、Workspace 根配置、`xtask` 初始骨架、CI 的 Windows 构建任务。

### 0.3 实现任务

#### 0.3.1 已核实的框架基线

| 项目 | 本次核查所得事实 | 对 LexWisp 的约束 |
|---|---|---|
| Kit 稳定版 | 官方最新稳定发布为 **v0.6.1，2026-09-09**；v0.6.0 完成 GPUI Kit 命名与统一入口调整 [S01] | 使用 `gpui-kit`，不照搬旧版 GPUI Component 教程 |
| 发布标签依赖 | v0.6.1 根清单对 `gpui-pre` 家族声明 `0.3.1` 兼容范围，而不是精确固定 0.3.1 [S02] | 清单版本与实际解析版本分别记录 |
| 当前 GPUI 快照 | 本次核查的 `gpui-pre-platform` 最新版本为 **0.3.5，2026-09-14**；平台子包使用一致的精确家族版本 [S03] | 首选冻结 0.3.5 家族，提交 `Cargo.lock` 并检查重复 GPUI 类型 |
| 统一入口 | Kit 门面提供 GPUI、base、component、assets；`component` 和 `assets` 是默认功能 [S04] | 业务统一经门面使用；测试支持仅在测试图启用 |
| 窗口根节点 | 官方入门要求窗口使用 `Root`；覆盖层须在内容中按需渲染 [S05] | 每个真实窗口独立 Root、焦点域和覆盖层 |
| 文本与输入 | 新版区分单行 Input、多行 Textarea；已有 TextView、SelectableText 与文本选择能力 [S01][S06] | 不自制选中文字引擎，不把重型 Editor 用于普通输入 |
| Windows 渲染 | 本次锁定快照的 Windows 源码是 DirectX/Direct3D 11 设备路径，文字路径包含 DirectWrite [S07] | 不根据旧资料直接宣称本项目必须 Vulkan；GPU 支持以该版本实测为准 |
| 零窗口驻留 | GPUI 存在 `with_quit_mode(QuitMode::Explicit)`，默认非 macOS 行为可能在最后窗口关闭时退出 [S08] | 必须显式设置 Explicit，不靠隐藏 GPUI 主窗维持进程 |
| 应用隐藏 | 本次快照 Windows `Platform::hide` 是空实现 [S09] | 不使用应用级 `hide()` 作为 Popup 已隐藏的证据 |
| 窗口激活 | 本次 Windows 窗口 `activate` 路径含模拟 Alt 输入 [S10] | LexWisp 避开该激活路径，使用经过测试的原生焦点适配 |
| 测试 | Kit 0.6.1 提供无头 UI 测试；像素/平台路径与无头交互并不等价 [S11] | 无头测试不能替代 Win32、DPI、GPU 和安装包测试 |

**版本采用方案**：`gpui-kit = "=0.6.1"`；通过锁文件冻结经验证的 `gpui-pre` 0.3.5 家族。此组合是依据已发布的兼容约束和当前源码选择的实施基线，不宣称本文已将其编译通过。Stage 0 必须验证 Debug、Release 和测试三张依赖图。

仅在当前基线无法通过必要能力探针时，允许采用**最小、固定 revision 的补丁**，并保留 patch 文件、上游问题链接、许可证和回归测试。不得引用浮动 `main`；不得退回其他 UI 栈。Rust 工具链必须选择满足完整依赖图的已发布 stable 精确版本，写入 `rust-toolchain.toml`；不把官方安装页的最低提示当成全部传递依赖的 MSRV 证明。[S12]

#### 0.3.2 官方组件与样例复用矩阵

| 产品部位 | 首选 Kit 能力 | 限制与验证 |
|---|---|---|
| Popup / 主窗口 | Root、窗口装饰、主题 tokens | 参考 `root_borderless`，不照搬固定示例尺寸 [S13] |
| 查询输入 | Textarea / TextareaState | 处理中文输入法、换行、提交快捷键 |
| API 地址、搜索框 | Input / InputState | 状态在初始化时创建，不在 render 中重建 |
| 结果文本 | TextViewState + TextView，必要时 SelectableText | 参考 `text_selection` 与 `stream-markdown`；必须实现选择与复制 [S06][S14] |
| Provider 选择 | Select / 可搜索选择组件 | 按锁定版签名实施，不发明不存在的属性 |
| 设置表单 | Button、Switch、Checkbox、表单布局、Tooltip | 必须有禁用态、错误信息和键盘焦点 |
| 导航 | Sidebar、Tabs 或等效已有导航 | 一个主窗口，不为每个设置页新建 HWND |
| 历史、收藏 | List、虚拟列表；必要时 DataTable | 0.6 系列的 Table 与旧数据表命名不同，避免套用旧 API |
| 确认与提示 | Dialog、Sheet、Notification | 覆盖层焦点正确，不侵入其他窗口 |
| 托盘菜单 | Win32 原生通知区与菜单 | 属于操作系统集成，不另造 UI 组件库 |

必须阅读并运行锁定标签的最小应用、无边框窗口、文本选择、流式 Markdown 样例以及组件故事中的实际用法。样例的无界 channel、固定 sleep、演示自动滚动、story 初始化等不是生产方案；只复用组件调用与经验证的生命周期方法。[S13][S14]

#### 0.3.3 关键架构取舍与采用方案

| 决策 | 备选方案 | 明确采用 | 理由 |
|---|---|---|---|
| 进程组织 | 多进程 UI/后台；单进程 | **单进程主程序 + 临时 Updater** | 满足固定架构，降低协议和部署复杂度 |
| Popup 保留 | 每次销毁；永久隐藏；定时释放 | **懒创建，隐藏后默认 30 秒销毁** | 在短时间重复查询延迟与长期后台占用之间折中 |
| 主窗口 | 永久隐藏；关闭销毁 | **关闭即销毁** | 设置和管理页不是高频弹出路径 |
| HTTP 执行 | UI 线程；每模块 runtime；统一 runtime | **单个专用 Tokio current-thread runtime** | 网络与 UI 解耦，避免自建多套工作池 |
| 数据库 | JSON 文件；sqlx 池；rusqlite actor | **rusqlite + 单连接数据库线程** | 本地单用户、低并发写入，无连接池必要 |
| 设置 | TOML 与 DB 分散；统一存储 | **SQLite 中版本化配置** | Provider、默认项、快捷键配置可事务提交 |
| 密钥 | 明文配置；自制加密；系统凭据 | **Windows Credential Manager** | 密钥不进入 SQLite、日志、导出配置 |
| 取词 | 模拟复制优先；UIA 优先 | **UIA 优先，显式许可的保守剪贴板回退** | 减少剪贴板副作用，保留通用兼容路径 |
| Provider 集成 | 全功能 SDK；直接 HTTP adapter | **小型协议适配器 + 共用传输层** | 控制依赖、错误和取消语义 |
| 流式显示 | token 到达即全窗刷新；合并 | **有界缓冲 + 最大约 30 Hz 局部刷新** | 避免渲染过载，不牺牲可见响应 |
| 搜索 | 启动加载全历史；数据库检索 | **数据库查询 + 游标分页 + UI 虚拟化** | 启动与内存不随历史线性增长 |
| 更新 | 原地直接覆盖；带日志替换 | **签名载荷 + staging + 事务日志 + 备份恢复** | Windows 可执行文件占用与中途失败均可处理 |

#### 0.3.4 架构与依赖方向

```text
LexWisp.exe
  ├─ app bootstrap / AppServices / QueryCoordinator
  ├─ GPUI main thread
  │    ├─ PopupController → PopupView
  │    ├─ MainWindowController → pages / setting drafts
  │    └─ NativeHost HWND → tray / hotkey / OS events
  ├─ network thread → one Tokio runtime → Provider adapters
  ├─ database thread → Repository → one SQLite connection
  └─ lazy UIA MTA thread → current-selection requests

LexWisp.Updater.exe       only during apply / recovery
  └─ update protocol / signature / journal / file replacement / health handshake

Dependency direction
  core ← providers, storage, platform-win, update-core
  core + service handles ← ui
  app → all application modules
  updater → core + update-core + narrow platform functions
  updater must not depend on ui / GPUI / provider clients
```

这里的箭头表示右侧使用左侧；禁止 `core` 引用 GPUI、Win32 或 SQLite。业务类之间优先以少量明确 Trait 分隔可替换边界，不建立通用依赖注入容器、全局反射事件总线或每个服务一个 crate。

#### 0.3.5 必须实施的探针

1. **零窗口生命周期**：Explicit quit mode；启动后不创建 GPUI 业务窗口；注册原生宿主、热键与托盘；创建并移除最后一个 GPUI 窗口后仍可由热键重建；显式退出才退出进程。
2. **真实隐藏与销毁**：通过受控 HWND 适配隐藏/显示，通过 GPUI 窗口移除接口销毁。验证窗口句柄、事件订阅、任务、纹理引用和业务 Entity 的释放；不从外部直接 `DestroyWindow` 破坏 GPUI 所有权。
3. **焦点与选区**：非激活显示、鼠标点击后正常激活、程序主动激活被 Windows 拒绝时正常降级；不得模拟 Alt 来抢焦点。
4. **文本选择**：中英混排、换行、跨段选择、Ctrl+C、正在流式接收时选择、点击工具栏不破坏已选文本。若安全 Markdown 渲染边界无法立即验证，先发布纯文本结果展示，不以自制渲染器补洞。
5. **DPI / GPU**：100%、150%、200% 缩放与双显示器；负坐标；Windows 11 集显和独显各至少一种；远程桌面/驱动异常作为降级测试，不先承诺全部支持。
6. **构建与测试**：锁定版本的 Release 产物能运行，无头测试能调用真实业务视图；依赖 features 没有把测试、inspector、profiler、WebView 或全部 tree-sitter 语言带入生产图。
7. **资源基线**：记录新启动、创建窗口、关闭窗口并超过 TTL、重复 100 次后的进程 Private Bytes、Working Set、GPU 内存、线程、句柄与 CPU；分别列出框架基线与业务增量。

若 GPUI 初始化本身建立全局渲染设备、字体缓存或线程，**不把“销毁所有窗口”解释为“释放整个 GPUI”**。不得在后台反复重建 Application 来伪装轻量，也不得用系统裁剪 Working Set 代替真实释放。

### 0.4 验收标准

所有探针有实际 Windows 结果；核心版本和来源完整；零 GPUI 窗口驻留、真实销毁、文本选择、Release 构建四项不可缺失。Stage 11 的内存预算在这里先做可行性筛查；框架基础驻留已超过发布门槛时，本阶段不得宣布架构性能成立。

### 0.5 测试方法

运行 `cargo metadata --locked`、`cargo tree -d`、`cargo tree -e features`；分别构建 release 与 tests；记录 `rustc -Vv`。使用 Windows 进程计数器、Spy++ 或等效句柄检查、截图与短录像验证探针。测试报告必须附机器 OS build、GPU/驱动、缩放、构建 hash 和采样时段。

### 0.6 完成定义

提交 `docs/research/framework-baseline.md`、`docs/research/windows-probes.md`、`docs/performance/baseline.json`、至少六份 ADR，以及可重复运行的探针。ADR 至少覆盖依赖基线、窗口生命周期、线程模型、取词隐私、存储凭据与更新恢复。不得仅以“框架支持 Windows”代替验证。

---

## Stage 1　工程骨架、状态所有权与应用生命周期

### 1.1 目标

建立可长期驻留的应用基础，使托盘、窗口、线程、任务和退出各有明确所有者。业务功能可暂未齐全，但不能依靠常驻隐藏设置窗口维持进程。

### 1.2 涉及模块

`apps/lexwisp`、`crates/core`、`crates/platform-win`、`crates/ui`、`xtask`。

### 1.3 实现任务

#### 1.3.1 项目目录

```text
LexWisp/
├─ Cargo.toml / Cargo.lock / rust-toolchain.toml
├─ .cargo/config.toml
├─ apps/
│  ├─ lexwisp/src/
│  │  ├─ main.rs / bootstrap.rs / shutdown.rs
│  │  ├─ services.rs / query_coordinator.rs / runtime.rs
│  │  ├─ config_service.rs / update_service.rs / diagnostics.rs
│  │  └─ command_router.rs
│  └─ updater/src/
│     ├─ main.rs / apply.rs / recover.rs / health.rs
├─ crates/
│  ├─ core/src/           # domain DTOs, traits, reducers, validation, errors
│  ├─ providers/src/      # transport, openai_chat, deepl, sse, mock(test)
│  ├─ storage/src/        # actor, repositories, migrations, export, backup
│  ├─ platform-win/src/   # host, tray, hotkey, selection, clipboard, dpi,
│  │                     # credential, startup, instance, window_bridge
│  ├─ ui/src/             # app_model, popup, main_window, pages, actions,
│  │                     # business_views, kit_bootstrap, theme
│  └─ update-core/src/    # signed manifest, journal, paths, hashes, protocol
├─ xtask/src/             # test, bench, package, release verification
├─ tests/
│  ├─ fixtures/ / integration/ / ui/ / windows-e2e/ / update-faults/
│  └─ contracts/
├─ packaging/
│  ├─ windows/app.manifest / app.rc / installer.iss
│  ├─ release-policy.toml / update-schema.json
│  └─ assets/             # original app icon and embedded static assets
├─ tools/probes/ / tools/benchmark/
├─ docs/adr/ / docs/research/ / docs/performance/ / docs/stages/
├─ docs/user/ / docs/release/ / THIRD_PARTY_NOTICES.md
└─ .github/workflows/
```

可在同一 crate 中增加业务 module，不为每个页面、Repository 或错误类型建 crate。`platform-win` 通过 feature 分离桌面能力与 updater 所需窄能力，防止 helper 拉入 UIA、GPUI 或不必要网络。

#### 1.3.2 状态的唯一所有者

| 状态 | 唯一写入者 | 读取/传播方式 | 生命周期 |
|---|---|---|---|
| 已提交 Settings、Provider 配置 | ConfigService 经 Repository 提交 | 不可变 snapshot + revision 事件 | 进程级，持久化 |
| 页面编辑草稿 | 对应页面 ViewModel | 本页 Entity | 页面打开至保存/丢弃 |
| 活动查询与请求代次 | QueryCoordinator | 类型化 QueryEvent | 按 Surface |
| Popup 显示/隐藏/销毁 | PopupController | WindowGeneration + 句柄 | 进程级控制器；视图短生命周期 |
| 主窗口导航和草稿 | MainWindowController / 页面 | 页级 Entity | 主窗口生命周期 |
| 历史与收藏 | SQLite Repository | 游标分页与变更通知 | 持久化 |
| 取词原生资源 | NativeHost / UIA worker | 小型 Send DTO | 本次事务或进程级宿主 |
| 更新事务 | UpdateService → Updater | 明确状态和持久化 journal | 一次更新事务 |

`AppModel` 只组合小型状态与服务句柄，不包含所有历史正文、所有页面实体、所有 HTTP client 或大量可变字段。禁止把整个应用塞入 `Arc<Mutex<AppState>>` 后从任意线程改写；不得持有锁跨越 await、SQL、HTTP 或 UI 调用。

#### 1.3.3 执行器与任务边界

- **UI 主线程**：GPUI、Entity 更新、Root、Window、NativeHost HWND 及其 WndProc。WndProc 只完成必要 Win32 处理与消息投递，不做数据库、HTTP、磁盘大读写或 UIA COM 查询。
- **网络线程**：一个命名线程运行 Tokio `new_current_thread()`，启用 I/O 与 timer；所有 Provider 请求共享它。限制 blocking pool 上限为 2；不可在此 runtime 内运行同步 UIA 或长时间 CPU 解析。Tokio 的池和 GPUI 自身线程均计入测量，不能声称应用固定只有四个线程。[S15]
- **数据库线程**：一个命名线程拥有唯一 SQLite connection，接收有界命令。网络和 UI 通过异步结果接收器读取，不阻塞 UI。
- **UIA 工作线程**：首次需要时创建一个 MTA 线程，拥有 COM 对象，不创建窗口。线程可能长期保留；取词完成不反复重建 COM。[S16]
- **事件桥接**：优先用 GPUI 异步上下文等待有界 receiver，更新时回到 UI 上下文。不得用每 10/50/100 ms 定时器检查 channel；没有消息就睡眠。

进程级 TaskRegistry 管理长期任务的取消句柄与退出等待；窗口级 TaskScope 管理窗口短任务、订阅和 timer。回调使用弱引用，避免 `Entity → Task → Entity` 强引用环。禁止为了“让请求跑完”随意 detach 窗口请求。[S17]

队列设计：UI 命令队列容量 64，数据库命令队列容量 128；取词 pending 容量 1、后来请求替换尚未执行请求；流式正文队列在 Stage 4 单独限制。更新进度和窗口移动可合并；退出、失败、完成、事务提交结果不可静默丢弃。UI `try_send` 满时显示可恢复 Busy，不在 UI 上等待空位。

#### 1.3.4 启动、单实例与原生宿主

启动顺序固定为：参数解析与更新恢复检查 → 用户/安装目录确定 → 单实例检查 → 最小日志 → 数据层 → runtime/services → GPUI 初始化及 Explicit quit → NativeHost/托盘/热键 → 必要时显示首次配置页。

使用当前用户与安装标识范围内的命名 mutex，阻止同一数据目录的两个主实例。第二次普通启动通过固定命令通知现有实例显示主窗口并退出；使用受限的 registered message / named event，不接受任意脚本、文件执行或外部查询正文。命令只能是预定义 enum，跨完整性级别失败时提示已运行，不自动提权。

NativeHost 使用**不可见的普通顶层 HWND**，不是 GPUI 业务窗口，不创建渲染器。其目的是接收 `WM_HOTKEY`、通知区回调、`TaskbarCreated`、显示器/主题变化及会话消息；单纯 message-only HWND 不能被默认视为所有系统广播的等价接收者。注册与销毁都在所属 UI 线程完成。

托盘实现采用 `Shell_NotifyIcon`、稳定 GUID 与 `NOTIFYICON_VERSION_4`。左键显示主窗口，右键菜单包含“手动查询”“历史记录”“收藏”“设置”“暂停快捷键”“检查更新”“退出”。“暂停”只暂停新的全局触发，手动查询仍可用。Explorer 重启后重新注册图标，注销时移除图标，资源由 RAII 持有。[S18]

首次启动显示引导页；配置完成后的 `--background` 启动不打开任何 GPUI 业务窗口。普通再次启动显示已有主窗口或创建它。托盘添加失败时不得进入完全不可见状态，应显示主窗口和错误；GPU 初始化失败时使用原生错误对话框说明无法启动 UI，并提供日志位置。

#### 1.3.5 退出协议

`Running → Quiescing → Flushing → Exiting`。先拒绝新请求、注销热键、取消查询和取词，再结束窗口任务，提交已确认的收藏/配置写入，关闭 DB，移除托盘，关闭宿主，最终调用 GPUI 显式退出。

普通退出等待关键写入最多 3 秒；超时不谎报“已保存”，给出受控失败提示或保留可恢复数据库状态。更新退出必须使用更严格的“写入清空且 DB 已关闭”确认，失败则取消安装，不由 updater 强杀原有主程序。系统关机通知不弹出阻塞对话框，不尝试在最后一刻完成网络请求。

UIA 外部 COM 卡死不能让退出无限等待；不调用 `TerminateThread`。终止进程时由操作系统结束剩余线程，前提是自身数据库与关键文件已正常处理；这不代表任意第三方 COM 调用具备可取消性。

### 1.4 验收标准

后台仅一个 `LexWisp.exe`，Updater 未启动；无 GPUI 窗口时热键和托盘仍工作；重复打开/关闭主窗口 100 次不累积窗口、订阅或线程；正常退出没有残留图标、子进程或锁定数据库。

### 1.5 测试方法

单实例并发启动测试、Explorer 重启测试、退出中队列拥塞测试、UIA 不返回测试、无窗口驻留 10 分钟采样。对纯状态 reducer 做随机事件序列测试；使用 Kit 测试验证最后窗口关闭不被业务误处理成退出。

### 1.6 完成定义

工程可从干净 checkout 构建；`cargo xtask` 别名已配置；有可运行托盘壳、真正创建/关闭的主窗口和完整退出路径；`docs/stages/stage-01.md` 列出执行命令及结果，不含虚构测试通过记录。

---

## Stage 2　领域模型、数据库、配置与凭据

### 2.1 目标

建立可持久化、可迁移、可恢复的业务数据层，并让敏感凭据与普通配置严格分离。任何存储失败都不能被 UI 误报为保存成功。

### 2.2 涉及模块

`core::model`、`core::repository`、`storage::*`、`platform_win::credential`、`app::config_service`。

### 2.3 实现任务

#### 2.3.1 目录与数据策略

正式安装目录采用 `%LOCALAPPDATA%\Programs\LexWisp`；用户数据采用 `%LOCALAPPDATA%\LexWisp`，由 Known Folder API 解析，不拼接用户名。下设 `data/lexwisp.db`、`logs/`、`updates/`、`exports/`。应用不要求管理员权限；安装目录与数据目录分离。

首发不实现任意可迁移的 portable 数据目录；开发测试可用显式 `--data-dir`，但正式更新只针对受支持安装根目录。解析路径使用 Windows 宽字符能力，测试中文、空格、长路径与不可写目录。

**隐私说明必须明确**：SQLite 中的原文、译文、收藏和普通配置是本机明文数据，系统文件权限不等于数据库加密。API key 由凭据管理器保护，但同一用户权限下的恶意软件不在其充分防护范围内。不得宣称“本地存储所以绝对安全”。

#### 2.3.2 核心数据类型

| 类型 | 必须字段 | 关键约束 |
|---|---|---|
| ProviderId / RequestId / EntryId | UUID | 不用界面列表下标作持久标识 |
| ProviderConfig | id、kind、display_name、revision、enabled、endpoint、model、options、credential_ref | options 强类型反序列化；不可混入 key |
| QuerySpec | request_id、surface_id、query_generation、window_generation、provider_revision、source、mode、languages、prompt、privacy_epoch | 创建时形成不可变快照 |
| SelectionSnapshot | text、capture_method、foreground_pid、foreground_hwnd、cursor_physical、captured_at | PID/HWND 只用于本次校验，默认不入历史 |
| QueryResult | source、result、format、provider_snapshot、mode、language、finish_reason、usage、timestamps | finish_reason 区分完整与截断 |
| HistoryEntry | id、request_id、QueryResult snapshot、created_at | 只有经协调器接受的完整成功结果自动写入 |
| Favorite | id、result_snapshot、optional_history_id、note、created_at、updated_at | 独立快照，不依赖历史仍存在 |
| AppSettings | schema_version、revision、分组设置 | 校验范围；未知未来版本不能静默重置 |
| AppError | code、retryability、safe_message、request_id、redacted_context | 不携带可直接打印的完整 HTTP body 或 key |

`source` 保留实际选中文字，仅规范换行和外侧空白；不做 NFKC 改写，不自动删除标点或内部换行。搜索字段可独立规范化；内容哈希不替代原文，也不作为跨用户遥测标识。

#### 2.3.3 SQLite 最小逻辑模式

以下是必须实现的逻辑模式；工程可用迁移文件拆分执行，但不得弱化唯一约束、外键与修订号语义。

```sql
CREATE TABLE app_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    document_json TEXT NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    revision INTEGER NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0,1)),
    config_json TEXT NOT NULL,
    credential_target TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE history_entries (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    created_ms INTEGER NOT NULL,
    provider_id TEXT,
    provider_name TEXT NOT NULL,
    source_text TEXT NOT NULL,
    result_text TEXT NOT NULL,
    search_text TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 0)
);
CREATE INDEX history_time ON history_entries(created_ms DESC, id DESC);
CREATE INDEX history_provider_time
    ON history_entries(provider_id, created_ms DESC, id DESC);
CREATE TABLE favorites (
    id TEXT PRIMARY KEY,
    history_id TEXT REFERENCES history_entries(id) ON DELETE SET NULL,
    snapshot_hash TEXT NOT NULL UNIQUE,
    source_text TEXT NOT NULL,
    result_text TEXT NOT NULL,
    search_text TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX favorite_time ON favorites(created_ms DESC, id DESC);
```

`provider_id` 在历史中仅是可失效的来源标识，不级联删除历史。删除 Provider 后，旧结果仍显示保存时的名称、模型、模式和非敏感参数快照。`snapshot_json` 不重复存放大段 source/result，避免正文双份膨胀；至少包含其模式版本、格式、语言、finish_reason、耗时和可选 usage。

收藏唯一哈希由 Provider 快照、查询模式、语言、原文和完整结果生成；再次收藏同一快照返回现有记录。不按原文单独去重，避免不同译文互相覆盖。收藏编辑只允许备注，不修改已保存的翻译结果；刷新产生新结果快照。

#### 2.3.4 数据库运行参数与 Repository

采用 `rusqlite` bundled SQLite，启用外键、WAL、`synchronous=FULL`，以小事务保证收藏和配置耐久性优先。WAL 的存在与检查点行为是 SQLite 的已知机制；不能简单复制活跃的主 DB 文件当作完整备份。[S19][S20]

初始参数：单连接、statement cache 32、页缓存约 2 MiB、禁用 mmap、busy timeout 1500 ms。写事务不可包含外部 HTTP、凭据 UI、文件下载或 await。按写入量触发低频 checkpoint，不设置每秒后台维护；不要每次启动全量 VACUUM。

Repository 至少提供：

```text
SettingsRepository: load, compare_and_swap(expected_revision, new_settings)
ProviderRepository: list, get, create, update(expected_revision), delete
HistoryRepository: append_if_epoch, page(cursor, filter), delete_ids,
                   clear_and_advance_epoch, prune, export_page
FavoriteRepository: add_snapshot, page, update_note, remove, export_page
MaintenanceRepository: migrate, quick_check, backup, shutdown_checkpoint
```

查询分页默认 50 条，最大 100 条；使用 `(created_ms, id)` keyset cursor，禁止一次加载全部记录。搜索采用参数化 LIKE，并正确转义 `%`、`_` 和 escape 字符；默认保留的历史规模下先验证这一简单方案，不预先引入搜索服务或重型全文系统。中文子串查询必须可用。搜索输入 debounce 200 ms，新查询使旧查询结果失效；必要时使用 SQLite progress/interrupt 能力中止过时长查询。

前端最多保留三个列表页摘要、当前详情及少量滚动状态；列表摘要只取原文/结果前 120 字和元数据，不取整段正文。详情按 EntryId 单独读取。

#### 2.3.5 历史、删除与并发写入语义

默认自动保存成功查询；不自动保存失败、取消、仍在流式接收或超过输出限制而中断的结果。`HistoryEntry` 的 `request_id` 唯一，重复完成事件不能产生重复记录。

默认历史保留 **90 天、最多 10,000 条、正文总量最多 100 MiB**，达到任一条件即按最旧记录分批清理；收藏不受自动清理影响。清理分批执行，不阻塞 UI，不因此删除收藏快照。时间用 UTC 毫秒保存，本地格式化显示；排序另加 ID 打破同一毫秒并列。

“清空历史”以及“关闭历史记录”须使数据库中的 `privacy_epoch` 递增。所有旧请求的自动写入携带旧 epoch，DB actor 必须拒绝。这同时解决“用户刚清空，旧请求完成又把内容写回来”的竞态。清空操作返回成功前，先使 UI 快照与 pending 写入失效；之后新发起查询遵循新设置。

默认关闭历史后仍允许用户显式收藏，并在设置中说明；收藏是单独明确操作。部分结果只能通过“保存当前部分结果”显式保存，并标注 `partial=true`，不能冒充完整结果；这一入口如首发不提供，相关操作必须禁用，而不是暗中保存。

删除操作是逻辑业务删除，不保证 SSD、旧备份、WAL 或文件系统层的法证不可恢复。需要清理残留时提供显式“清理数据库空闲空间”，暂停写入、checkpoint 与重建，并准确说明不等于介质安全擦除。

#### 2.3.6 凭据存储与更新一致性

采用 `CredWriteW` / `CredReadW` / `CredDeleteW` 的 Generic Credential；凭据目标名使用 `LexWisp/<install-id>/provider/<uuid>/<secret-revision>`。持久化范围采用本机当前用户，不做漫游同步。Windows 凭据接口及其调用限制按官方定义实现。[S21]

API key 最大接受 2048 UTF-8 字节；过长时明确拒绝，不静默截断。内存包装不得实现暴露内容的 Debug/Display/Serialize；使用 zeroize/secrecy 类能力尽量缩短存活时间。HTTP 库或分配器可能形成不可控副本，因此不承诺密码学意义的内存全清零。

修改 key 使用补偿事务：先写入新 revision 凭据 → 在 DB 事务中更新引用及 Provider revision → 成功后删除旧凭据。DB 失败就删除本次新建凭据；崩溃留下的孤儿凭据由下次启动按本应用命名空间和有效引用回收。删除 Provider 先提交配置删除，再删除旧凭据；删除失败显示可重试清理项。绝不因系统凭据不可用而回退到明文。

读取密钥按请求获取，禁止为所有 Provider 长期缓存明文 key。测试连接的草稿密钥只在该次请求中使用，取消/离开页面后释放；保存成功后清空密码输入框的实际文本，UI 显示“已保存密钥”，而不是重新读取填入。

#### 2.3.7 迁移、损坏与备份

以版本化 SQL migration 和 `PRAGMA user_version` 管理模式，迁移前创建一致性备份。首次新建与已有损坏库必须分辨，不允许打开失败就覆盖新库。遇到未来 schema 版本进入保护模式，不能自动降级或重置。

迁移必须事务化；v1 更新链要求至少兼容 N/N-1 的读取合同，优先添加列/表，禁止未经更新协议升级进行破坏性迁移。备份采用 SQLite online backup 或在完全关闭连接并处理 WAL 后备份；不得直接复制一个正在写入的 `.db`。[S20]

### 2.4 验收标准

配置、历史与收藏重启后一致；失败/取消不产生成功历史；清空不被旧任务复活；删除历史不影响收藏；数据库与普通配置导出中扫描不到测试 key。数据库异常有恢复路径，不能丢弃原文件。

### 2.5 测试方法

真实临时 SQLite 测试、迁移前后 fixture、磁盘满/权限拒绝/写入失败注入、revision 冲突测试、秘密 canary 扫描。测试 10,000 条历史和 10,000 条收藏的分页、中文搜索、删除和重启；凭据使用隔离 target，清理不触及真实用户条目。

### 2.6 完成定义

Repository 合同与迁移可运行；关键事务测试通过；提供不含密钥的样例配置结构与数据说明；`docs/stages/stage-02.md` 记录备份恢复演练与性能结果。

---

## Stage 3　全局快捷键与安全取词

### 3.1 目标

在不抢先破坏外部选区、不误读剪贴板、不对受保护窗口强行操作的前提下，取得用户主动指定的选中文字。

### 3.2 涉及模块

`platform_win::{hotkey,selection,clipboard,host}`、`core::capture`、`app::command_router`。

### 3.3 实现任务

#### 3.3.1 快捷键管理

默认查询热键 **Ctrl+Alt+Q**；这是产品默认选择，不保证所有软件环境均无冲突。使用 `RegisterHotKey`、`WM_HOTKEY`、`MOD_NOREPEAT`，不安装持续键盘低级 hook。按 Win32 约束拒绝保留键和无效组合；冲突时 UI 清楚显示未注册，不暗中切换为别的键。必须测试不同键盘布局与 AltGr，说明 Ctrl+Alt 类热键在部分布局中的冲突风险并允许改绑。[S22]

更新热键采用临时新 ID 先注册新组合，再提交设置，最后注销旧组合。新注册或配置提交失败保留旧组合；相同组合走 no-op。暂停快捷键不删除用户配置；恢复时重新检测冲突。

收到热键立即记录 foreground HWND/PID、光标物理坐标、触发时间和请求代次。此时**不得激活 Popup、主窗口或设置页**。连续触发采用 latest-wins；旧取词事务结束也不能开启新查询。

#### 3.3.2 取词流程

```text
Hotkey
  → snapshot foreground + cursor + request generation
  → validate not own window / password / blocked app / locked session
  → UIA current focused element → TextPattern.GetSelection
  → validate source still current, selected text nonempty, within limits
  → success: deliver SelectionSnapshot
  → unsupported / empty / timeout:
       if policy permits conservative clipboard fallback → copy transaction
       otherwise → nonactivating failure card with manual-input action
```

UIA 只读取当前焦点元素及必要的有限祖先路径，最多 5 层、最多 8 个选区；不遍历整个桌面或文档树。多选区按接口返回顺序以换行合并，并标记 multiple_ranges。GetSelection 返回的空范围可能只是光标，不能将 DocumentRange、ValuePattern 的整段内容当作选中内容。[S23]

当来源为 LexWisp 自身时，使用其现有输入/结果选区动作或忽略重复触发，不通过模拟 Ctrl+C 自循环。`IsPassword` 明确为真、用户排除应用、锁屏、安全桌面、不同完整性级别无法访问时，直接拒绝自动取词；不申请管理员权限、UIAccess、DLL 注入或绕过 UIPI。[S25]

输入上限同时满足 **16,384 个 Unicode scalar 与 64 KiB UTF-8**。超过任一上限即拒绝整次自动提交，显示长度和手动编辑入口；禁止静默截断后发送。NUL 与非法 UTF-16 的处理必须显式，不能崩溃或意外拼接敏感邻近文本。

#### 3.3.3 UIA 超时与卡死处理

一个懒创建的 MTA worker 持有 UIA 接口。优先设置 `IUIAutomation2` connection timeout 300 ms、transaction timeout 500 ms；这些是本项目起始参数，可在取词设置的高级区调整。官方接口提供等待参数，但不意味着任意外部同步调用能被 Rust Future 取消。[S16][S24]

用户侧 UIA 总截止时间默认 800 ms，可在高级设置调整；总时限不得小于已配置的单项超时。超时立即结束当前取词的可见等待并忽略迟到结果。**不为每次超时新建线程**：最多一个正在执行调用、一个待处理请求。卡住的调用未返回时，将 UIA 标为“暂不可用”，转入手动输入或已授权回退；待调用真正返回后恢复 worker。连续超时三次对该来源进程熔断 60 秒，仅保存短期计数，不存窗口标题/正文。

不执行无限重试，不在 UI 线程 join 卡死线程；系统挂起/会话锁定取消活动取词，唤醒后从新请求重新验证来源。

#### 3.3.4 剪贴板回退事务

默认策略为 **UIA 优先 + 需在引导页明确启用的保守复制回退**；未同意前不发送 Ctrl+C。设置中说明复制回退会短暂更改系统剪贴板，剪贴板历史/第三方管理器可能观察到内容，恢复旧值不能撤回这些外部记录。

回退只允许当前来源仍未变化且不属于明确受保护字段。若无法排除敏感/密码输入，回退需要用户显式操作，不能自动强行复制。

事务必须满足以下顺序：

1. 在小型临界区打开剪贴板，记录 sequence number，并完整快照**支持恢复的全部现有格式**。默认仅支持空剪贴板与可完整保存的纯文本格式组合；不认识的富文本、图片、OLE、延迟渲染或超限格式存在时，保守模式拒绝修改，改用手动粘贴。快照总上限 1 MiB。
2. 释放剪贴板锁。等待用户本次热键修饰键实际释放，最多 300 ms；不得强行替用户释放其他按键。需要临时检查键状态时仅在本次事务短窗口中进行，后台不持续轮询。
3. 再次校验前台来源、请求代次和剪贴板 sequence 未被第三方改变；任一不符立即终止。使用 SendInput 发送明确的一次 Ctrl+C 序列，检查送入事件数量。
4. 仅在本次事务期间监听剪贴板变化，等待最多 600 ms；读取新 sequence 对应的 Unicode 文本。没有变化就失败，**绝不读取并提交旧剪贴板内容**。来源 PID/clipboard owner 可获得时用作辅助校验；sequence 变化本身不构成来源的绝对证明。
5. 验证前台来源未变化、数据格式与长度有效；无法可靠关联本次复制时，失败关闭，而不是猜测内容。
6. 恢复旧剪贴板前重新取得 OpenClipboard 锁，并在持锁状态比较 sequence 与本次复制结果。仍一致才恢复旧值；用户或其他应用已复制新内容时，不覆盖它。恢复失败单独提示，不假报成功。[S26]
7. 释放快照，移除本次剪贴板 listener、timer 与 hook，完成事务。取消也必须走清理路径；事务清理前不启动下一次模拟复制。

可提供默认关闭的“允许覆盖无法完整恢复的格式”高级选项，启用前必须明确风险；首发不得把此选项当成解决所有应用兼容性的默认捷径。所有 clipboard Handle 所有权、GlobalLock/Unlock 和 SetClipboardData 转移规则封装在小型 RAII 模块中。

#### 3.3.5 错误交互与取词兼容矩阵

| 场景 | 行为 | 用户可用动作 |
|---|---|---|
| 未选中文字 | 不发 Provider 请求 | 在主窗口手动输入 |
| 超过长度限制 | 不截断、不上传 | 打开编辑并明确缩短 |
| UIA 不支持 | 按权限尝试保守回退 | 开启回退或手动粘贴 |
| 剪贴板含不可恢复格式 | 默认不覆盖 | 手动复制/粘贴；明确改策略 |
| 来源切换或旧请求返回 | 静默丢弃旧结果 | 新请求继续 |
| 受保护/高权限窗口 | 不绕过权限 | 在允许的位置手动输入 |
| 快捷键冲突 | 保留旧有效配置或显示未注册 | 设置页重设 |
| UIA worker 卡住 | 不累积线程 | 手动输入；诊断提示 |

### 3.4 验收标准

普通编辑器、浏览器普通正文和至少一个 Office/PDF 阅读场景完成实测，记录每种场景实际走 UIA 还是回退。不可把若干软件成功测试宣传成“所有 Windows 软件均支持”。全过程不改变来源内容，恢复不覆盖用户的新复制，无选区时不发送请求。

### 3.5 测试方法

用受控 Windows 测试程序提供 TextPattern、空选区、密码字段、多选区、延迟、永不返回等行为；补充真实应用手工矩阵。验证连续热键、按住修饰键、切换前台、Clipboard owner 改变、相同文本新复制、第三方抢占、取消中恢复与退出时清理。

### 3.6 完成定义

热键与 UIA/回退功能可独立测试；错误码稳定；生成 `docs/research/selection-compatibility.md`；敏感来源、不完整快照和超时竞态没有被“暂时忽略”。

---

## Stage 4　Provider 协议、请求协调、流式响应与取消

### 4.1 目标

实现两个可实际配置使用的 Provider adapter，以及完整的并发、取消、错误与流式数据合同。Mock 只能作为测试输入，不能成为正式 Provider 功能的替代品。

### 4.2 涉及模块

`providers::{transport,openai_chat,deepl,sse}`、`core::{query,error}`、`app::query_coordinator`。

### 4.3 实现任务

#### 4.3.1 首发 Provider 与能力

| Provider kind | 协议与能力 | 采用范围 | 不支持时的行为 |
|---|---|---|---|
| `openai_chat` | Chat Completions 兼容 HTTP API；可选 SSE 流式 | 翻译、解释、自定义模板；OpenAI 与经测试兼容端点 | 显示具体协议错误，不宣称任意“兼容”服务都可用 |
| `deepl` | DeepL Text Translation HTTP API；非流式 | 翻译；Free/Pro 地址选择 | 禁用解释和自定义 prompt |
| `mock` | 可编排成功、延迟、流、错误、取消 | 测试构建与开发工具 | 正式设置页不出现 |

OpenAI adapter 明确使用 `/chat/completions` 协议，不自动混用 Responses API；DeepL 使用 `/v2/translate` 的文本数组、目标语言与认证字段。协议以各提供方的官方文档及锁定 fixture 为依据。[S29][S30]

Provider 不包含 UI、数据库写入、热键或窗口控制。网络成功只产生结果，是否展示、入历史和收藏由协调器及 Repository 决定。

#### 4.3.2 Provider 合同

以下是本项目要实现的接口合同，不是对 GPUI 现有 API 的逐字声明；DTO 与错误类型由 core 定义，transport/取消相关类型由 providers 定义。

```rust
// Object-safe adapter boundary. Implement with boxed Send futures.
trait Provider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> ProviderCapabilities;
    fn validate(&self, config: &ProviderConfig) -> Result<(), ConfigError>;
    fn execute<'a>(
        &'a self,
        request: ProviderRequest,
        context: RequestContext,
        deltas: DeltaSink,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutput, ProviderError>> + Send + 'a>>;
}
```

`RequestContext` 包含 cancellation token、截止时间、已验证配置 snapshot、按次取出的 secret，以及安全传输句柄；不能包含 GPUI Entity 或 Window。`ProviderOutput` 包含最终完整文本、finish_reason、可选 usage 和时序元数据。`DeltaSink` 是有界出口，可在消费者关闭或取消时立即停止生产。

新增 Provider 只需添加 adapter、配置类型、能力描述、表单字段和协议测试；不得修改 Popup 的 HTTP 处理逻辑。Adapter registry 在启动时静态注册，不加载第三方动态代码。

#### 4.3.3 Provider 配置模型

公共字段：名称、启用状态、类型、endpoint、凭据引用、请求超时、最大输入/输出、网络策略。OpenAI 字段：model、流式开关、可选 temperature、输出 token 限制字段策略、受限自定义 header。DeepL 字段：Free/Pro、目标语言映射、可选 formality（仅在服务支持时发送）。

OpenAI model 必须由用户填写或从显式获取的模型列表选择；不硬编码可能退役的模型。默认不后台拉取模型列表。不同模型对 temperature、`max_tokens` 与 `max_completion_tokens` 的支持不同，允许“不发送/指定其中一种”的明确配置，不能盲目同时发送或无声降级。

endpoint 表示完整协议基地址，例如 `https://api.openai.com/v1`；用 URL 结构化解析后追加固定路径，正确保留基路径并避免重复 `/v1`。禁止把任意用户字符串拼接成 shell 命令。UI 显示最终请求路径的非敏感预览。

自定义 header 名和值必须拒绝 CR/LF；禁止覆盖 Host、Content-Length 等传输控制字段；认证 header 只能经专门秘密字段管理，不允许把 key 填进可导出的普通 header JSON。URL 中禁止 userinfo、fragment 和作为凭据使用的 query 参数。

#### 4.3.4 网络与安全默认值

远程 endpoint 必须 HTTPS，证书验证保持开启。允许用户明确启用 loopback HTTP，只接受字面 `127.0.0.1` / `[::1]` 范围及经过限制解析的 localhost；远程私网 HTTP 不作为默认例外。不能把“允许本地 HTTP”扩展为“忽略所有 TLS 错误”。

认证请求默认禁止重定向；不跟随到另一主机携带 Authorization。Provider client 按有效网络配置 revision 复用，连接池空闲保留 30 秒、每主机最多两个空闲连接；配置变化释放旧 client 的空闲池，活动请求仅持有其自己的 snapshot。最多保留四个配置 client，不为每次查询永久创建一个。

网络模式首发提供 **直连** 与 **显式 HTTP/HTTPS 代理**；直连明确禁用环境代理的意外继承。代理地址禁止嵌入明文密码；需要认证时凭据同样进入 Credential Manager。不要把未经实现的 PAC/系统代理自动发现开关做成可选但无效的 UI。代理模式变更必须重建相应 client 并重测。

初始超时：连接 5 秒、首个有效响应 15 秒、流式空闲 20 秒、整次请求 90 秒。都受 cancellation token 控制；只重置流空闲计时，不重置总截止时间。设置允许合理范围调整，总时限上限 300 秒。v1 **不自动重试可能计费的 POST**；按钮明确提供手动重试。429 读取有界 Retry-After 并显示可重试时间，不能无限循环重试。

输入本地上限沿用 Stage 3；非流式 HTTP body 最大 1 MiB，最终结果最大 **512 KiB UTF-8**。流式单事件最大 256 KiB、单行最大 128 KiB；超限结束为 `ResponseTooLarge`，不继续无限读取。Content-Type 与内容不符时给协议错误或按用户显式兼容模式处理，不将任意 HTML 错误页当译文。

#### 4.3.5 Prompt 与语言语义

模式包含 `Translate`、`Explain`、`Custom`。默认 `Translate`，源语言 `auto`，目标语言 `zh-Hans`；不基于简单“含汉字”判断偷偷反转中英方向。目标语言下拉可快速切到英文；Provider capability 必须验证语言映射。

AI 翻译默认 system 指令限定翻译任务、目标语言和只返回结果；source 独立放入 user 内容，并清楚标记其为待处理资料。自定义模板只支持明确列出的 `{text}`、`{source_language}`、`{target_language}`、`{mode}` 变量；不执行代码、文件读取、网络工具或 shell 插值。

Prompt injection 防御采用职责隔离而不是宣称模型绝不会偏离指令。AI 输出可能不准确或不遵循格式；应用不执行其内容，不据此打开任意文件，不自动调用工具，也不把其中的“系统指令”升级为应用命令。

DeepL 源语言自动识别使用其协议，目标语言映射由 adapter 维护；不支持的语言在发请求前报错。usage 可为空，不伪造 token 或账单价格；无可靠单价时仅展示 token/字符用量，不推算费用。

#### 4.3.6 请求状态机与竞争规则

```text
Idle
  → Capturing (global shortcut only)
  → Validating
  → Connecting
  → Streaming / WaitingNonStream
  → Succeeded | Failed | Cancelled | Superseded | Truncated
```

**每个 Surface 最多一个活动请求**。首发两个 Surface：Popup、主窗口手动查询。全应用同时最多两个 Provider 请求，包括测试连接；测试连接不建立长队列，忙时显示“稍后测试”。同一 Surface 新查询/刷新/切换 Provider 后重新执行，必须先使旧 generation 失效并取消旧请求。不同 Surface 不互相覆写或错误取消。

每个事件带有 `(surface_id, query_generation, request_id, window_generation, provider_revision)`。协调器接收时逐项检查；WindowGeneration 变化后不能把旧结果送进新建 Popup。UI 已完成/已取消状态不能再被迟到 delta 改回 Streaming。

成功的线性化点是协调器接受最终结果，并验证请求仍是当前代次的时刻。在此之前被取消就不自动保存；已经接受成功并向 DB 发出合法写入后，再关闭窗口不将已完成结果改成失败。历史清理通过 privacy_epoch 另行处理，不靠时间猜测先后。

同一文本重复按热键默认仍视为新查询，避免缓存错用旧 Provider/语言配置。v1 不实现持久化响应缓存；用户刷新一定生成新 RequestId，并明确可能再次计费。

#### 4.3.7 流式传输与 UI 交付

Chat Completions 请求固定 `n=1`，只处理对应 choice。收到有效结束标记或该 adapter 明确认可的完成语义且文本非空，才可判定成功；`finish_reason=length` 标为截断，拒绝/内容过滤显示专门状态，工具调用标为不支持。EOF 本身不证明成功，空文本不记成功历史。

SSE parser 必须处理 UTF-8 跨字节块、CRLF、事件跨 HTTP chunk、多 `data:` 行、空事件、comment、结束标记、正常终止元数据、突然 EOF 和服务端错误。不能用 `split("\n\n")` 加“每块就是一个 JSON”的假设。只提取支持的 content 字段；工具调用、图片、未知结构被忽略或报不支持，不执行。

每请求正文 delta channel 容量 64，每个 delta 最多 8 KiB；接收端慢时施加背压，发送等待也必须响应取消。终态使用独立的可靠完成通道，携带最终完整结果与最后序号：最终结果是权威值，终态先到时可直接应用最终文本并丢弃同代次残余 delta，防止少字、重复追加和结束丢失。

在 UI 桥接层合并增量，最多每 33 ms 通知一次结果区域；一次 UI 更新处理量和解析时间受限制，不能在 UI 上排空无界流。高频 usage/progress 合并；按钮和整页导航不因每个 token 重建。首个有效内容允许立即显示，不等待固定批量满。

Provider worker 内的最终文本 buffer、UI 当前文本及冻结选择 snapshot 各有上限，总体必须纳入 Stage 11 内存测量。不保存无限增长的每 token Vec、完整事件历史或多份 Markdown AST。

#### 4.3.8 取消合同

触发取消的事件包括：新代次查询、用户停止、Popup 隐藏/关闭、主窗口查询页明确取消、删除/禁用当前 Provider、会话锁定、进程退出、更新进入安装阶段。修改 Provider 配置只影响新请求；涉及 endpoint/密钥删除或禁用时取消该 Provider 的旧活动请求。

取消必须依次实现：立即更新可见状态 → token.cancel → 停止 delta 发送 → 丢弃响应 stream/future → 释放网络资源和 semaphore permit → 清理 TaskRegistry。不得仅“不再更新 UI”而继续后台完整读取响应。

本地取消确认目标小于 50 ms，发布门槛 P95 ≤100 ms；DNS/第三方内部阻塞等不可控环节需单列，不能继续占据应用请求槽。取消 HTTP 连接不保证服务端停止生成或停止计费，UI 帮助中必须说明。

### 4.4 验收标准

两个真实 adapter 能按配置工作；OpenAI 流式与非流式都可测试；DeepL 能正确处理非流式结果。新请求不会混入旧文本；取消后不自动入历史；超限、断网、401、429、5xx 和畸形流都有稳定错误。

### 4.5 测试方法

建立本机协议测试服务器，覆盖每一个字节边界切分 UTF-8/SSE、多事件合并、缓慢首字节、持续空事件、丢结束帧、输出超限、取消时阻塞发送、晚到完成、429/Retry-After、跨域 redirect 和 TLS 失败。测试 server 记录连接关闭与请求数，证明未偷偷重试。真实服务 smoke 需操作者提供凭据并显式执行，CI 默认只用确定性 fixtures，不上传用户内容。

### 4.6 完成定义

Provider 合同、状态机与所有竞态测试落地；新增 adapter 的路径可说明；至少保留一次经授权的真实 Provider smoke 记录或明确列为发布前外部凭据门禁，不以 Mock 通过假冒服务兼容验证。

---

## Stage 5　Popup 交互、文本选择与窗口生命周期

### 5.1 目标

完成核心划词链路：触发 → 取词 → 鼠标附近首绘 → 流式/完整结果 → 选择复制/收藏/刷新 → 隐藏 → 到期真实销毁。

### 5.2 涉及模块

`ui::popup`、`ui::business_views::result`、`app::query_coordinator`、`platform_win::{window_bridge,dpi}`。

### 5.3 实现任务

#### 5.3.1 窗口与查询状态分离

```text
Popup lifetime
  Absent
    └─ show → Creating → Visible
  Visible
    ├─ hide → HiddenWarm(deadline, window_generation)
    ├─ pin / unpin → Visible
    └─ app exit / session lock → Destroying → Absent
  HiddenWarm
    ├─ show before deadline → Visible (same window)
    ├─ deadline / immediate release → Destroying → Absent
    └─ app exit / session lock → Destroying → Absent
```

默认 `idle_destroy_seconds = 30`，允许 0–300 秒；0 表示隐藏后立即销毁，**不表示永久保留**。不提供永久隐藏作为默认或隐含选项。TTL 从真正隐藏时刻起算，使用单调时间；再次显示取消旧 timer，使用 generation 检查阻止过期 timer 关闭新窗口。

隐藏时立即取消尚未完成的请求并停止动画、输入光标和流式刷新；窗口保温不是请求继续运行的许可。到期必须移除 GPUI window，清空 WindowHandle、View Entity、Subscription、TaskScope、选择 snapshot、缓存段落及其强引用。保留进程级控制器和小型偏好，不保留整棵 UI 树。

已经成功的结果在保温期间可保留用于短时重新打开；首次新查询清除不属于新代次的内容。到期销毁后不为“记住最后一条”在全局另存大段正文；需要找回结果时读取历史。pin 只控制失焦自动隐藏，不产生第二个 Popup，也不使关闭后的窗口永久驻留。

#### 5.3.2 创建、隐藏、激活与销毁 API 边界

创建使用 GPUI 的真实 WindowOptions 和 `Root::new`；无边框视觉沿用 Kit 官方样例结构。WindowOptions 中初始可见性、激活与装饰字段以锁定版本编译验证，不杜撰属性。建议先不可见/不激活创建，待原生样式和坐标确认后显示。[S13]

`WindowBridge` 的边界限定为：取得有效 raw HWND、设置 tool-window 样式、非激活显示、隐藏、安全请求焦点、位置与 topmost 调整。所有调用在 UI 所属线程执行，句柄关联 WindowGeneration。不得跨线程直接使用旧 HWND，不能只凭数值相同认为窗口仍是原窗口。

隐藏采用已验证的单窗口 Win32 行为并通知业务状态；不调用空实现的应用级 hide。销毁必须经 GPUI 移除窗口并让它管理原生释放，不能从外部强行 DestroyWindow 绕开 GPUI 的状态表。[S08][S09]

Popup 默认不在任务栏与 Alt+Tab 中出现，使用 `WS_EX_TOOLWINDOW` 等适当样式。默认以非激活方式靠近鼠标显示，首次捕获完成前不夺取外部焦点。普通点击结果区域后允许原生激活以便选择复制；不永久设置阻止一切交互的 `WS_EX_NOACTIVATE` 而导致复制失效。

主动键盘模式可显式请求焦点，但不得借用本次快照包含模拟 Alt 的 GPUI activate 路径；使用窄适配调用系统允许的 SetForegroundWindow 等行为，失败时保留可点击 Popup，不做焦点窃取绕过。Windows 本身对前台激活有限制。[S10][S28]

#### 5.3.3 布局与交互

默认宽 420 DIP，范围 320–720；默认最大高 560 DIP，且不得超出当前 monitor work area。窄屏时自动缩小。根布局为标题/工具栏、原文折叠区、结果滚动区、状态栏；不按每个 token 改变外层窗口尺寸。

| 区域 | 必须内容 | 交互合同 |
|---|---|---|
| 顶部 | Provider、目标语言、pin、关闭 | 改 Provider/语言后显式开始新代次；pin 不开启新窗口 |
| 原文 | 前几行预览、展开、复制原文 | 可选择；长文本内部滚动，不撑满桌面 |
| 结果 | 加载、流式、成功、错误/部分结果 | 使用 Kit 文本选择；内容区可滚动 |
| 工具栏 | 复制结果、收藏、刷新/停止、打开主窗口 | 复制默认复制完整结果；选区内 Ctrl+C 复制选区 |
| 底部 | 模式、状态、耗时、可选 usage | 不展示猜测的账单费用；错误有操作入口 |

首次取词成功后先显示本地 loading shell，不等待 HTTP 首字节。取词失败可显示非激活错误卡，但不能先抢焦点再试取词。键盘 Tab 顺序从 Provider/语言到结果及操作区；焦点在 Popup 内时 Esc 关闭最上层菜单/对话框，随后才隐藏 Popup。Popup 未获得焦点时不全局劫持 Esc。

“刷新”使用当前原文与当前选定配置新建 RequestId；刷新中保留旧成功内容的视觉预览并标记“正在重新查询”，不得把预览当作本次结果保存。停止后保留已收到部分文本并标注未完成，禁用普通收藏；已有成功历史不因刷新失败丢失。

复制结果在成功后可用；流式过程可复制当前快照，但必须标为“复制当前内容”，且不能自动记录为完整成功。点击收藏须等待数据库成功回执再显示已收藏；重复点击幂等，失败可重试。

#### 5.3.4 文本选择与 Markdown 安全

优先复用 `TextViewState` 的增量更新与 `TextView::new(...).selectable(true)`，参考官方流式和选择样例；视图状态在构造时创建，不在 render 内重新分配。[S06][S14]

默认结果以**可选择纯文本**展示，优先使用 Kit 的 SelectableText 将 Provider 输出作为字面文本；不得经 Markdown 解析后声称它是纯文本。简单 Markdown 为用户可启用的表现选项，使用单独的 TextViewState 路径。Markdown 不得加载远程图片、HTML、脚本、iframe、文件资源、数学执行扩展或任何后台网络内容；若上游组件无法逐项关闭，回退到纯文本，不能只写“上游应该安全”。外链仅对用户明确点击的 http/https 地址生效，显示目标域名；拒绝 file、javascript、data 和自定义可执行协议。

用户开始拖选或键盘扩展选择时冻结当前可见文本 snapshot，继续有界接收后台内容，显示“有新内容，恢复更新”。结束选择后不擅自清空选区；用户点击恢复或完成复制后按明确交互更新。若 Kit 不能直接暴露足够的 selection-change 事件，先用已有 pointer/key 事件实现窄业务冻结与显式恢复按钮，不重写通用 selection engine。流式完成也要尊重冻结 snapshot。

自动滚动仅在用户原本处于底部且未选择文本时发生；用户上滚后暂停跟随。长 URL、代码、RTL、emoji、组合字符、中文标点与换行均不能破坏选择边界。右键菜单提供复制选区/全部，使用已有菜单能力。

#### 5.3.5 放置、DPI、失焦与系统事件

以热键触发时的鼠标**物理坐标**作为锚点，选对应 monitor work area。先尝试右下偏移 12 DIP，空间不足翻转到左/上，再夹紧到 work area。负坐标正常处理；不得把跨显示器全局物理坐标直接除以某个显示器 DPI 当作全局逻辑坐标。

GPUI 的显示器坐标与原生坐标转换集中在一个 module：按目标 monitor 原点和比例转换尺寸、偏移，并明确最终哪一层负责定位。处理 WM_DPICHANGED 时避免与 GPUI 重复缩放；保存偏好用 DIP，实际 HWND 定位使用已验证映射。[S27]

默认失焦隐藏，但“未激活显示的预览窗”不能因最初本来没焦点而马上隐藏。创建时进入 `Preview` 子状态：外部前台未改变时保留；用户点击其他窗口、切换前台或进入已激活后失焦才隐藏。需要 foreground/mouse 事件时仅在可见期注册窄事件监听，关闭时注销；不安装全时低级鼠标 hook。初始 preview 最多可见时间默认关闭自动超时，用户可设置。

pin 时忽略普通外部失焦，仍响应关闭、Esc、锁屏和退出。锁屏或会话切换立即取消请求、销毁 Popup、清除可见正文；解锁不自动重新展示敏感内容。显示器断开时重新定位；休眠唤醒清理过时 timer 和来源窗口，不继续旧取词。

### 5.4 验收标准

温热与冷创建均可完成完整流程；30 秒策略真实释放窗口而非 hide；选择复制正确；流式刷新不夺走选区；失焦、pin、Esc 与覆盖层行为一致；关闭后的后台不继续拉取请求。

### 5.5 测试方法

Kit 无头测试点击真实业务按钮，检查回调、焦点和布局；Windows 真机验证非激活首显、选区、剪贴板、DPI、Alt+Tab、外部失焦。使用假时钟测试 TTL=0/30/300、新建与旧 timer 竞态；执行 1,000 次显示/隐藏/销毁循环并记录资源趋势。

### 5.6 完成定义

Popup 使用真实 Provider pipeline 和存储；控制器状态图有自动化覆盖；冷/暖首绘与取消时延有实测文件；Stage 0 的框架限制已通过局部适配处理，未用永久隐藏主窗规避。

---

## Stage 6　主窗口、手动查询、历史与收藏管理

### 6.1 目标

形成完整桌面管理入口，同时保证主窗口不会成为隐藏后的长期内存负担。

### 6.2 涉及模块

`ui::main_window`、`ui::pages::{query,history,favorites}`、共享 `ResultView`、相关 Repository。

### 6.3 实现任务

#### 6.3.1 主窗口壳

一个可调整大小的主窗口，默认 960×680 DIP、最小 720×480 DIP；较小工作区按实际尺寸降级。左侧导航包含“查询”“历史”“收藏”“设置”“关于与诊断”。主内容只有当前页和必要缓存，不一次实例化所有设置表单、所有历史详情。

关闭按钮实际销毁主窗口，不退出主进程；存在未保存草稿时显示保存/丢弃/取消，不把关闭解释为保存。最小化遵循普通 Windows 行为，但不是自动隐藏到托盘的替代实现。再次从托盘打开时重建页面，恢复少量已提交偏好，不恢复明文密码输入。

所有窗口共享 Kit 主题 tokens 与 LexWisp 业务 `ResultView`，但不共享会混淆焦点或选择的同一个可变 TextView Entity。ResultView 的复用是代码复用，每窗口拥有独立状态。

#### 6.3.2 手动查询页

多行输入、Provider、模式、源/目标语言、查询/停止、交换语言、清空、粘贴和结果区。默认 Ctrl+Enter 提交，Enter 换行；中文输入法 composition 未提交时不能误触发查询。全局热键不承担页面的文本编辑快捷键。

用户按“粘贴”是明确读取当前剪贴板，与 Stage 3 的自动取词回退不同；此处可以读取旧剪贴板，因为用户显式要求粘贴，但仍执行格式和长度校验。源文本可手动编辑；点击查询后创建不可变 snapshot，之后输入修改不改变正在执行请求。

从 Popup “打开主窗口”传递当前业务 snapshot，不重复收费发请求；从历史打开同样先显示保存结果。只有用户按查询/刷新才建立新请求。主窗口切页不默认开始新查询；离开查询页时取消仍在运行的请求并提示未完成，避免不可见持续生成。

#### 6.3.3 历史页

列表/详情布局，按时间倒序；支持关键词搜索、Provider 筛选、日期范围、单项删除、多选删除、清空、复制原文/结果、再次查询、收藏和导出。默认列表只读摘要，点击后加载全文。

“再次查询”使用当前可用 Provider：原 Provider 仍可用时预选；已删除时提示选择替代者，不能偷偷将旧文本发往另一个服务。旧结果必须标明保存时 Provider/模型和时间。

确认清空对话框说明不会删除收藏。页码不使用大 OFFSET 深翻页；搜索/过滤变化重置 cursor，异步结果带 filter revision，过期查询不替换新列表。

#### 6.3.4 收藏页

支持搜索、详情、复制、备注编辑、取消收藏、批量删除、导出和再次查询。收藏保留原文、完整结果、Provider 快照、语言/模式、创建时间，不随历史清理丢失。

同一收藏从不同页面取消时广播 `FavoriteChanged(id)`，当前 Popup/详情中的状态同步，不重新读取所有收藏。默认不加载全收藏哈希集合到内存；查询当前 snapshot 是否已收藏由索引和局部缓存完成。

#### 6.3.5 导出与数据可携带性

提供版本化 JSON 和可读 Markdown 导出，按分页流式读取并写入临时文件，完成后原子提交目标文件；取消时移除临时文件。导出仅包含用户所选历史/收藏与非敏感元数据，不含任何密钥、代理凭据、日志或机器标识。

导出大数据时显示进度，但不得每条记录强制整页渲染；对 Markdown 正确转义围栏/分隔内容，避免原文破坏结构。v1 不提供 CSV 以减少公式注入面，也不提供未经验证的任意数据库导入。数据库备份恢复是单独维护流程，不等同于业务导入。

### 6.4 验收标准

历史与收藏完整可用，关闭重开一致；主窗口关闭释放页级 Entity；10,000 条记录不一次装入内存；搜索/翻页无旧结果闪回；导出取消和失败不留下看似完整的坏文件。

### 6.5 测试方法

生产页面无头交互测试、Repository 集成、中文输入法真机测试、键盘完整遍历、10,000 条记录分页资源测量、删除后详情失效、导出中取消/磁盘满测试。

### 6.6 完成定义

主窗口具备真实业务能力而非静态占位；查询/历史/收藏与 Popup 使用同一领域协议；每条保存、删除、再次查询路径有测试证据。

---

## Stage 7　设置系统、主题、快捷键与开机启动

### 7.1 目标

使用户无需编辑配置文件即可配置全部首发功能。设置页必须真实控制运行行为，并具备草稿、校验、保存回执、失败恢复与重启一致性。

### 7.2 涉及模块

`ui::pages::settings`、`app::config_service`、`platform_win::{startup,hotkey,credential}`、主题与窗口控制器。

### 7.3 实现任务

#### 7.3.1 页面组织与保存模型

设置按“常规”“Provider 与网络”“查询与取词”“快捷键”“Popup”“外观”“数据与隐私”“更新”“高级与诊断”分组。优先使用 Kit 的导航、Form、Select、Input、Textarea、Switch、Dialog 等能力；不做无限滚动、没有标题层级的一整页开关堆积。

每个分组拥有 draft 与 base_revision。普通输入修改不直接写库；点击保存或明确的即时开关操作才提交。主题可实时预览，取消恢复已提交主题；热键、凭据、启动项必须先校验再生效。页面显示保存中/已保存/失败，禁止无反馈吞掉错误。

对所有普通配置按分组保存；不要求一个巨型事务跨 SQLite、Credential Manager 和注册表实现不存在的原子性。ConfigService 协调可补偿操作，保留旧值与新值，失败回滚；跨系统崩溃一致性通过持久化的非敏感操作意图和启动恢复完成。设置 revision 冲突时要求重新加载或显式合并，不覆盖其他窗口刚提交的值。

#### 7.3.2 首发设置清单

| 分组 / 字段 | 默认值 | 有效范围 / 约束 | 生效与持久化规则 |
|---|---|---|---|
| 常规 / 登录启动 | 关闭 | 当前用户 | 成功写入/移除自身启动项后回执 |
| 常规 / 启动到后台 | 配置完成后开启 | 首次启动仍显示引导 | 下次启动；不弹空窗口 |
| 常规 / UI 语言 | 简体中文 | v1 可先只提供简中，结构预留 i18n | 不显示未实现语言 |
| Provider / 名称、启用、类型 | 用户创建 | 名称 1–80 字符；类型不可无声转换 | 保存后 revision++ |
| Provider / endpoint、model | 无隐含可用模型 | URL 与协议约束见 Stage 4 | 新请求生效 |
| Provider / key / 代理 secret | 未设置 | 系统凭据；不得明文导出 | 保存或测试后清空输入框 |
| Provider / 默认 Provider | 首个有效配置或用户选择 | 必须启用且校验通过 | 只影响新请求 |
| Provider / streaming | AI 默认开 | adapter capability 限制 | 新请求生效 |
| Provider / 模型参数 | 不发送可选参数 | 显式 temperature / token 字段策略 | 不猜测服务支持 |
| 网络 / 模式 | 直连 | 直连、显式 HTTP/HTTPS 代理 | 重建对应 client |
| 网络 / 本地 HTTP | 关闭 | 只允许受限 loopback | 需风险提示后开启 |
| 网络 / 超时 | 5 / 15 / 20 / 90 秒 | 总时限 ≤300 秒，各值独立校验 | 新请求生效 |
| 查询 / 模式 | 翻译 | 翻译、解释、自定义 | 根据 Provider 禁用不可用项 |
| 查询 / 源与目标语言 | auto → zh-Hans | adapter 支持列表 | 新请求生效 |
| 查询 / 模板 | 内置翻译/解释模板 | 自定义 ≤8 KiB；变量白名单 | 本地预览不发网络 |
| 取词 / 复制回退 | 未授权时关闭 | 保守回退；风险覆盖另行确认 | 下次取词生效 |
| 取词 / 应用排除 | 空 | 按用户选择的 exe 标识；不匹配标题关键词 | 取词前先检查 |
| 取词 / UIA 超时 | 300 / 500 ms | 100–2000 / 100–3000 ms | 仅空闲 worker 安全更新 |
| 取词 / UIA 总等待 | 800 ms | 300–5000 ms；不小于单项超时 | 新请求生效；不等于可强杀同步 COM 调用 |
| 快捷键 / 查询 | Ctrl+Alt+Q | RegisterHotKey 可注册组合 | 注册成功才保存 |
| 快捷键 / 暂停 | 非持久暂停状态 | 不更改已保存组合 | 立即生效 |
| Popup / 隐藏后销毁 | **30 秒** | 0–300；0 为立即销毁 | 隐藏窗按新期限重算，已超时立即销毁 |
| Popup / 宽度 | 420 DIP | 320–720 | 可见窗立即布局；保留屏幕约束 |
| Popup / 最大高度 | 560 DIP | 240–900，且不越 work area | 不逐 token 自动增高 |
| Popup / 鼠标偏移 | 12 DIP | 0–64 | 下次定位 |
| Popup / 失焦隐藏 | 开 | pin 可临时覆盖 | 预览子状态不能误判 |
| Popup / 默认 pin | 关 | 一次最多一个 Popup | 新显示生效 |
| Popup / 非激活预览 | 开 | 可选显式键盘聚焦模式 | 遵守 Windows 前台限制 |
| Popup / 可见自动隐藏 | 关闭 | 启用时 5–120 秒 | 与隐藏后销毁 TTL 独立 |
| Popup / 停止后保留部分结果 | 开 | 永远标记未完成 | 不自动保存成成功 |
| 外观 / 主题 | 跟随系统 | 系统、浅色、深色 | 预览；保存后持久化 |
| 外观 / 字号 | 14 DIP | 12–20 | Kit tokens；不重建整个应用 |
| 外观 / 字体 | 系统 UI/中文 fallback | 仅本机已安装字体 | 不打包大体积字体库 |
| 外观 / Markdown | 关 | 可选择纯文本始终可用 | 需通过资源加载安全测试 |
| 外观 / 动效 | 跟随系统减少动态效果 | 普通/减少 | 隐藏窗永不持续动画 |
| 隐私 / 保存历史 | 开 | 首次引导明确说明 | 关闭立即提升 privacy_epoch |
| 隐私 / 历史保留 | 90 天 / 10,000 条 / 100 MiB | 天数 1–365；条数 ≤50,000；容量 ≤500 MiB | 修改后分批清理并提示影响 |
| 隐私 / 收藏 | 显式操作保留 | 不受历史清理影响 | 清空需单独确认 |
| 数据 / 导出、备份、清理 | 手动 | 输出位置由用户选择 | 取消/错误不损坏源数据 |
| 更新 / 自动检查 | 开 | 每 24 小时最多一次，启动延迟后检查 | 仅元数据；不自动安装 |
| 更新 / 自动下载 | 关 | 可手动下载 | 不影响 Provider 请求优先级 |
| 更新 / 通道 | stable | v1 不显示未发布通道 | 禁止无签名自定义通道 |
| 诊断 / 日志级别 | warn + 必要 info | debug 有时限，最多当前会话 | 始终脱敏，不允许正文日志 |
| 诊断 / 导出报告 | 手动 | 展示清单与预览 | 明确授权后写出 |

这些字段必须映射到强类型 Settings，而不是散落字符串键。固定安全上限、签名信任根、任意 shell、关闭 TLS 验证等不属于可配置产品项。实现里出现新的用户偏好时，必须同步设置 UI、schema、默认值、迁移、测试和帮助；不能只把它藏在配置文件中。

#### 7.3.3 Provider 编辑与测试

Provider 管理支持新增、复制非敏感配置、编辑、删除、启用/禁用、设为默认、测试连接。复制配置不复制或显示 key，用户可以显式复用已有凭据引用的功能不进入 v1，避免跨 Provider 意外共享秘密。

测试连接必须使用当前草稿，不偷偷保存。AI 测试发送固定无敏感测试文本和用户选定模型；界面说明会访问外部服务且可能计费。测试时可取消，不读取当前剪贴板或用户最近查询。HTTP 认证通过但模型不存在不能显示“完全可用”；结果区分连通性、认证、模型/协议与实际生成测试。

删除当前默认 Provider 前选择替代项或设置为无默认；无 Provider 时查询按钮给出配置入口，不发不完整请求。删除正在使用的 Provider 取消其活动请求；历史保留原始快照。

#### 7.3.4 主题与无障碍

跟随系统主题通过系统事件及 Kit 主题机制处理，不每秒读注册表。可见窗口同步更新，隐藏保温窗不持续重绘，重建窗读取最新主题。颜色、间距、边框、圆角由 Kit tokens 管理；LexWisp 只定义有限品牌 accent 和业务状态 token。

支持系统高对比/减少动态效果下的可读降级。所有图标按钮有可访问名称、Tooltip 与可见焦点，不能仅用颜色表达错误。选区对比、禁用态、滚动条和小字号在深浅主题均需实测；系统字体 fallback 必须包含常见中文与 emoji，不附带或分发系统字体文件。

#### 7.3.5 登录启动

采用当前用户 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `LexWisp`，值为带引号的绝对 exe 路径与固定 `--background`。不写 HKLM，不创建提权计划任务，不修改其他软件启动项。Windows 可延迟启动项执行，应用不能保证登录后立刻运行。[S34]

保存时验证完整命令长度与系统限制、安装路径存在且属于当前安装。启用失败不显示已开启；禁用只移除本应用对应值。区分“应用请求启用”“启动项已注册”和“系统实际允许执行”，不要把注册表中存在值误报为用户在任务管理器未禁用。系统策略或用户外部设置限制时给准确提示，不绕过系统控制。

安装移动/修复由安装器或显式维护操作更新路径，不能在每次运行时偷偷重新启用已被用户关闭的项。卸载时移除自己的项。

### 7.4 验收标准

表中所有已开放设置都有效、可保存、可恢复；草稿未保存不会污染运行配置；主题取消可回退；热键冲突、凭据错误、注册表失败保留旧配置；重启后无默认值回跳。

### 7.5 测试方法

对每个字段实施默认值/边界/非法值/保存/重启测试；对系统副作用做故障注入和补偿恢复；窗口打开时改变主题与 TTL；启动项与任务管理器外部禁用手工测试；秘密字段序列化扫描。

### 7.6 完成定义

设置清单逐项完成并标出对应测试；没有“仅修改了 UI 开关，业务未读取”的伪功能。配置恢复报告覆盖中途崩溃和 revision 冲突。

---

## Stage 8　错误体系、诊断、安全边界与故障降级

### 8.1 目标

使长期运行中的网络、原生能力、数据与渲染异常可理解、可定位、可恢复，同时不让日志成为正文或密钥泄露渠道。

### 8.2 涉及模块

`core::error`、`app::diagnostics`、各 service 错误映射、关于/诊断页。

### 8.3 实现任务

#### 8.3.1 错误分类

| 错误族 | 示例 code | UI 行为 | 自动恢复边界 |
|---|---|---|---|
| 取词 | NoSelection、ProtectedSource、CaptureTimeout、ClipboardConflict | 简短原因 + 手动输入 | 不反复复制、不绕过权限 |
| 配置 | NoProvider、InvalidEndpoint、HotkeyConflict、RevisionConflict | 定位到对应字段 | 不悄悄替换配置 |
| 凭据 | CredentialMissing、CredentialUnavailable | 重新配置/重试 | 无明文 fallback |
| 网络 | Offline、DnsFailure、TlsFailure、ConnectTimeout | 保留原文，手动重试 | 不关闭证书验证 |
| Provider | Unauthorized、RateLimited、UnsupportedModel、ProtocolError | 类型化说明 | 不显示完整错误 body |
| 响应 | ResponseTooLarge、StreamInterrupted、EmptyResponse | 标明部分/失败 | 不伪装完整成功 |
| 数据 | StorageBusy、DiskFull、MigrationFailed、CorruptDatabase | 禁止假报保存；提供恢复 | 不覆盖原数据库 |
| 窗口 | WindowCreateFailed、GraphicsUnavailable | 原生提示/安全关闭 | 不无限创建崩溃窗口 |
| 更新 | SignatureInvalid、HashMismatch、IncompatibleSchema、ApplyFailed | 停止安装或进入恢复 | 不绕过验签 |
| 用户控制 | Cancelled、Superseded | 正常状态，不红色报错 | 清理资源 |

错误链保留内部分类和安全上下文，但原始第三方错误不能直接 `{err:?}` 写日志；URL、header、body、文件路径先经过结构化脱敏。来自服务端的文字不作为可点击命令或富 HTML 展示。

#### 8.3.2 日志与诊断

采用结构化 tracing，默认只记录生命周期、稳定错误码、匿名本次 RequestId、耗时、状态转移和数值计数。正文、译文、完整 prompt、API key、Authorization、代理密码、剪贴板内容、窗口标题、完整用户路径全部禁止进入日志。

日志最多 **5 个文件 × 1 MiB**；异步写入使用有界缓冲，满时允许丢低优先级诊断并计数，不阻塞 UI。不得为日志额外保持无界任务。debug 模式仍不记录正文或密钥；重启默认关闭。

诊断页展示 app/version/hash、OS build、GPU/驱动可用信息、DPI、依赖锁摘要、窗口数、任务数、请求数、Private Bytes、句柄数、最近错误码与更新状态。资源实时采样仅在诊断页可见或显式 benchmark 期间运行，关闭页面立即停止，不让诊断本身提高后台 CPU。

导出诊断先展示内容清单和脱敏预览。默认不收集数据库、用户历史、密钥、内存 dump；崩溃 dump 含潜在敏感内存，不作为默认上传功能。**v1 无遥测、无后台分析服务、无自动崩溃上传**。

#### 8.3.3 长期稳定性与安全约束

所有 unsafe 集中在平台/FFI 小模块，每块说明线程、句柄、缓冲区和回调生命周期不变量；Rust panic 不得越过 extern WndProc/COM 边界。不能用 catch_unwind 宣称可以恢复访问违规、驱动崩溃或任意内存破坏。

外部文本、Provider 响应、导入的更新 manifest、ZIP 文件名都按不可信输入处理。优先拒绝超限数据，避免正则灾难回溯和无限递归解析。密钥只发给该 Provider 配置的已验证 endpoint，不在自动 fallback 中发送到另一家。

缓存上限、任务上限、线程上限与失败次数都必须明确。断网、休眠唤醒、Explorer 重启、显示器变化不会创建第二套 runtime/DB actor。状态恢复优先使用户能手动重试，不静默重放过期选区。

### 8.4 验收标准

用唯一秘密 canary 和敏感正文运行全流程，在日志、普通导出、错误弹窗、诊断包和更新参数中搜索均不可出现秘密；只有用户主动导出的业务数据允许包含所选原文/译文。所有错误有动作，不存在无解释转圈。

### 8.5 测试方法

秘密扫描、畸形响应 fuzz、panic 边界测试、磁盘故障、网络断连、窗口创建失败模拟、诊断页开关的 CPU 对比。验证 Service 恢复不重复注册热键、托盘和事件 listener。

### 8.6 完成定义

错误码目录、日志字段白名单、诊断包 schema、隐私说明已提交；UI 不暴露原始内部异常；全部高风险输入有长度限制与失败测试。

---

## Stage 9　更新发现、签名验证、下载与安装预检

### 9.1 目标

在主进程内完成“检查 → 下载 → 验证 → 等待用户安装”，未经可靠签名与完整性检查的内容绝不能执行。此阶段尚不替换正在运行的程序。

### 9.2 涉及模块

`app::update_service`、`update_core::{manifest,signature,package,policy}`、更新设置页、`xtask::release_manifest`。

### 9.3 实现任务

#### 9.3.1 更新状态与调度

```text
Disabled / Idle
  → Checking
  → UpToDate | Available | CheckFailed
  → Downloading
  → Verifying
  → ReadyToInstall
  → PreparingExit       (Stage 10)
```

后台检查只下载小型元数据，默认每 24 小时最多一次，启动后延迟 60–180 秒并加随机抖动；只安排一个下一次 timer，不每秒检查时间。无 Provider 流式响应时再进行低优先级下载；用户查询优先。自动检查失败只记录安全错误，不反复弹窗；用户手动检查必须显示结果。

默认不自动下载，不静默安装、不突然重启。界面显示版本、大小、发布日期、可信发布说明、最低要求和“下载”“取消”“安装并重启”。发布说明同样按不可信文本渲染，禁止远程图片与脚本。

#### 9.3.2 签名信任模型

采用 **Ed25519 签名 manifest + SHA-256 文件哈希 + HTTPS 传输**。SHA-256 只证明与声明一致，单独不能证明声明来自发布者。发布公钥及 `key_id` 白名单编译进主程序与 updater；私钥只存在受控发布环境，绝不放进仓库、安装包或用户配置。

采用明确 envelope，避免依赖双方“恰好相同”的 JSON canonicalization：

```json
{
  "envelope_version": 1,
  "key_id": "release-primary",
  "payload_base64": "<base64 of exact UTF-8 payload bytes>",
  "signature_base64": "<Ed25519 signature>"
}
```

签名输入为固定域分隔前缀 `LexWisp-update-v1\0` 与原始 payload bytes 的连接。先限制 envelope 大小并解析最小结构，再严格验签，再解析 payload；拒绝重复关键字段、未知 schema、不合法 UTF-8、错误签名长度。不要自行实现椭圆曲线算法，使用经维护的密码库并固定依赖。

payload 必须包含：`app_id`、`manifest_schema`、`version`、单调 `release_sequence`、`channel`、`platform`、`architecture`、`published_at`、`expires_at`、`min_os_build`、`min_updater_protocol`、`data_schema_read_min/max`、`data_schema_write_version`、`package_url`、包大小与 SHA-256、每个文件的相对路径/大小/SHA-256、发布说明摘要。

校验 app_id 精确匹配、x64/Windows/stable 匹配、SemVer 与 sequence 一致性、schema 和 updater 协议兼容。不接受来自网络的任意降级。`highest_trusted_sequence` 保存在独立更新状态，不随数据库备份回滚；同 sequence 只允许完全相同 manifest hash，不能同序号换包。

签名不解决发布私钥失窃，也不等于完整的 TUF 多角色系统。v1 使用有限静态信任根，公钥轮换通过旧信任根授权的应用版本完成；不能下载一个未受现有信任链保护的新 key 后立即信任它。系统时钟明显错误导致有效期校验失败时明确提示，不提供“忽略验签/过期继续”按钮。

#### 9.3.3 更新源与网络策略

正式更新源由 `packaging/release-policy.toml` 固定，包含已验证的 HTTPS manifest URL、允许的下载/CDN 主机列表、应用标识和公钥。**此 SPEC 不虚构尚不存在的 LexWisp 官网、仓库或签名身份**；Stage 12 必须填入实际发布者控制的值，缺失则正式发布命令失败。

更新不携带 Provider key，不读取浏览器 cookie，不允许 URL 内嵌凭据。下载 redirect 最多三次，每一跳都必须 HTTPS 且在精确允许主机清单内；不得使用 `ends_with("trusted.com")` 之类可被相似域名绕过的检查。更新链路代理可以使用已配置的普通网络模式，但不向目标站点转发代理认证 header。

#### 9.3.4 下载与包预检

在用户数据目录 `updates/<transaction-id>/` 创建当前用户专属 staging，下载到 `.part`，边下载边计数和计算哈希，不把整包放入内存。首发可不实现跨会话断点续传；失败保留有界临时文件或删除，不能将部分下载当成完整包。

初始硬上限：manifest 128 KiB、压缩包 250 MiB、解压总量 750 MiB、条目 128、解压比 100:1。实际发行包应远小于上限；超过上限必须有 release policy 升级和测试，不在客户端任意放宽。

v1 包含自包含的 `LexWisp.exe`、`LexWisp.Updater.exe` 和许可文本；图标、UI assets 尽量内嵌。文件路径使用固定白名单，拒绝额外可执行文件、脚本、DLL 搜索劫持载荷。不得执行包里的安装脚本。

ZIP 校验拒绝：`..`、绝对路径、UNC、盘符、冒号/ADS、保留设备名、末尾空格/点、大小写归一后重复路径、符号链接、硬链接描述、reparse point、超限内容与嵌套炸弹。展开时路径仍在 staging 根内，创建/打开目标时避免跟随重解析点；只做一次字符串 canonicalize 不能抵御后续路径替换。

逐个验证文件大小和哈希，校验 PE 平台架构、版本元数据及正式包 Authenticode 信任结果。`WinVerifyTrust` 的成功返回是零值，不能把它误当普通 HRESULT 用 `SUCCEEDED` 判断；始终关闭验证状态句柄并限制信任验证 UI。[S33]

Authenticode 用于 Windows 发布身份与附加完整性校验；**manifest 根密钥签名才是本应用更新授权的核心依据**。签名证书有效但不属于被授权的包仍不能安装。离线链验证或吊销信息不可用时不得写成“已确认未吊销”，按发布策略明确失败原因。

#### 9.3.5 可安装预检

准备安装前验证：安装根属于本次安装且可写、非系统目录、主程序与 updater 路径正确、下载与备份空间充足、无其他更新事务、数据库 schema 在兼容范围、存在一致性备份方案、当前 updater protocol 可理解此次包。

磁盘可用空间至少覆盖新展开包、旧版本备份、数据库备份和 journal 安全余量；按实际文件大小计算，而非固定猜一个值。备份/替换文件必须在允许的本地卷布局内；不支持跨卷原子替换就拒绝该布局，不静默退回不安全 copy/delete。

### 9.4 验收标准

可展示更新、取消下载、重新检查并形成 ReadyToInstall；错误签名、换包、重复序号不同内容、过期、平台不匹配、路径穿越、ZIP 炸弹、空间不足均不能进入可安装状态。无真实发布源时，使用受控测试签名环境验证，正式通道仍保持构建门禁。

### 9.5 测试方法

测试公钥与正式公钥分离；搭建本地测试源，覆盖 redirect、网络中断、错误 Content-Length、哈希不符、签名不符、时钟异常和所有路径攻击 fixture。断言下载期间没有启动任何包内 exe。

### 9.6 完成定义

更新 manifest schema、签名命令、验证代码、下载状态机和恶意包测试齐全；正式构建不能接受测试 key，正式更新源配置缺失时不能伪装已接入更新。

---

## Stage 10　独立 Updater、文件替换、故障恢复与重启

### 10.1 目标

交付真正可工作的 `LexWisp.Updater.exe`：在主程序正常退出后替换文件，失败时恢复旧版本，并通过健康握手保证数据库迁移与二进制版本一致。

### 10.2 涉及模块

`apps/updater`、`update_core::{journal,apply,recovery,health}`、`app::{bootstrap,shutdown,update_service}`。

### 10.3 实现任务

#### 10.3.1 Helper 运行方式与权限

主程序使用**当前已安装且可信的 updater**，先复制到当前事务 staging 后执行这份副本。这样更新包中的新 `LexWisp.Updater.exe` 可被替换，执行中的 helper 自身不被覆盖。复制后校验哈希和身份；不得直接执行尚未验证的新包 updater 来“让它验证自己”。

helper 使用当前用户权限，不常驻、不安装为服务、不弹 PowerShell 或 cmd 窗口。它不依赖 GPUI、Provider、浏览器、外部解释器，也不从网络再下载未知代码。参数仅包含事务标识和受控根定位信息；计划从已验证文件读取，不能允许调用者指定任意需要覆盖的目标路径。

staging、backup、journal 与恢复 helper 的 ACL 仅开放必要的当前用户/系统权限；不能使用任意用户可写的共享临时目录。保护边界不包含同权限恶意软件或管理员完全控制机器的情况，不声称以 ACL 防御所有本机攻击。

#### 10.3.2 主程序退出前握手

1. 用户明确点击“安装并重启”。先处理未保存草稿；拒绝新的查询和数据编辑，取消活动 Provider 与取词，刷新已确认写入。
2. 主程序生成 update transaction，保存完整已签 manifest 和本地安装状态，创建受控同步事件/父进程句柄。helper 独立重验 manifest、载荷与目标路径，获得更新互斥锁，回报 `Prepared`。
3. helper 在 Prepared 前不改安装文件。主程序等待准备失败时仍可恢复正常使用，不先自杀再盼 helper 成功。
4. 主程序完成 SQLite checkpoint、关闭连接并确认无后台写入，向 helper 发送 `DatabaseClosed`，正常退出。helper 等待**已验证的主进程句柄**变为 signaled，不单靠 PID 轮询，避免 PID 复用。
5. 原主程序不退出或 DB 未关闭时，helper 有界等待后中止更新并保留可用旧版；不能按进程名强杀所有 LexWisp，更不能杀其他用户进程。

整个同步协议采用显式版本与随机 nonce；继承句柄只开放所需句柄，不把 DB、文件、网络或凭据句柄全部继承给子进程。主程序和 helper 必须验证双方属于同一安装与当前事务。

#### 10.3.3 并发启动保护

helper 持有独立更新锁直至提交或完成恢复。普通主程序启动在初始化 DB/GPUI 前检查锁与未完成 journal；正在更新时立即退出或显示最小维护提示，不能长时间驻留并锁住待替换主 exe。单实例锁与更新锁的顺序固定，避免死锁。

操作系统加载新实例本身可能短暂占用 exe，因此文件替换必须对 sharing violation 做有界重试；不能承诺仅凭 mutex 实现文件系统层的绝对原子多文件切换。不能把“等待更新完成”实现成启动旧 exe 后无限等待，从而反过来阻止替换。

#### 10.3.4 持久化事务日志

journal 是独立文件，含协议版本、事务 ID、旧/新版本和 manifest hash、允许根目录、文件列表、每文件旧/新 hash、备份路径、数据库备份信息、步骤状态和健康结果。写入采用临时文件 + flush + 原子替换；关键步骤前记录 intent，完成后记录完成事实。

```text
Prepared
  → MainExited
  → BackupReady
  → Replacing
  → FilesInstalled
  → CandidateStarted
  → CandidateReady
  → Committed
  → CleanupPending / Done

任一可恢复失败
  → RollbackIntent
  → BinaryRestored
  → DatabaseRestored (仅限尚未放行用户写入的候选启动)
  → RolledBack

无法自动恢复
  → RecoveryRequired (保留全部证据，不启动半安装版本)
```

断电可能发生在“文件已换但 journal 还未记完成”的缝隙，所以恢复必须检查文件实际 hash，将旧文件、新文件、备份分别与已签清单比对，不单信上一次状态字符串。

#### 10.3.5 文件替换与备份

主程序退出后先备份旧版允许列表文件，验证备份完整性；为数据库创建一致性备份并记录其 schema/hash。旧可执行文件始终保留一个已验证副本。

每文件采用同卷 `ReplaceFileW` 或符合该 API 前提的原子替换操作，逐项 journal；不先删除旧 exe 再慢慢 copy 新 exe。相关 API 有明确条件和错误语义，不是整个多文件安装的原子事务。[S32]

替换顺序优先新 updater 与非关键静态文件，**主 exe 最后**。首发尽量两个自包含 exe 加许可文件，避免一个尚未启动的主 exe 因依赖 DLL 已被换成不兼容版本而无法进入恢复入口。禁止修改 data、用户导出和无关文件。

杀毒软件/索引器造成 sharing violation 时指数退避重试，总计最多 15 秒；失败进入恢复，不无限等。若备份验证失败、空间变化或文件 hash 异常，在破坏现有可运行版本之前中止。

#### 10.3.6 数据库迁移与健康握手

**二进制回滚不等于数据库回滚。** 必须同时实现以下合同，否则 updater 不算完成：

- 更新前保留一致性 DB 备份，并验证旧版的读取范围与新版写 schema 的关系。v1 发布链优先兼容 N/N-1；不兼容破坏性迁移不能依靠“更新失败后换回 exe”解决。
- helper 以 `--update-validate <transaction>` 启动新主程序。候选模式暂不注册可操作热键、不开放用户写入、不自动发 Provider 请求、不执行自动清理，不让用户在健康确认前形成新数据。
- 新主程序完成配置加载、迁移、数据库 quick_check、关键查询自检、GPUI/原生初始化及必要的最小窗口创建/销毁探针，再发 `CandidateReady`。这能发现部分启动问题，但不承诺预测所有未来崩溃。
- helper 通过自己创建的子进程句柄和事件等待，健康验证总期限 60 秒；候选须短暂存活并无明确错误。成功后先持久化 `Committed`，再发 `CommitGranted`，主程序才开放正常 UI/写入。
- 候选验证失败时先要求其关闭。只允许在有界等待后终止**helper 自己为本事务启动且身份验证过的候选子进程**，不得强杀原用户主进程或同名其他进程。确认候选结束、DB 句柄关闭后，恢复二进制和更新前 DB；正确处理 WAL/SHM，不对仍打开的库删除文件。
- 一旦 `CommitGranted` 后允许用户正常写入，就禁止自动用旧 DB 快照覆盖当前数据。此后回到旧二进制必须满足其 schema 可读合同；不满足时进入维护恢复，而不是静默丢掉新收藏。

候选启动过程不修改凭据、不重写用户启动偏好；本来无密码的更新验证不应触发秘密外发。失败恢复后可以重启旧主程序，并显示“更新失败，已恢复版本 X”的安全原因。

#### 10.3.7 崩溃与断电后的恢复入口

正常启动最早阶段扫描未完成 journal，在任何数据库迁移或业务写入前进入恢复。使用事务 staging 中已校验的恢复 helper，或当前可验证 updater；不存在可信 helper 时原生提示重新运行官方安装器修复，保留所有数据与备份。

恢复按实际文件 hash、journal 与候选提交状态决定继续安装或恢复，不盲目“总是重新执行下一步”。每一步必须幂等；重复执行恢复不能删除已恢复备份或重复覆盖用户数据。

如果已提交但 `CommitGranted` 信号因崩溃未到达，新主程序通过验证持久化提交状态判断可正常启动，不能反复回滚。更新独立的高水位序号不随数据恢复降低；内部恢复允许使用本次事务的已验证旧版本，不作为对网络降级包的通用许可。

#### 10.3.8 清理与卸载交互

helper 提交/恢复后退出；不每次开机常驻检查。保留最近一个已验证旧版本和 DB 备份最多 7 天，实际清理在后续启动/维护空闲时有界执行；这些备份可能含历史正文，设置页说明占用和隐私，并提供显式清理。

不删除当前未完成事务需要的文件。卸载先确认没有活动更新锁，必要时明确等待或取消；卸载脚本不能在 helper 工作时删除 staging。失败日志仅记录路径的安装相对名、步骤和错误码，不含用户正文或秘密。

### 10.4 验收标准

从 N 到 N+1 可安装并重启；旧主进程未退出时不覆盖；包内新 updater 能被替换；更新中断可恢复；候选迁移失败后旧版和旧 DB 可重新打开；提交后不会自动恢复旧 DB 丢失用户数据。

### 10.5 测试方法

对**每个 journal 转移和每个文件替换前后**注入终止，重启验证；覆盖断电模拟、磁盘满、只读目录、文件占用、备份损坏、错误 PID/句柄、候选超时、候选崩溃、数据库迁移失败、健康信号丢失、双重启动和恢复重复执行。使用真实 Windows 文件锁，不仅测试内存 mock filesystem。

### 10.6 完成定义

helper 独立构建且不含 GPUI 依赖；`docs/release/update-protocol.md` 和恢复说明齐全；故障矩阵所有强制用例通过；不以“正常升级成功一次”替代恢复能力。

---

## Stage 11　系统测试、性能预算与长期运行验收

### 11.1 目标

把“轻量、快速、稳定”落实为可重复测量的发布门槛。本阶段是综合验证与优化，不是此前阶段第一次编写测试。

### 11.2 涉及模块

`tests/*`、`tools/benchmark`、`xtask::{test,bench,soak}`、性能问题涉及的生产模块。

### 11.3 实现任务

#### 11.3.1 测试分层

| 层次 | 工具与对象 | 必须验证 | 不能替代 |
|---|---|---|---|
| 纯 Rust 单元 | 普通 `#[test]`、属性测试 | reducer、校验、排序、状态转移、路径限制 | 原生窗口行为 |
| Service/Repository 集成 | 临时 SQLite、HTTP fixture、假时钟 | 真实数据与协议、取消、epoch、迁移 | 真实 Provider 兼容 smoke |
| Kit 无头 UI | `#[gpui_kit::test]`、TestAppContext、测试扩展 | 生产视图上的点击、键盘、焦点、布局、禁用态 | Windows 桌面交互与像素 |
| Windows 原生 E2E | 交互式 Windows runner、UIA、测试宿主 | 热键、剪贴板、托盘、非激活、DPI、退出 | 长期资源趋势 |
| 真机视觉与可用性 | 截图、键盘、鼠标、IME、Narrator | 选区、CJK、对比度、窗口边界 | 业务协议覆盖 |
| 更新故障注入 | 真实文件系统、子进程、步骤 failpoint | 替换、断电恢复、数据库兼容 | 正式签名/分发配置 |
| 性能与 soak | Release + 进程/GPU计数器 | 延迟、CPU、内存、泄漏趋势 | 单元正确性 |

Kit 测试必须通过实际生产 View 的事件入口执行，不能直接调用内部 save 方法后声称按钮 E2E 通过。测试 ID 稳定且具业务含义；使用 `find/within/click/input/press` 等当前测试能力时依锁定文档验证签名。普通 Rust 测试中避免无差别 `use gpui_kit::*` 导致测试宏混淆。[S11]

无头 snapshot 不应被解释为 Windows GPU 像素截图。官方 Metal 测试能力不证明 Windows 渲染正确；原生 E2E 在拥有可交互桌面的 runner 上执行，不能用 Windows Service 的 Session 0 环境冒充用户桌面。若 GPUI 可访问性节点不足以定位某个控件，应补充可访问性属性和限定测试辅助，不能仅靠脆弱屏幕坐标。

#### 11.3.2 性能测试条件

参考档位：Windows 11 x64、至少 4 个物理核心、16 GiB RAM、SSD、Intel 同档集显，1920×1080、100% 缩放；另测 150%/200% 和独显。具体机器型号、驱动、OS build、供电模式和所有样本写入报告。不得只在高端台式机上取一个最好值。

全部测量使用 Release、相同 Cargo.lock、同一构建 hash，关闭 inspector、debug overlay、测试 instrumentation 与非必要日志；正式基准使用外部测量器，不能把诊断页开启的持续采样混入普通后台数据。

内存分别报告 **Private Bytes/PrivateUsage、Working Set、GPU dedicated/shared memory**；不能把任务管理器被换页压低的 Working Set 当作进程实际承诺内存。Windows 官方 API 对私有提交与工作集有不同定义。[S35]

CPU 以“单核等效占用”统一口径：`100 × (进程 user time 增量 + kernel time 增量) / 墙钟时长`。多核累计时间不能再与任务管理器按总核心数归一的数值混为一谈。[S36]

#### 11.3.3 目标与发布门槛

以下全部是**待实测验证的产品预算**。参考机器上硬门槛失败就不得以“轻量版已完成”发布；确有框架下限时需给出基线、原因和明确的需求变更，而不是悄悄修改数字。

| 指标 | 优化目标 | 发布门槛 / 判定条件 |
|---|---|---|
| 冷启动至热键/托盘可用 | ≤800 ms | P95 ≤1500 ms；不含首次配置交互 |
| 后台新启动，尚未打开业务窗 | Private Bytes ≤80 MiB | 稳定后 ≤120 MiB |
| 曾使用 Popup/主窗，全部关闭并超过 TTL | Private Bytes ≤100 MiB | 空闲 10 分钟后 ≤160 MiB |
| 单 Popup 展示 4 KiB 结果 | Private Bytes ≤150 MiB | ≤220 MiB；GPU另报 |
| 主窗口含 10,000 条历史的分页视图 | Private Bytes ≤200 MiB | ≤280 MiB，不全量加载正文 |
| 后台 CPU，10 分钟无交互 | 单核等效 ≤0.1% | ≤0.3%；不包含明确更新下载时段 |
| 冷 Popup，取词完成→首个实际可见帧 | P95 ≤180 ms | P95 ≤300 ms |
| 暖 Popup，取词完成→首个实际可见帧 | P95 ≤40 ms | P95 ≤80 ms |
| 普通响应 UIA 来源，热键→loading首帧 | P95 ≤250 ms | P95 ≤500 ms；回退路径单列 |
| 本机 mock 首 chunk 到 UI 可见 | P95 ≤50 ms | P95 ≤100 ms |
| 本地取消可见确认 | P95 ≤50 ms | P95 ≤100 ms |
| 取消后本应用请求槽释放 | ≤100 ms | ≤250 ms；第三方无法中止的阻塞单列 |
| 历史首屏，10,000 条 | P95 ≤80 ms | P95 ≤150 ms，排除首次迁移 |
| 关键 DB 保存，正常 SSD | P95 ≤30 ms | P95 ≤100 ms；失败仍正确处理 |
| 1,000 次完整窗口周期后的空闲增长 | ≤8 MiB | ≤16 MiB，且后半程没有持续正增长趋势 |
| 24小时 soak 后资源增长 | ≤16 MiB | ≤24 MiB；无活动请求时任务/窗口应回到基线 |
| 常规安装退出 | ≤1秒 | ≤3秒；已确认写入不丢失 |
| 用户发行包大小 | 主 exe 尽量 ≤40 MiB，安装包 ≤60 MiB | 单项偏离需依赖体积报告；不是通过牺牲正确性强压体积 |

第三方网络耗时不作为本地首绘承诺。取词总耗时分别报告 UIA、剪贴板回退、拒绝/超时；不能从总体样本里删除慢路径再宣称全部取词 P95 达标。首帧采用可见呈现的观测或可靠渲染事件，不能用“调用了 open_window”时间冒充用户看到结果。

#### 11.3.4 性能测试场景

至少 30 次新进程冷启动、100 次冷 Popup、1,000 次暖显示，报告 P50/P95/P99、原始样本与失败数。冷 Popup 指窗口确已销毁；GPUI 全局已初始化与整个进程冷启动要分开统计。

窗口泄漏测试使用 TTL=0 执行 1,000 次真实销毁；另用默认 30 秒做真实到期测试，并以假时钟覆盖更多竞态，不能把假时钟测试记为真实释放性能。句柄、GDI/USER 对象、线程、订阅、任务与窗口数量应回到合理稳定基线，少量一次性缓存增长须解释，不能按循环累加。

24 小时 soak 使用确定性本机 Provider：查询、取消、错误、主窗口开关、搜索、收藏、主题切换、断网、休眠/恢复、Explorer 重启组合。不中断真实用户工作，不调用付费 Provider 数千次。活动记录与日志都按限制清理。

额外压力测试：64 KiB 输入、512 KiB 输出、持续小 chunk、快速滚动选择、10,000 条收藏、50,000 条历史上限配置。达到边界时应拒绝/截断为显式非成功状态，而不是 OOM 或 UI 长时间冻结。

#### 11.3.5 优化顺序

先修无界状态、强引用环、未取消任务、隐藏动画、重复 runtime、全量列表、过频通知，再优化字体/Markdown缓存、连接池和打包体积。最后才考虑分配器、编译参数或上游局部 patch。禁止用 `EmptyWorkingSet`、周期性 trim、每次关闭重启主进程或夸大的压缩数字掩盖资源问题。

调优 GPUI 时先定位应用层可控行为；不得为几十 KiB 节省重写一套组件库。窗口释放不能承诺卸载所有驱动/字体全局缓存，报告明确区分应用泄漏与可复现的平台基线。

### 11.4 验收标准

所有强制功能测试和更新故障测试通过；参考机器上的性能硬门槛有原始数据；24 小时运行无未处理崩溃、持续增长、旧请求污染或数据丢失。未达标项目必须修复，不能隐藏在平均数里。

### 11.5 测试方法

`cargo xtask test --suite all`、`cargo xtask bench --scenario popup`、`cargo xtask bench --scenario idle`、`cargo xtask soak --hours 24`。这些是本工程必须实现的命令合同，不是已经存在的外部工具；命令执行真实测试并输出结构化 JSON，遇到缺少交互桌面等前提以非零状态失败，不能默认为通过。

### 11.6 完成定义

提交 `docs/performance/release-candidate.md`、原始 JSON/CSV、真机截图与完整用例矩阵。关键 reducer、路径验签和更新状态转移必须覆盖正反路径；覆盖率数字仅作辅助，不替代场景门禁。

---

## Stage 12　依赖控制、Release 配置、安装包与发布流水线

### 12.1 目标

产出无需开发环境即可运行的正式分发包，建立可审计、可复现配置、可验证签名的发布流程。

### 12.2 涉及模块

Workspace 依赖与 profiles、`packaging/*`、`xtask::package/release`、CI、用户与开发者文档。

### 12.3 实现任务

#### 12.3.1 依赖选择与准入

| 用途 | 采用依赖 / 能力 | 范围控制 |
|---|---|---|
| UI | `gpui-kit = "=0.6.1"`，锁定兼容 pre 家族 | 生产仅 component/assets 等必要功能；测试图单独启用 test-support |
| Windows API | `windows` crate，与可用平台版本协调 | 仅启用实际 Win32 namespace；统一包装错误与句柄 |
| async 网络运行时 | `tokio` | 一个 runtime；rt/net/time/sync 及实际宏所需 features |
| 取消 | `tokio-util::CancellationToken` | 仅启用其所需 features，不自制不可靠 bool 取消 |
| HTTP/TLS | `reqwest` + Rustls 验证路径 | JSON、stream、所需代理；关闭不必要默认功能 |
| SQLite | `rusqlite` bundled + backup 所需功能 | 单连接 actor；不用 ORM/连接池 |
| DTO/配置 | `serde`、`serde_json` | 强类型、版本字段、限制大小 |
| ID与URL | `uuid`、`url`、`semver` | 使用结构化解析，禁止手写地址切割 |
| 错误 | `thiserror`，内部边界可用 `anyhow` | UI 暴露类型化安全错误，不直接输出原始链 |
| 秘密 | `zeroize` / `secrecy` | 明确生命周期；系统凭据为持久化后端 |
| 日志 | `tracing` + 轻量输出层 | 有界、轮转、字段白名单 |
| 更新密码学 | `ed25519-dalek`、`sha2`、`base64` | 标准算法；严格校验；无自制密码实现 |
| 包展开 | `zip` 的必要压缩算法 | 禁用不需要格式/算法；实施路径与大小限制 |
| 测试 | `proptest`、`tempfile`、协议 fixture server | 不随正式 exe 分发 |
| 构建资源 | Win32 resource 编译工具或小型 build helper | 固定来源；图标/manifest/版本信息可审计 |
| 安装器 | **Inno Setup**，固定已验证版本 | 当前用户安装；`PrivilegesRequired=lowest` [S37] |

除上述已核实的 UI 基线外，普通依赖补丁版本由 Stage 0/12 对照实际已发布版本、安全公告、MSRV、许可证后锁定，并提交锁文件；不得把未核实的“最新版本号”写入清单。Cargo.lock 是可执行精确版本证据，表格是技术选择，不是完整依赖锁的替代物。

必须检查重复的 `gpui` 类型、不同来源的 pre 家族、HTTP/TLS 栈、Windows bindings、大型 parser/字体/assets。Kit 本身的传递依赖不能凭想象消失；只关闭有文档支持且实测无损的功能，保留 `cargo tree` 前后证据。禁止因依赖优化删除必要输入法、可访问性或选择能力。

#### 12.3.2 Release 配置

起始配置如下；以性能/体积测量决定后续微调，但不能使用不兼容用户机器的指令集。

```toml
[workspace]
resolver = "2"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "unwind"
debug = 1
strip = "none"
incremental = false
```

这些选项的语义按 Cargo 官方 profile 定义；优化级别不是必然更快/更小，应实测选择。[S31] Windows 构建保留对应 PDB 供内部排障，但不随安装包分发；避免对成品二次处理破坏签名或符号匹配。构建路径做 remap，检查 PE/PDB 引用没有泄露开发机用户名。

采用 `x86_64-pc-windows-msvc`，在依赖支持且 Stage 0 构建验证通过的前提下统一 `+crt-static` 并确认原生 C/C++ 库 CRT 链接一致，使首发安装不依赖用户自行安装开发工具。若实际依赖无法静态统一，必须明确列出并由安装器处理精确运行库前置条件，不能发布仅在开发机能跑的包。

禁止 `target-cpu=native`；禁止为了体积启用 UPX 或其他容易影响签名/杀毒和启动行为的运行时壳。首发不启用未经测试的 PGO；优化必须保持可调试性和可靠性。

两个 exe 都使用 Windows GUI subsystem，正常运行无控制台闪烁。嵌入 app/version/company（真实发行者）、图标与 manifest：`asInvoker`、PerMonitorV2 DPI、正确兼容声明、longPathAware。应用标识固定，用户可见名称为 LexWisp。

#### 12.3.3 安装、修复、卸载

Inno Setup 默认安装到当前用户 `%LOCALAPPDATA%\Programs\LexWisp`，不索取管理员权限；创建开始菜单项与卸载注册信息。桌面快捷方式与登录启动默认不勾选，避免安装器与设置页保存的意愿冲突。安装器不得写入 Provider key。

修复安装先协调关闭本安装主程序及更新事务，不按 exe 名全局杀进程。不会覆盖用户数据库与配置；处理不完整安装时提供日志和明确结果。

默认卸载移除程序、快捷方式、托盘残留与本应用启动项，保留用户数据和凭据；界面提供明确的“同时删除个人数据与保存的凭据”选项。只有用户选中才清理本安装命名空间，不能删除其他应用的凭据。卸载完成后的保留数据位置写清楚。

首发官方安装包、主 exe、updater 均执行 Authenticode 签名并时间戳；签名后的二进制再计算 package 与 manifest 哈希。外层安装包签名在最终组包后进行，顺序不可颠倒。公共分发应说明 SmartScreen 等信誉结果并非仅凭签名即可保证不提示。

#### 12.3.4 CI 与发布供应链

PR 流水线：格式检查 → clippy → core/协议/数据测试 → Windows headless UI → Release 编译 → 安全与许可证扫描。交互式 Windows 原生 E2E 与性能走受控 runner；从 fork 的不可信 PR 不能获得签名凭据或控制有秘密的 runner。

发布流水线：受保护 tag → 锁定构建环境/依赖 → 全套测试 → 两 exe 构建 → 安全/许可证/SBOM → exe 签名 → 生成并验签 payload → 安装器打包与签名 → 干净 VM 安装/升级验证 → 人工授权发布真实 stable 元数据。

工具链安装与下载固定版本并验证来源/校验值；不用未审查的 `curl | sh` 或远程 `Invoke-Expression` 建立受信构建机。CI action 固定到可审计 revision，权限最小化，签名任务与普通构建隔离。

**必需但不能由 Agent 虚构的发布输入**：发行者身份、证书/签名服务权限、Ed25519 私钥或签名服务、实际更新源 URL、公钥和下载主机清单。缺失时允许生成显式标记的开发包并通过测试 key 演练，但 `release --channel stable` 必须失败；开发包不能冒充正式可发布签名包。Agent 应完成全部代码和配置校验，并准确列出剩余外部授权，不生成假证书或假部署结果。

#### 12.3.5 标准工程命令

必须实现并在文档解释以下命令；外层 xtask 负责参数校验、前提检查、组合执行与报告，不包装成返回恒为成功的脚本。

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p lexwisp -p lexwisp-updater --release --locked
cargo xtask test --suite core
cargo xtask test --suite providers
cargo xtask test --suite storage
cargo xtask test --suite ui
cargo xtask test --suite windows-e2e
cargo xtask test --suite updater
cargo xtask test --suite all
cargo xtask bench --scenario popup
cargo xtask bench --scenario idle
cargo xtask soak --hours 24
cargo xtask package --channel dev
cargo xtask release --channel stable
```

不要用 `--all-features` 构建正式产品，避免启用 inspector/test-support/WebView 等无关模块。测试构建通过 features 引入的依赖与生产 Release 图分别检查。

### 12.4 验收标准

干净 Windows 机器无 Rust/VS/Node/Python 时能安装运行；两个 exe 无控制台；用户无需改配置文件；更新、修复、卸载可执行；安装包不包含 PDB、测试 key、测试服务器、开发路径、Provider secret 或无关 assets。

### 12.5 测试方法

Windows 11 clean VM 安装、普通用户权限、中文路径、无网络、代理、安装后查询、N→N+1、修复、卸载保留/删除数据两条路径。核验签名链和 manifest，扫描包内容与依赖，重跑性能 smoke。

### 12.6 完成定义

产出开发与正式发布路径，SBOM、许可清单、签名/更新手册、安装卸载测试证据齐全。正式外部输入齐备时生成可分发 signed artifacts；输入缺失时其状态明确为发布阻塞，不能填“已发布”。

---

## Stage 13　发布候选验收与交付

### 13.1 目标

确保交付的是完整工具，而不是若干独立演示的组合。所有核心功能须通过同一个正式配置与安装版本验证。

### 13.2 涉及模块

全工程、安装包、更新源、测试报告、用户帮助与发行说明。

### 13.3 实现任务

#### 13.3.1 一条不可拆散的端到端发布路径

新用户安装 → 首次引导与隐私确认 → 添加 Provider 和密钥 → 测试连接 → 设置默认 Provider → 在外部应用选中文字 → 全局热键 → 非激活 Popup → 选择复制 → 收藏 → 刷新并取消 → 关闭 Popup → 30 秒后验证实际销毁 → 打开历史与收藏 → 修改热键/主题/TTL → 重启验证设置 → 打开登录启动 → 安装签名更新 → 验证数据不丢 → 卸载与数据保留测试。

其中不能通过手工修改 SQLite、配置文件、环境变量或源码跳过产品缺失步骤。演示记录必须标明哪些请求使用真实 Provider，哪些使用 mock 做故障注入。

#### 13.3.2 功能可追踪矩阵

| 需求 ID | 用户需求 | 实现阶段 | 必需验收证据 |
|---|---|---|---|
| F01 | 全局快捷键取词 | 1、3、7 | 冲突/重绑/来源稳定/无选区测试 |
| F02 | 多 Provider 与 API 配置 | 2、4、7 | 两 adapter、密钥隔离、真实 smoke |
| F03 | 鼠标附近 Popup | 0、5 | 冷暖首绘、DPI、非激活定位 |
| F04 | 结果文字选择复制 | 0、5 | 中文/跨段/流式选择、Ctrl+C |
| F05 | 收藏与刷新 | 2、4、5、6 | 快照独立、刷新新请求、取消 |
| F06 | 历史与收藏管理 | 2、6 | 分页、搜索、删除、备注、导出 |
| F07 | 完整设置与主题 | 7 | 所有字段重启一致、失败回滚 |
| F08 | 登录启动与托盘 | 1、7、12 | 普通权限、Explorer重启、卸载 |
| F09 | 持久化与安全凭据 | 2、8 | DB恢复、秘密扫描、凭据补偿 |
| F10 | 独立 Updater | 9、10、12 | 验签、真实替换、每步故障恢复 |
| F11 | 低空闲内存与CPU | 0、1、5、11 | Private Bytes、GPU、CPU原始数据 |
| F12 | 并发取消与状态管理 | 1、4、5、11 | generation、epoch、队列与取消竞态 |
| F13 | 诊断与长期稳定 | 8、11 | 脱敏导出、24小时soak |
| F14 | 可发布安装包 | 12、13 | 干净机器、签名、升级、卸载 |

#### 13.3.3 最终产物

发布归档必须包含 `LexWisp.exe`、`LexWisp.Updater.exe`、签名安装包、更新 ZIP、签名 manifest、SHA-256 清单、SBOM、第三方许可、用户快速上手、隐私说明、Provider 配置说明、更新恢复手册、开发构建说明与发布说明。符号文件存内部归档，不面向普通用户打包。

仓库中交付完整源码、Cargo.lock、工具链版本、migration、测试 fixture、CI、安装器脚本、签名输入校验逻辑和本 SPEC 的落实记录。不得提交真实 key、真实用户数据库、剪贴板采样或个人聊天内容。

#### 13.3.4 Coding Agent 阶段报告格式

每阶段提交一份简短报告，至少包含：提交 hash、完成模块、实际运行命令、测试数量与失败情况、Windows 环境、性能样本位置、残留问题、ADR 变更。测试没有运行时写“未执行”及原因，不能写“应当通过”替代结果。

连续实施时，以本 SPEC 为默认决策完成可完成的开发工作，不为已明确的默认值反复询问。涉及证书、真实 API 付费授权、生产更新源发布等外部权限时，保留清晰的执行门禁和所需输入；不可通过伪造凭据或降低验证来跨越。

### 13.4 验收标准

F01–F14 均有对应代码和证据；不存在阻塞级数据丢失、安全、无法退出、持续泄漏、选择复制失效或不可恢复更新问题。发布说明准确列出兼容限制，不宣称“支持所有应用”“永不占内存”“取消绝不计费”等无法保证的能力。

### 13.5 测试方法

使用正式签名候选包完整执行 13.3.1 路径，复跑 Stage 11 硬门槛与 Stage 10 故障矩阵中的关键样例；比对最终发行包 hash 与测试包，确保测试的是要发布的同一个产物。

### 13.6 完成定义

只有功能、数据安全、更新恢复、真实 Windows 性能与正式分发链均达到门禁，才可标记 **Release Ready**。单纯代码编译通过、静态 UI 好看、Mock 可运行或安装包能生成，均不构成最终完成。

---

## 附录 A　一手资料与核查索引

**核查日期统一为 2026-09-16。** 在线文档可能继续变化；实施时优先读取本 SPEC 锁定版本的源码与 Cargo.lock。下列引用支持相应框架/API 事实，不为 LexWisp 的设计预算提供已经实现或已经测量的背书。

| 引用 | 官方资料 / 版本 | 本文主要核查用途 |
|---|---|---|
| [S01] | longbridge/gpui-kit Releases | v0.6.0 命名/组件变化，v0.6.1 发布与测试能力 |
| [S02] | gpui-kit v0.6.1 根 Cargo.toml | Kit 与 gpui-pre 的版本声明区别 |
| [S03] | gpui-pre-platform 0.3.5 发布包 | 2026-09-14 快照与平台依赖家族 |
| [S04] | gpui-kit v0.6.1 门面 Cargo.toml / [lib.rs](https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/crates/kit/src/lib.rs) | 默认 feature、component/assets/test-support 与 re-export |
| [S05] | GPUI Kit Getting Started | 初始化、Root、窗口与覆盖层结构 |
| [S06] | v0.6.1 text_selection example | 既有 TextView 选择与跨区域文本交互 |
| [S07] | gpui-pre-windows 0.3.5 Windows 源码 | DirectWrite、DirectX renderer；具体 [设备源码](https://docs.rs/crate/gpui-pre-windows/0.3.5/source/src/directx_devices.rs) |
| [S08] | gpui-pre 0.3.5 app.rs | Explicit quit mode、最后窗口移除语义 |
| [S09] | gpui-pre-windows 0.3.5 platform.rs | Windows 应用级 hide 与激活的实际实现 |
| [S10] | gpui-pre-windows 0.3.5 window.rs | 窗口激活路径与模拟输入副作用 |
| [S11] | GPUI Kit Test 文档 | 无头 UI 测试边界、查找与输入、snapshot |
| [S12] | GPUI Kit Installation | Windows 构建前提与安装建议 |
| [S13] | v0.6.1 root_borderless example | 无边框窗口和 Root 样例 |
| [S14] | v0.6.1 stream-markdown example | TextViewState 增量更新与样例任务结构 |
| [S15] | Tokio runtime Builder 官方文档 | current-thread runtime 与 blocking pool 配置 |
| [S16] | Microsoft UI Automation Threading Issues | UIA 客户端线程与 COM apartment 要求 |
| [S17] | GPUI Kit Coding Guides | Entity/Subscription 生命周期、业务模块与增量 UI |
| [S18] | Microsoft Notification Area | 托盘通知区 API 与交互 |
| [S19] | SQLite Write-Ahead Logging | WAL、检查点与连接行为 |
| [S20] | SQLite Online Backup API | 一致性数据库备份 |
| [S21] | Microsoft CredWriteW | Windows Credential Manager 写入接口 |
| [S22] | Microsoft RegisterHotKey | 热键注册、消息和限制 |
| [S23] | Microsoft IUIAutomationTextPattern::GetSelection | 选区、多个范围与空选区 |
| [S24] | Microsoft IUIAutomation2 timeout properties | [ConnectionTimeout](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation2-put_connectiontimeout) 与 [TransactionTimeout](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation2-put_transactiontimeout) |
| [S25] | Microsoft SendInput | 输入注入与 UIPI 完整性级别限制 |
| [S26] | Microsoft GetClipboardSequenceNumber | 剪贴板变化序列语义 |
| [S27] | Microsoft High DPI Desktop Application Development | DPI 感知、坐标与显示器变化 |
| [S28] | Microsoft SetForegroundWindow | 前台窗口激活限制 |
| [S29] | OpenAI Chat API Reference | Chat Completions 请求与响应协议 |
| [S30] | DeepL Translate Text | /v2/translate、认证、语言与文本字段 |
| [S31] | The Cargo Book / Profiles | Release profile 选项及优化取舍 |
| [S32] | Microsoft ReplaceFileW | 文件替换前提与错误语义 |
| [S33] | Microsoft WinVerifyTrust | 信任验证与返回值判定 |
| [S34] | Microsoft Run and RunOnce Registry Keys | 当前用户登录启动项与系统执行限制 |
| [S35] | Microsoft PROCESS_MEMORY_COUNTERS_EX | PrivateUsage / WorkingSetSize 的区别 |
| [S36] | Microsoft GetProcessTimes | 跨线程累计 user/kernel CPU 时间 |
| [S37] | Inno Setup / PrivilegesRequired | lowest 非管理员安装模式 |

[S01]: https://github.com/longbridge/gpui-kit/releases
[S02]: https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/Cargo.toml
[S03]: https://docs.rs/crate/gpui-pre-platform/0.3.5
[S04]: https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/crates/kit/Cargo.toml
[S05]: https://gpui-kit.com/docs/getting-started/
[S06]: https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/examples/text_selection/src/main.rs
[S07]: https://docs.rs/crate/gpui-pre-windows/0.3.5/source/src/
[S08]: https://docs.rs/crate/gpui-pre/0.3.5/source/src/app.rs
[S09]: https://docs.rs/crate/gpui-pre-windows/0.3.5/source/src/platform.rs
[S10]: https://docs.rs/crate/gpui-pre-windows/0.3.5/source/src/window.rs
[S11]: https://gpui-kit.com/docs/test/
[S12]: https://gpui-kit.com/docs/installation/
[S13]: https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/examples/root_borderless/src/main.rs
[S14]: https://raw.githubusercontent.com/longbridge/gpui-kit/v0.6.1/examples/stream-markdown/src/main.rs
[S15]: https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html
[S16]: https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading
[S17]: https://gpui-kit.com/docs/coding-guides/
[S18]: https://learn.microsoft.com/en-us/windows/win32/shell/notification-area
[S19]: https://sqlite.org/wal.html
[S20]: https://sqlite.org/backup.html
[S21]: https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew
[S22]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
[S23]: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationtextpattern-getselection
[S24]: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation2-put_transactiontimeout
[S25]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
[S26]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber
[S27]: https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows
[S28]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow
[S29]: https://developers.openai.com/api/reference/resources/chat
[S30]: https://developers.deepl.com/api-reference/translate/request-translation
[S31]: https://doc.rust-lang.org/cargo/reference/profiles.html
[S32]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew
[S33]: https://learn.microsoft.com/en-us/windows/win32/api/wintrust/nf-wintrust-winverifytrust
[S34]: https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys
[S35]: https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex
[S36]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes
[S37]: https://jrsoftware.org/ishelp/topic_setup_privilegesrequired.htm

## 附录 B　实施中不得破坏的不变量

| 编号 | 不变量 |
|---|---|
| I01 | 常规使用只有一个主进程；Updater 不常驻 |
| I02 | 没有 GPUI 业务窗口时，托盘和热键仍可用 |
| I03 | 隐藏不是销毁；TTL 到期必须释放真实窗口与业务引用 |
| I04 | 取词完成前不激活自己的窗口；无新复制事件不读取旧剪贴板冒充选区 |
| I05 | 同 Surface 只有当前 generation 能改变当前结果 |
| I06 | 关闭/取消不仅停止显示，还必须停止本应用读取响应并释放请求槽 |
| I07 | 队列、文本、缓存、线程和日志均有明确上限 |
| I08 | 用户正在选择的文本不被流式更新强制替换 |
| I09 | 清空历史后的旧请求不能自动写回；收藏独立于历史清理 |
| I10 | 密钥不进入普通数据库、日志、命令行或非敏感导出 |
| I11 | 所有开放设置都真实生效且可持久化；失败不能显示成功 |
| I12 | UI、WndProc 和 GPUI render 不执行阻塞网络、数据库或 UIA 调用 |
| I13 | 更新载荷必须先授权验签再执行；hash 不替代签名 |
| I14 | 更新失败恢复同时考虑二进制与数据库；提交后不能回滚旧快照丢掉新数据 |
| I15 | 无头测试不替代 Windows 真机；目标预算不等于实测成绩 |

**最终完成标准是一个经验证可安装、可使用、可升级恢复并能长期轻量驻留的 LexWisp，而不是对这些能力的文字承诺。**
