# LexWisp MAIN SHELL UI REDESIGN SPEC

**版本：1.0｜日期：2026-09-20｜性质：主 UI / Surface 架构重构规范**

> 本 SPEC 取代旧文档中将 **Quick Shell** 与 **Chat Panel** 作为两个独立产品 Surface 的 UI 设计。  
> 本轮只实现 **Chat Experience**，但主窗口与命名必须保持中性，不能把整个 LexWisp Shell 永久绑定为 Chat。  
> 未来可以由 Cowork 或其他 Experience / Plugin 使用同一套 Shell 展开模式。  
> 本轮不实现 Cowork，不实现 Experience Marketplace，不提前搭建复杂通用 UI Plugin API。

---

# 1. 产品定位

LexWisp 的主 UI 不再理解为：

```text
Quick Shell
+
Chat Panel
+
各种文字 Action
```

而统一理解为：

```text
                     Main Shell
                         │
                Host-owned window
                         │
                Experience content
                         │
              Current: Chat plugin
              Future: Cowork / others
                         │
        ┌────────────────┼────────────────┐
        ▼                ▼                ▼
     Compact          Expanded         Workspace
```

其中：

- **Main Shell** 是宿主拥有的窗口和 chrome
- **Presentation** 决定窗口的视觉展开程度
- **Experience** 决定主内容
- 当前唯一 Experience 是 **Chat**
- Chat 仍然保持 Built-in Plugin / Controller 架构
- 将来更换为 Cowork 或其他体验时，不应重写 Windows 窗口生命周期与动画系统

不要把：

```text
Compact = Chat
Expanded = Chat
Workspace = Chat
```

固化进类型命名。

它们只是 Shell 的展示状态。

---

# 2. 本轮范围纠正：停止内置 Translate / Polish 产品功能

旧实现把 Translate / Polish 当成首版必须完成的内置 Declarative Action，并因此把：

- Action Palette
- selection preview
- 参数 UI
- Copy / Favorite / Replace
- 默认 Action route

全部塞进了 Quick Shell。

这个理解现在作废。

## 2.1 当前阶段明确要求

当前阶段只做好：

```text
Main Shell
+
Chat Experience
+
Conversation
+
Composer
+
基本附件能力
+
Workspace
```

以下功能暂不作为首版主 UI 功能：

- Translate
- Polish
- 选中文字后自动执行翻译
- 选中文字后自动执行润色
- Replace original
- Translate / Polish 的参数面板
- Quick Shell 顶部 Action Radio 列表
- Summarize / Methodology / Limitations 等常驻快捷 Action chips
- Default Action 自动路由

---

## 2.2 对现有代码的处理

从应用启动路径移除当前硬编码注册：

```text
assets/translate
assets/polish
DeclarativePackage::parse(...)
```

即：

```text
lexwisp-app/src/main.rs
```

不再默认注册 Translate / Polish。

可以保留：

- Declarative Plugin 基础设施
- Script Plugin 基础设施
- Context capture / clipboard / replace 的底层能力
- Plugin Manager

但这些能力不得继续绑架 Main Shell UI。

Translate / Polish 未来如果恢复，优先作为独立 **Script / Plugin capability** 接入，而不是重新写进 Shell 核心。

本轮不要顺便实现 Translate / Polish 的 JS 插件。

---

# 3. Surface 架构

## 3.1 SurfaceKind

目标从：

```rust
enum SurfaceKind {
    QuickShell,
    ChatPanel,
    ControlCenter,
}
```

收敛为：

```rust
enum SurfaceKind {
    MainShell,
    ControlCenter,
}
```

`ChatPanel` 不再是独立 native window。

旧 `QuickShell` 的窗口生命周期迁移为 `MainShell`。

---

## 3.2 Shell Presentation

新增中性展示状态，命名可按项目风格微调：

```rust
enum ShellPresentation {
    Compact,
    Expanded,
    Workspace,
}
```

语义：

| Presentation | 定位 | 当前 Chat 中的表现 |
|---|---|---|
| `Compact` | 瞬时入口 | Composer 为主 |
| `Expanded` | 轻量连续交互 | 窄长 Transcript + Composer |
| `Workspace` | 完整工作区 | Sidebar + Main Content |

不得出现：

```rust
CompactChat
LargeChat
ChatWorkspace
```

之类把 Shell 状态绑定到 Chat 的命名。

---

# 4. Experience 边界

## 4.1 当前实现

当前只构造：

```text
Chat Experience
```

