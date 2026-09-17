# AGENTS.md

## 1. 项目定位与固定约束

LexWisp 是面向 Windows 技术用户的轻量原生 AI 文本工具，采用 **Rust＋GPUI＋GPUI-Kit＋windows-rs**。产品采用 **单进程 Host＋插件**，不是把功能堆在一个 GUI 程序中再冠以插件名称。

首批功能为 Chat、Translate、Polish。Chat 是 Native 插件；翻译、润色和自定义 Prompt 动作优先使用声明式插件；第三方多步骤逻辑使用 Script 插件。三类插件共用 Action、授权、任务监管和结果展示路径。

产品约束：

- 快捷键入口支持三种可切换模式：动作选择、选中文字直接翻译、执行用户指定默认动作；无选中文字不偷偷上传剪贴板。
- Chat 支持多会话和独立大面板，小窗转大窗必须延续同一会话及请求。
- 默认翻译／润色收起取消，Chat 收起继续；用户可以覆盖动作／插件的收起策略。
- 默认保存已提交输入、结果和部分结果到本地；支持关闭记录、删除和清空。
- 替换原文必须由用户主动点击并再次验证目标，绝不由模型或脚本自动写回。
- 窗口短期隐藏后复用，闲置后释放；正常后台只运行一个 `LexWisp.exe`。
- Provider 以 OpenAI 兼容文本接口为当前基线，服务地址、Key、模型和 Profile 可配置。
- 自定义 Prompt／脚本先通过文件编辑；日常设置必须有正常 UI。
- 分发以解压运行的 Windows x64 绿色版为基线；不依赖用户安装 Rust、Node 或开发工具。

未经新的明确需求，不引入 Electron、Tauri、WebView UI、双进程 GUI、每插件独立常驻进程、在线插件市场或多协议 Provider 大重构。未来维护型 Updater 可以短暂独立运行，不改变正常主程序单进程边界。

## 2. 架构与依赖边界

### 2.1 资源唯一所有者

Host 所有并管理：进程生命周期、插件注册与启停、ActionRegistry、权限、Invocation、取消、HTTP／AI、Provider、Context、存储、凭据、任务、窗口路由、主题和系统能力。

插件只拥有自己的业务状态、Action、参数、UI 内容与受监管的任务。**插件拥有 View／Surface 内容，Host 拥有 Window。** Host 不通过插件 ID 分支实现聊天、翻译或润色业务。

不要按插件重复创建 Tokio Runtime、reqwest Client、数据库连接池、GPUI App、主题系统或 Windows 监听器。HTTP 因不同网络配置可有少量复用实例，不等于允许每请求新建 Client。

### 2.2 Workspace 责任

| crate | 责任 | 项目依赖方向 |
|---|---|---|
| `lexwisp-core` | IDs、DTO、纯规则、状态机、必要端口 | 不依赖其他项目 crate |
| `lexwisp-platform-windows` | Win32／COM、热键、托盘、UIA、剪贴板、凭据 | core |
| `lexwisp-storage` | 配置、SQLite、迁移与一致性备份 | core |
| `lexwisp-host` | 注册、监管、授权、共享服务、AI 与声明动作执行 | core、storage、platform-windows |
| `lexwisp-ui` | GPUI 壳、UI 注册、公共产品组件、展示投影 | core |
| `lexwisp-plugins-builtin` | Chat 模型、动作、视图和生命周期 | core、ui |
| `lexwisp-plugins-script` | JS 加载、VM、受控 Host API 桥接 | core |
| `lexwisp-app` | Composition Root、启动、资源和发布入口 | 组装上述模块 |

core 不导入 GPUI、Win32、reqwest、SQLite 或具体插件。Host 不依赖具体插件 crate；UI 不依赖 Host 实现；插件不依赖 Host 内部模块。Host→UI 通过纯数据命令与主线程桥接，Native View 工厂在 UI 契约下注册，不能把 GPUI 类型塞回 core。

现有仓库有不同但合理的模块命名时按责任判断，不为对齐名字进行无意义搬家。保持依赖无环，只在真实边界成立时新增 crate；不预建一堆空包。

### 2.3 状态唯一真源

运行中请求与结果归 Host ExecutionStore；会话语义归 Chat 插件；持久化归存储层；光标、滚动、选区等瞬时状态归 View。关闭窗口不能丢失继续运行所需的业务状态。

同一份聊天正文不要分别在 Chat、History、Popup 中持久化三遍。用引用、版本和展示投影复用数据，不建立全局 `Arc<Mutex<AppState>>` 让所有操作竞争一把锁。

## 3. 插件与公共契约

Action 是统一用户能力入口。Native、Declarative 和 Script 通过同一注册、校验、权限与 Invocation 流程，不建立“内置功能快捷后门”。

使用稳定的 PluginId、ActionId、InvocationId、ConversationId 和 MessageId。插件显示名可变，身份不可依赖显示名。注册整包先验证再原子生效，失败不能留下半注册动作。

Native 是可信、编译期 Rust 实现，不是任意 DLL 插件。不承诺 Rust ABI。需要 `dyn` 分发的异步 trait 必须使用对象安全签名或已验证的转换方案；不要把原生 async trait 与任意 trait object 支持混为一谈。主线程 UI 工厂不强行要求 Send／Sync。

禁用／重载顺序：拒绝新请求，推进 generation，撤销授权，取消任务，解绑 UI／订阅，有限清理，再激活新版本。旧句柄、Promise 和事件不能复活已停用插件。

声明插件以 manifest 和 Prompt 为真源，参数表单由 Host 渲染；禁止 manifest 只是摆设而逻辑仍写死在 UI。模板仅做已知变量替换，不执行表达式或代码。用户输入作为 user 内容，不当作模板语法执行。

不提供万能 `invoke(name, json)` 作为绕过类型和权限的后门。真正插件间通信只使用经过授权的明确契约，不直接访问另一个插件的私有状态。

## 4. GPUI 与 GPUI-Kit 开发规则

### 4.1 版本与组件复用

开始修改 UI 前，先读 Cargo.lock、workspace dependencies、相关 Kit 源码和同版本示例。优先经 `gpui_kit` 门面使用匹配的 GPUI／Component 类型，不混入来源或版本不同的同名类型。Kit 对依赖家族的管理见 [K1]。

优先使用 Kit 的 Button、Input、Select、Dialog、Tooltip、Toast、主题、列表和滚动能力。只为重复的产品场景做薄组合；不要在项目内重新建立通用组件库或仿造一套输入框／Markdown 编辑器。

上游缺陷先定位和构造最小复现；必要时做最小兼容层或锁定 patch，并记录移除条件。不能复制整套上游实现到本仓库，或通过反复换版本解决未知原因的编译失败。

### 4.2 初始化与窗口

Kit 初始化一次，每个真实窗口一个顶层 Root，不为每个页面再包独立 Root。正确维护窗口内的 overlay、焦点和选择范围，参考同版本入门代码。[K2]

设置显式退出模式，关闭最后一个 GUI 窗口不等于退出 Host；用托盘或明确退出动作结束进程。[K3] 不靠永不关闭的隐藏 GPUI 主窗口常驻。

所有真实窗口由 WindowRegistry 创建、复用和销毁。插件仅注册内容或请求 Surface。Popup 收起、View 销毁、Invocation 取消分别建模，不在窗口 Drop 中无条件 abort 所有业务任务。

### 4.3 Entity、状态和渲染

有跨帧状态才使用 Entity；纯呈现用 RenderOnce／普通组合。InputState、FocusHandle、ScrollHandle、订阅和子 Entity 在所属 View 初始化时建立并持有，不在 render 里反复创建。[K4]

render 只描述当前界面。不得在 render／layout／paint 中发请求、跑 SQL、读文件、求值脚本、生成随机身份、创建长期任务或注册订阅。不要在 render 末尾无条件 notify 自己。

使用真实业务 ID 作为稳定 ElementId，重复控件加所属对象命名空间；不要以排序后会变的列表下标或本地化标题作为身份。