Chat 继续使用：

```text
ChatController
ChatUiPort
Conversation state
Message state
```

这些业务状态不迁入 `MainShellView`。

---

## 4.2 为未来保留的最小接缝

不要本轮开发完整通用 Experience Plugin API。

只需要把现有：

```text
QuickShellViewFactory
ChatPanelViewFactory
```

收敛成一个中性 factory，例如：

```rust
type ShellContentViewFactory =
    Rc<dyn Fn(/* existing GPUI args */) -> AnyView>;
```

当前 factory 永远构造 Chat Experience。

未来 Cowork 可以替换或注册另一 factory，而不需要重写：

- WindowRegistry
- WindowOptions
- Presentation transition
- Theme
- Root
- Main Shell chrome
- HiddenWarm
- placement

本轮不要增加 Cowork placeholder UI、假按钮或空页面。

---

# 5. 单一窗口原则

三个 presentation 必须尽可能保持：

```text
同一个 WindowId
同一个 GPUI Root
同一个 ChatController
同一个 Conversation
同一个 Composer draft
```

转换：

```text
Compact
   ↓ Send
Expanded
   ↓ Expand
Workspace
```

不得继续采用：

```text
创建 ChatPanel 新窗口
→ Attach 新 observer
→ 隐藏 QuickShell
```

作为正常主流程。

用户看到的必须是**同一个窗口自身展开**。

---

# 6. Compact Presentation

## 6.1 视觉目标

参考 Copilot 的紧凑入口，但不是像素级复制。

目标关键词：

```text
轻
少
悬浮
圆角
输入优先
低 chrome
一眼能输入
```

推荐初始范围：

```text
宽度 640–720 DIP
高度 160–220 DIP
```

根据字体、DPI、附件预览自动调整，但不得初始就成为 600+ px 高的大面板。

---

## 6.2 内容层级

Compact 只保留最必要元素：

```text
┌───────────────────────────────────────┐
│                                       │
│  Ask LexWisp…                         │
│                                       │
│  +      [context/attachment]     Send │
└───────────────────────────────────────┘
```

允许：

- LexWisp logo / 极弱化 experience 标识
- `+`
- 文本输入
- 附件预览
- Send
- Close
- 必要时模型/Provider 的极弱入口

不得出现：

- Conversation list
- Rename
- Delete
- History toolbar
- Translate / Polish Radio
- Action chips 常驻一整排
- 大块 selection preview
- “Ready” 独立状态栏
- 独立设置按钮占据一级视觉位置
- 多行说明文字

---

## 6.3 快捷键行为

全局快捷键唤起 Main Shell 时：

```text
Hidden / Destroyed
        ↓
Compact
```

窗口必须先出现，再异步更新 selection context。

若有 verified selection，可在 Composer 上方或输入区内显示一个轻量 removable chip：

```text
[ Selected text × ]
```

但：

- 不自动翻译
- 不自动润色
- 不自动执行插件
- 不出现 Replace original
- 用户仍然可以直接输入 Chat 请求

---

# 7. Expanded Presentation

## 7.1 触发

用户在 Compact 中发送第一条消息后：

```text
Compact
   ↓
send request once
   ↓
窗口向外扩展
   ↓
Expanded
```

**不得因为 presentation 切换重新发送请求。**

---

## 7.2 视觉目标

Expanded 是：

> 窄长、轻量、可持续对话的 Main Shell。

不是第二套 Chat App。

推荐初始范围：

```text
宽度 640–720 DIP
高度 560–720 DIP
```

保持明显的竖向比例。

---

## 7.3 内容层级

```text
┌───────────────────────────────────────┐
│ LexWisp / current experience      ↗ × │
│                                       │
│  User message                         │
│                                       │
│  Assistant message                    │
│  ...                                  │
│                                       │
│ ┌───────────────────────────────────┐ │
│ │ Ask anything…                     │ │
│ │ +        model/tool         Send  │ │
│ └───────────────────────────────────┘ │
└───────────────────────────────────────┘
```

顶部只保留：

- 当前 Experience 的弱标识
- Workspace Expand
- Close / Overflow

Chat 的：

- Stop
- Regenerate
- Copy
- message actions

应靠近对应 message 或 composer，不要堆成固定大工具栏。

---

## 7.4 移除当前 QuickShell 的混乱布局

现有 QuickShell 中以下布局应删除：