一份状态只有一个 owner。组件回调报告用户意图，由 owner 更新；同步外部状态时不要反向重复触发用户回调。更新多个相关字段后一次通知，避免反馈循环。

不要用派生值反复克隆整个历史，也不要为了避免借用错误把所有对象包装成 Arc＋Mutex。先修正状态归属、借用范围和调用边界。

### 4.4 流式、长列表和可见性

流式增量在 Host 累积，UI 按短时间片合并显示，终态立即 flush。限制的是投递和文本处理成本，不是假定 GPUI 没有帧合并。只刷新相关消息／结果实体，不每个 token 重绘整个主窗口。

Markdown 按消息版本缓存，不每 token 全量重解析。富文本选择不可靠时提供真正可选择的纯文本／只读多行视图，不将“整段复制按钮”当作文本选择。

历史和会话同时做数据分页与 UI 虚拟化。关闭窗口停止不必要的刷新和动画；业务继续不表示隐藏 UI 也必须继续渲染。用户主动向上滚动后停止自动追底。

输入法组合期间 Enter 不发送；Escape 按覆盖层层级处理。Dropdown、菜单、文件对话框和候选窗口的焦点变化不能误判为离开 Popup。

### 4.5 异步更新与引用生命周期

不得保存或跨 await 使用 `&mut App`、`&mut Window`、`&mut Context<_>`。通过拥有明确生命周期的句柄传递，回到 GPUI 线程更新 Entity。[K4]

后台回调不强持有整棵 View。使用 WeakEntity 或等价弱引用，并检查窗口／插件／请求 generation；目标已销毁时正常丢弃 UI 更新，但 Host 的任务和持久化按自身规则继续。

保留必要 Subscription，销毁时释放；不能用无归属 detach 消除生命周期警告。跨线程只传 Send 数据，不给 GPUI 或 COM 对象补 `unsafe impl Send/Sync`。

### 4.6 拖放

只在明确接受输入的位置注册拖放。拖动过程中只检查轻量 metadata，不解压包、不扫描目录、不读完整文件。

插件包落入插件管理页后进入同一导入预览与授权流程，不直接执行代码。文件路径、包内容、链接和大小都要验证。类型不匹配时给出说明，不把任意文件静默解释为插件或模型输入。

## 5. 异步、取消与可靠结果

Host 只有一个业务 Tokio Runtime；所有业务任务经 TaskScope／Supervisor 管理。每任务可追溯到插件，Action 子任务还关联 Invocation；不要在各处裸 `tokio::spawn` 后不保存 handle。

UI 线程不阻塞网络／数据库；数据库、UIA 和脚本各自按已定义执行域工作。长期阻塞循环使用有所有者的工作线程，不永久占用 spawn_blocking。已启动的阻塞任务不能仅靠 abort 被停止，取消承诺必须符合真实语义。[K5]

每个 Invocation 有唯一终态；用户停止、成功、失败竞争时由 Supervisor 决定。处理器不能同时 emit Completed 并返回错误。终态后晚到 delta 不再更改结果。

使用有界队列和正文大小限制。UI 可以合并或跳过刷新，但业务文本、错误和终态不能无恢复地丢弃；订阅落后时重取版本化快照。

会话切换不改变请求的归属。小窗转大窗先 attach 后 detach，不取消再重发。Chat 同一会话最多一个有效生成，其他会话可独立运行。

取消网络与撤销权限不等于删除内容。删除或清空内容时先建立写入屏障，防止后台检查点把记录重新插入。不要声称取消客户端就能撤销服务端费用。

## 6. Script 与安全边界

脚本当前采用独立工作线程上的 rquickjs／QuickJS，共享 VM、每插件独立 Context。Script 不直接持有 GPUI、reqwest Client、数据库或系统句柄。

只加载包内 JS 与受控相对模块；不在用户机器运行 npm、安装依赖、加载任意动态原生模块或下载远程代码。不要同时引入另一套脚本引擎。

manifest 的 capability 是请求，只有 Host 的 grant 才是授权。每次敏感调用重新检查 generation；权限 UI 不能替代执行层检查。JS→Host 闭包与 Promise 固定携带真实身份，不能使用跨 await 的“当前插件”全局变量。

提供明确的 CPU 时间、微任务、内存、输出、网络返回体和 KV 配额。覆盖入口函数与 Promise job pump；检查实际 allocator feature 是否影响内存限制。[K6] VM 资源限制不是操作系统级沙箱，不能把同进程恶意代码隔离宣传为完整安全保证。

网络范围必须核查重定向目标，不跨 origin 泄漏认证。默认不给第三方 shell、任意文件、全局历史或后台自动写回能力。脚本只能通过用户明确授权的数据与操作工作。

导入先检查 ZIP 路径、Windows 特殊路径／名称、链接、文件数和解压量，再执行安装事务。预览和扫描绝不运行入口脚本。哈希用于内容一致性，不等于可信签名。

## 7. Provider、存储与 Windows 实现习惯

### 7.1 网络

Provider 选择留在 Host；参数、Profile 和 endpoint 是配置，不在插件中按品牌特判。一个明确兼容适配器服务多个配置实例。

保留自定义 Base URL 的路径前缀；检查最终请求路径，不重复追加 `/v1` 或让前导 `/` 清空前缀。认证头按请求注入，不放进跨 Provider 共用 Client 的默认头。

SSE 必须按协议增量解析，处理跨 chunk UTF-8／JSON、空 choices、usage、完成标识与截断。网络 chunk 不等于完整 JSON。异常断流保留部分文本，不显示为成功。

不要默认自动重试可能计费的生成 POST；不通过关闭 TLS 校验、静默切换 Provider 或无上限重试掩盖错误。

### 7.2 存储

普通设置为版本化配置文件，内容数据为 SQLite；只保留一个权威来源。数据库通过存储模块访问，不在 View 和插件里拼 SQL。

写配置先验证再原子保存，失败保留旧内容；不要用“无法解析就恢复默认并覆盖”处理用户数据。迁移可定位、可备份，较新 schema 禁止被旧程序重置。

流式批量落盘，结束时及时提交；取消、失败和中断状态明确。读取分页；有 WAL 时使用一致性备份，不直接复制单个活跃 `.db` 文件。

Key 只进 Credential Manager，配置保留引用；日志、诊断、示例与测试不能出现真实 Key。自动历史开关不影响用户主动保存，但后者必须是明确操作。

路径不依赖工作目录，支持中文和空格。默认用户数据与程序分离；portable.flag 切换的行为明确，不把跨机器复制等同于凭据也可迁移。

### 7.3 Windows

平台 unsafe 必须小范围封装，说明句柄／指针所有权、线程和释放条件。遵守 COM apartment，UIA 在无窗口 MTA 线程执行；不能用无限新增线程处理不可取消调用。[K7]

热键触发先捕获原前台目标，再显示会抢焦点的窗口。选区、候选复制文本、无选择、不支持和失败使用不同状态；不能拿旧剪贴板当新选区。

剪贴板回退要考虑完整格式与竞争更新，无法保护时降级，不静默破坏图片或富文本。序号变化不是目标来源证明。

替换必须确认原进程／控件／选区仍有效并有一次性用户授权；不把全字段 SetValue 当成选区替换。SendInput 受权限级别限制，不自动提权，不误写当前其他窗口。[K8]

DPI 和多屏坐标集中转换，处理负坐标与工作区边界。热键改绑先成功注册新键再释放旧键，托盘在 Explorer 重启后恢复。

## 8. 代码质量、复用与抽象

动手前阅读涉及模块和已有实现，优先复用已存在的服务、错误、组件和验证函数。先找真正重复，再提取抽象；不要因为将来“可能有多个实现”增加一层接口。

好的边界通常是有真实意义的类型、模块和少量 trait。避免通用 DI、反射总线、万能 Manager、无调用者接口及只会转发的多层 Service／Repository／Adapter。没有业务意义的泛型和继承式“抽象类”不要引入。