```text
Action Radio row
Source Preview large card
Parameter editors
Result / Chat 双模式大容器
Use Clipboard 独立按钮
Copy / Favorite / Replace / Stop / Send 整排
底部 candidate 说明
```

它们不是新的 Expanded 主结构。

---

# 8. Workspace Presentation

## 8.1 触发

Expanded 右上角提供一个明确的：

```text
Expand / Open workspace
```

操作。

点击后：

```text
Expanded
   ↓
Main Shell 向侧面扩展
   ↓
Sidebar reveal
   ↓
Workspace
```

仍然是同一个 WindowId。

---

## 8.2 视觉结构

```text
┌───────────────┬────────────────────────────────────────────┐
│ LexWisp       │                                            │
│               │              Main Experience               │
│ New chat      │                                            │
│ Search        │              Chat transcript               │
│               │                                            │
│ Conversations │                                            │
│ - A           │                                            │
│ - B           │                                            │
│ - C           │                                            │
│               │       ┌─────────────────────────────┐      │
│               │       │ Composer                    │      │
│               │       └─────────────────────────────┘      │
│               │                                            │
│ Settings      │                                            │
└───────────────┴────────────────────────────────────────────┘
```

推荐初始范围：

```text
宽度 1080–1240 DIP
高度 720–820 DIP
```

必须限制在当前 monitor visible work area 内。

---

## 8.3 当前 Workspace Sidebar

首版只做真实可用内容：

- New chat
- Search conversations，如果搜索尚未实现可延后，不做假按钮
- Conversation list
- Settings
- Collapse back to Expanded

不要为了模仿 Copilot 加入尚不存在的：

- Cowork
- Library
- Notebook
- Agents
- Word / Excel / PowerPoint
- Research Assistant
- Analysis Assistant

未来 Experience / Plugin 真正存在后再接入。

---

# 9. 动画规范

## 9.1 Compact → Expanded

推荐：

```text
180–240 ms
ease-out
```

视觉行为：

- Window bounds 连续扩大
- Composer 尽量保持视觉锚点
- Transcript 区域随着空间出现
- 不使用白屏切换
- 不销毁 Composer Entity 后重新创建
- 不切换 WindowId

---

## 9.2 Expanded → Workspace

推荐：

```text
220–300 ms
ease-out
```

视觉上优先：

- 右侧主内容相对稳定
- 窗口主要向左侧扩展
- Sidebar 从左侧 reveal / fade
- 不做全屏缩放式动画

这样用户会理解为：

> “打开工作区 / 侧栏”

而不是：

> “跳到了另一个程序窗口”。

---

## 9.3 动画实现边界

当前仓库没有现成的 window-bounds animation abstraction。

实现前先做一个最小技术 probe：

1. 确认当前 GPUI 版本是否能稳定更新 native window bounds
2. 验证 Windows 10 / 11 下连续 resize 是否抖动
3. 验证 DPI / multi-monitor
4. 如果 GPUI 没有合适能力，再在 `lexwisp-platform-windows` 增加**很薄的窗口 bounds helper**

不得为了动画创建第二个透明窗口做障眼切换。

如果原生 window resize 动画在当前 GPUI/Windows 组合上不稳定：

```text
优先保证单 WindowId + 正确状态切换
```

可以先做非常短的 content transition，再单独完成 native bounds interpolation。

---

# 10. GPUI-Kit 使用原则

优先使用已有 GPUI-Kit：

- Root
- Button
- Input / Textarea
- Scroll / Virtual list
- TextView / Markdown
- Popover / Menu / Dialog
- Theme token
- Tooltip
- Separator
- List / selectable row

自定义组件只用于：

- Shell chrome
- Composer composition
- Transcript composition
- Attachment preview
- Workspace sidebar
- Window morph 所需的少量容器

不要重新手搓已有基础控件。

---

# 11. 推荐 UI 组件拆分

目标结构可以是：

```text
lexwisp-ui
└── main_shell/
    ├── mod.rs
    ├── controller.rs
    ├── window.rs
    ├── chrome.rs
    ├── composer.rs
    ├── attachment_strip.rs
    ├── workspace_sidebar.rs
    └── experience_host.rs
```

Chat-specific UI：

```text
lexwisp-plugins-builtin
└── chat/
    ├── controller / existing business code
    ├── experience_view.rs
    ├── transcript.rs
    └── message.rs
```

不要求一次性按目录逐字拆分。

核心原则：

> Host Shell 负责窗口与 presentation；Chat Plugin 负责 Chat 内容与业务投影。

---

# 12. Composer 规范

Composer 是三个 Presentation 共用的核心 Entity。

不得为 Compact / Expanded / Workspace 各创建一套输入状态。

应共享：

- draft text
- focus
- attachments
- model preference
- send state
- IME composition state

Presentation 切换后：

- 输入文字不丢
- IME 不异常提交
- attachment 不丢
- focus 行为可预测

---

# 13. Attachment 数据模型

由于目标 Compact UI 明确包含图片 / 文件入口，不允许只画一个不能用的 `+`。

在真正显示附件按钮前，补齐最小 Chat draft 模型，例如：

```rust
struct ChatDraft {
    text: String,
    attachments: Vec<AttachmentRef>,
}
```

推荐逐步升级 message content：

```rust
enum MessagePart {
    Text(String),
    Image(ImageAttachment),
    File(FileAttachment),
}
```

本轮优先支持：

1. 图片
2. 纯文本 / Markdown / Code 文件
3. 其他文件至少有明确 unsupported 状态

PDF / DOCX 深度解析如果会明显扩大范围，可以在后续阶段补充；不得偷偷实现一个脆弱解析器。

Provider serialization 必须根据 Provider 能力处理，不得把本地文件路径直接塞进 prompt。

---

# 14. Chat Experience 当前功能

完成本轮 UI 后，Chat 至少应稳定支持：

- New conversation
- Switch conversation
- Rename conversation
- Delete conversation
- Send
- Streaming
- Stop
- Regenerate
- Model selection
- Message copy
- Context budget notice
- Attachment draft
- HiddenWarm 后继续生成
- Compact → Expanded → Workspace 全程保持同一个 conversation

其中：

- Rename / Delete 不应常驻 Expanded 顶部
- 在 Workspace 中可以放到 conversation row / overflow
- Model selector 收进 Composer，不再单独占一整行 Radio

---

# 15. Window 生命周期

`MainShell` 继续使用：

```text
NotCreated
→ Visible
→ HiddenWarm
→ Destroyed
```

但 presentation 独立保存：

```text
Compact
Expanded
Workspace
```

建议行为：

### Hotkey

```text
Hidden
→ show Compact
```

### Send in Compact

```text
Compact
→ Expanded
```

### Expand button

```text
Expanded
→ Workspace
```

### Collapse workspace

```text
Workspace
→ Expanded
```

### Close / Escape

按产品设置执行 hide；HiddenWarm 期间：

- Chat generation 继续
- Controller 不重建
- Window 再次显示时状态同步

不要因 presentation change 触发 HiddenWarm。

---

# 16. HostUiCommand 调整

旧命令：

```text
ShowQuickShell
ShowChatPanel
```

应逐步迁移到中性命名，例如：

```rust
ToggleMainShell(...)
ShowMainShell(...)
SetMainShellPresentation(ShellPresentation)
ShowControlCenter
ApplyLaunchContext(...)
RefreshPlugins
Quit
```

具体 payload 与 bugfix SPEC 中的 `launch_generation` 对齐。

`ShowChatPanel` 最终删除。

---

# 17. 视觉信息层级

必须遵守：

```text
1. Composer / 当前任务
2. Transcript / 当前内容
3. Conversation navigation
4. Provider / Settings / secondary action
5. Plugin management
```

不要把第 4、5 层功能重新提升成一级工具栏。

---

# 18. 当前 Main Shell 明确禁止出现的东西

以下全部视为旧 UI 遗留，不得在新设计中原样保留：

- `LexWisp Quick Shell` 大标题
- 一整排 Action Radio
- 一整排 Summarize / Key Points / Methodology / Limitations
- 大号 source preview card 常驻
- Model 独立 Radio 行
- Rename / Delete / Close 同时挤在顶部
- Copy / Favorite / Replace / Stop / Send 同级堆成大按钮行
- 底部独立 `Ready`
- `Open chat…` 通过另一个 native window 打开
- Chat 与 Action Result 在同一个 Surface 中硬切两个完全不同布局

---

# 19. Reference Screenshots

**建议把参考截图放进仓库，让 Coding Agent 在实现前直接查看。**

推荐路径：

```text
docs/ui-reference/
├── copilot-compact.png
├── lexwisp-current-expanded.png
└── copilot-workspace.png
```

使用原则：

### `copilot-compact.png`

参考：

- Compact 的体量
- 留白
- 圆角
- 输入优先
- 极少 chrome
- 附件入口的位置层级

不要复制：