保持控制流直接：校验使用 guard clause，复杂分支用明确 enum；不要用嵌套闭包和层层 Option／Result 链掩盖错误处理。不要为了减少行数写不可读的表达式。

生产路径避免 `unwrap`／`expect` 处理用户输入、网络、文件、数据库和系统调用；不可恢复的启动失败需要可见诊断和可控退出。禁止把错误吞掉或统一变成空字符串。

新增依赖要有实际功能、维护或正确性收益；优先已有库，不为一个小 helper 引入庞大框架。禁止顺手做与当前任务无关的大规模格式化、改名和架构迁移。

注释解释约束、线程、安全和为什么这样处理，不重复代码字面意思。删除废弃实现及不用的字段；不要长期保留两套“临时兼容”路径。

## 9. 测试与验证投入

测试优先覆盖真实风险：取消竞争、过期事件、权限撤销、上下文隔离、SSE 边界、路径校验、数据迁移和删除后回流。纯逻辑用普通 Rust 测试；真实 UI 协调用少量同版本 Kit 测试；Windows 系统交互保留原生 Release 验证。[K9]

不要为每个 getter、配置常量和 UI label 写一份测试。相似输入用表驱动；HTTP mock、临时数据库与 fixture 共用最小帮助代码，不复制成多个测试框架。不要按数量堆测试，也不要以“不要太多测试”为由删除高风险回归。

不重测 GPUI-Kit 全部控件，不构造巨量易碎的视觉快照，不创建和当前变更无关的性能脚本。每次实际缺陷优先补一个最小复现与回归。

测试专用 feature 放在 dev-dependencies，匹配运行依赖的来源与版本。不盲目用 `--all-features`。线上 API 与真实密钥不进入自动测试。

## 10. 构建、交付与事实记录

在最早能运行时就构建 Release，并从实际输出／解压目录启动；之后每次形成可体验增量继续验证。不要等插件、设置、历史全部完成才第一次试 Release。

按当前 workspace 中已经存在的包运行相关命令，完整工程使用：

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
```

锁定 Rust toolchain 和 Cargo.lock，不用漂移依赖换取偶然成功。需要改变公共契约、依赖来源或数据格式时，记录原因与迁移／验证，不悄悄破坏已可用功能。

一个主要打包脚本足够；不创建大量一行 shell／PowerShell 包装、临时排障脚本和重复总结文档。临时探针验证完删除或并入最小回归。

性能使用 Release 测量，记录硬件、Windows、驱动、DPI 和场景；区分工作集、Private Bytes、GPU 内存和线程／句柄数量。禁止用工作集修剪假装解决泄漏，也不要声称销毁窗口等于所有 GPU 资源归零。

区分编译通过、启动成功、功能通过、性能达标。缺少 Windows 或真实服务条件时明确记录未验证项，不能伪造成功截图、测试输出、启动结果或 benchmark。已有阻塞不妨碍完成独立的可验证工作，但未验证项不能计为完成。

产物不得包含 API Key、个人数据、日志、构建缓存或无权分发的资源。可执行文件之外有真实运行依赖时随包提供，不为“单文件”删除必要内容。最终在无开发工具的普通用户环境验证解压运行。

## 11. 上游依据

使用锁定版本源码决定精确签名，以下资料用于核对关键语义，不替代本项目的构建与实机验证。

- [K1　GPUI-Kit API](https://docs.rs/gpui-kit/latest/gpui_kit/)
- [K2　GPUI-Kit Getting Started](https://gpui-kit.com/docs/getting-started/)
- [K3　QuitMode](https://docs.rs/gpui-kit/latest/gpui_kit/enum.QuitMode.html)
- [K4　GPUI-Kit Coding Guides](https://gpui-kit.com/docs/coding-guides/)
- [K5　Tokio spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [K6　rquickjs Runtime](https://docs.rs/rquickjs/latest/rquickjs/runtime/struct.Runtime.html)
- [K7　Microsoft UI Automation Threading Issues](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading)
- [K8　Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
- [K9　GPUI-Kit Testing](https://gpui-kit.com/docs/test/)