- Microsoft / Copilot 商标
- Copilot logo
- 品牌专属图标
- 品牌颜色
- 像素级尺寸

### `lexwisp-current-expanded.png`

作为**当前状态与反例参考**：

- 证明现有信息层级过多
- 不要继续在该布局上简单换颜色 / 圆角
- 应进行 composition 重构

### `copilot-workspace.png`

参考：

- Workspace 左侧 Sidebar + 主内容的层级
- 从轻量入口扩展到完整工作区的产品逻辑
- 大空间下仍保持主 Composer 明确

不要照搬不存在的 Copilot 功能。

---

## 19.1 给 Coding Agent 的明确要求

实现前：

1. 阅读本 SPEC
2. 查看三张 reference screenshot
3. 阅读当前 `surface.rs`、`view.rs`、`chat_panel.rs`
4. 先画出新的 component / ownership map
5. 再开始改代码

如果当前 Agent 环境无法读取图片，则以本 SPEC 的文字约束为准，不得根据文件名猜测截图内容。

---

# 20. 推荐实施阶段

## Stage UI-0：清理错误产品范围

- 移除内置 Translate / Polish 默认注册
- 移除 Quick Shell Action radio / parameter/result 主流程
- 保留底层插件基础设施
- 不新增 JS Translate / Polish

验收：

```text
应用启动后主体验只有 Chat
```

---

## Stage UI-1：单 MainShell 架构

- `QuickShell + ChatPanel` 收敛为 `MainShell`
- 建立 `ShellPresentation`
- 单 WindowId
- 单 Shell content factory
- 当前 factory 只返回 Chat

先不追求完整动画。

验收：

```text
Compact / Expanded / Workspace 切换不创建第二个 Chat native window
```

---

## Stage UI-2：Compact

- 新 Composer
- Focus / IME
- Send
- Context chip
- Close
- 基础 attachment draft

验收：

```text
Win+C 后能快速出现并直接输入
```

---

## Stage UI-3：Expanded

- Transcript
- Streaming
- Stop
- Regenerate
- Composer 共享
- Compact → Expanded transition

验收：

```text
第一次 Send 只发一次请求，窗口自然展开，对话不中断
```

---

## Stage UI-4：Workspace

- Sidebar
- Conversation list
- New chat
- Rename / Delete via contextual controls
- Settings entry
- Expanded ↔ Workspace transition

验收：

```text
切换 presentation 不改变 active conversation，不重新发送请求
```

---

## Stage UI-5：Attachment

- Image preview
- Text/code attachment
- drag & drop
- Provider capability / unsupported handling

验收：

```text
Attachment 不是装饰按钮，加入后真正进入 ChatDraft / request pipeline
```

---

## Stage UI-6：视觉与性能收尾

- Window bounds animation
- DPI / multi-monitor
- small screen clamp
- Dark / Light
- keyboard navigation
- focus restore
- resource cycle
- long transcript performance

---

# 21. 验收场景

至少人工验证：

### Scenario A

```text
Win+C
→ Compact 立即出现
→ 输入“你好”
→ Send
→ 同一窗口展开为 Expanded
→ 流式回复
```

### Scenario B

```text
Expanded
→ 点击 Expand
→ 同一窗口向侧边扩展
→ Sidebar 出现
→ active conversation 不变
```

### Scenario C

```text
Workspace
→ 新建 Conversation B
→ 返回 A
→ A 的消息完整保留
```

### Scenario D

```text
生成中关闭 MainShell
→ HiddenWarm
→ 请求继续
→ 再打开
→ terminal 状态正确显示
```

### Scenario E

```text
Win+C
→ 原应用有选中文字
→ MainShell 先出现
→ selection context 稍后作为 chip 到达
→ 不触发 Translate / Polish
```

### Scenario F

```text
输入未发送文本
→ Compact / Expanded / Workspace 来回切换
→ draft、附件、IME 状态不丢
```

---

# 22. 最终设计原则

本轮完成后，LexWisp 的 UI 关系必须是：

```text
Main Shell
    │
    ├── Presentation
    │      ├── Compact
    │      ├── Expanded
    │      └── Workspace
    │
    └── Experience
           └── Chat（当前）
```

而不是：

```text
Quick Shell = Translate/Polish/Chat 混合工具箱
Chat Panel = 第二个窗口
```

Shell 是长期稳定的产品框架；Chat 是当前默认 Experience。

未来加入 Cowork 或其他插件时，应复用 Main Shell，而不是复制第三套、第四套独立窗口。
