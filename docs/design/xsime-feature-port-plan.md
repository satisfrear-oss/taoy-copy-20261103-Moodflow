# 从 XSIME 吸收基础功能：Windows + Android 双平台计划

> 状态：**实施中**（立项 2026-10-17，最近更新 2026-10-18）。决定记录见 [ADR 0056](../adr/0056-吸收XSIME基础功能与Android端.md)。
> 参考对象为 [scholay/xsime](https://github.com/scholay/xsime) `main`（Apache-2.0，macOS/iOS 为主线，另有 Windows/Android/Linux 端）。
> 本文回答「拿什么、放哪、按什么顺序做、怎么验收」，并在 §9 记录进度、§10 汇总全部未完成项；
> 单项功能的细节另见各自的设计文档（如 [buffer.md](buffer.md)、[plugin-system.md](plugin-system.md)）。
>
> **一句话现状（2026-10-18）**：P0、P1 完成；P2 Windows 接入的代码已合入 `main`（PR #96、#97、#98），**未真机验证**；
> P3 Android 的 XSIME 界面移植在 PR #100（CI 全绿，未合入，**未真机验证**）；P4 二批只做了打字测速。

## 0. 结论先行

1. **吸收的是功能与交互契约，不是 XSIME 的引擎。** XSIME 的中文输入全部建在 librime 上；
   Moodflow 有自研 Rust 引擎（`moodflow-engine` / `moodflow-dict` / `moodflow-store`）。
   把 librime 拉进来等于并行维护两套引擎、两套用户词库，**不做**。
2. **先进纯逻辑 crate，再接两个宿主。** 每个功能的状态机、数据格式、失效规则写在零平台依赖的
   crate 里，必须过 `cargo check-android`；Windows（TSF + `moodflow-ui`）与 Android
   （`moodflow-mobile` → `moodflow_android/` 里的 JNI 绑定）只做薄薄的呈现与投递。这是本仓「平台分层策略」的
   直接应用（见 AGENTS.md 同名一节）。
3. **首批只拿六项基础功能**（§2）。**2026-10-18 修订（ADR 0057）**：除摩斯电码与电音演奏外，原先排除的 XSIME 能力全部纳入二期计划（[xsime-extended-plan.md](xsime-extended-plan.md)）；下面这句是一期立项时的范围：其余 XSIME 能力（Mailbox 终端、截图录屏、音乐、摩斯、
   密码库、iCloud 等）明确不在范围内（§7）。
4. **两边都有的功能，也吸收 XSIME 做得更好的地方**（§3）。已核实的差距直接列为行动项；
   没核实到的只列为对照核查项，不预设 Moodflow 落后。

## 1. XSIME 功能盘点与取舍

XSIME 的结构：librime 引擎 + 三个「槽位」界面（Buffer / Capsule / Mailbox）+ 插件目录。
按「是否是输入法的基础能力」「Moodflow 是否已有」逐项判断：

| XSIME 功能 | 本质 | Moodflow 现状 | 取舍 |
|---|---|---|---|
| **Buffer**（上屏前的文本工作台，按块投递） | 输入核心交互 | 无 | ✅ 首批 |
| **候选矩阵**（`↓` 展开最多 3 行连续页、方向键/数字键导航） | 候选交互 | 有横/竖排与翻页，无矩阵 | ✅ 首批 |
| **并击**（同批组键 / 先左后右，键位方案 JSON 导入导出） | 输入方式 | 无 | ✅ 首批 |
| **剪贴板历史**（Capsule「临时」模块的最小子集） | 周边基础能力 | 只有 `HostServices` 读写剪贴板 | ✅ 首批（仅文本/链接） |
| **跨平台方案导入**（同一份 Rime ZIP / UTF-8 TXT·TSV 词库五端导入） | 数据迁移 | 有 `.wpkg` 方案包，无 Rime 包导入 | ✅ 首批（转换到 `.wpkg`，见 §2.5） |
| **九宫格拼音**（移动端） | 移动端输入方式 | 无（UI 里的「九宫格」是九宫格图片拉伸，与 T9 无关） | ✅ 首批（Android 专属） |
| 学习词库异常提示与恢复 | 数据安全 | 用户库打不开时**静默降级**为不落盘（§3.2） | ✅ 首批（§3.2） |
| 日常统计 / 打字测速 | 周边 | 日常统计已有（`moodflow-store::stats`：字符分类、时段、码长、选重、活跃时长、速度模型 v2）；无打字测速 | 🔶 二批，补打字测速（§3.3） |
| Buffer 插件：AI 生成 / 润色 / 实时翻译 | 依赖 Buffer | Hub 已有 AI 服务与温柔改写（ADR 0012/0028） | 🔶 二批，复用 Hub 的 AI 配置 |
| XSIME 五笔（五笔码与拼音免切换） | 方案 | 已有混输、满码唯一上屏、临时拼音与编码提示 | 🔁 已覆盖，用 XSIME 用例做对照测试（§3.5） |
| Mailbox（终端 / 对话 / 收件） | 桌面应用 | Hub 有对话 | 🔶 二期（ADR 0057），在 Moodflow 里叫「工作区」 |
| Capsule 截图、录屏、OCR、笔记、资源、密码、iCloud | 桌面应用 | — | 🔶 二期（ADR 0057）；iCloud 改为同步到用户自选文件夹，默认端到端加密 |
| LaTeX、文献引用（Scholay） | 垂直插件 | — | 🔶 二期（ADR 0057） |
| 电音演奏、摩斯电码 | 趣味插件 | — | ⛔ 不做 |

## 2. 首批六项：要实现的契约

下面每项列出**从 XSIME 继承的行为契约**（已在其 macOS/Windows/Android 上验证过的规则），
以及在 Moodflow 里的落点。契约是验收依据，实现由我们自己写。

### 2.1 Buffer（缓冲工作台）

契约（取自 XSIME `ARCHITECTURE.md`、Android README、`Shared/Sources/RimesCore/DefaultBuffer.swift` 的行为描述）：

- 开启后，确认的候选**先进 Buffer 而不是宿主**；每次确认的字符串是一个块，英文按词成块。
- 「插入下一块」「插入全部」「删除最后一块」；Return 只在无组字时发送 Buffer，**同一次按键
  不能既确认组字又发送**。
- **投递授权绑定目标**：换输入框、失焦、隐藏键盘/面板、切输入法、进程销毁都会撤销待投递
  内容的授权，旧内容绝不投递到新目标。宿主拒收时保留文本可重试，不算已消费。
- 密码框与私密字段禁用 Buffer。
- 正文是临时数据：升级、重启、重置可直接丢弃，不要求用户保存（XSIME 2026-10-10 维护策略）。
- 容量上限（XSIME 为 16,384 UTF-16 码元）。

XSIME Windows 端已打磨过的细节，一并作为 Windows 验收项（取自其 `platforms/windows/installer/README.md`
与 `native/README.md`）：

- 在宿主输入框按快捷键（XSIME 默认 Alt+Shift+B）**显式绑定**目标；换框、换应用、换输入法后暂停
  输入与投递，重新绑定才继续，不自动跟随焦点。
- 发送键轻按投递下一块、长按（默认约 1.2 秒，可配 500–3000 ms）逐块投递**按下时冻结的队列**；
  可选 1/2/3/5 秒的节奏投递；最后一块确认后可自动关闭面板。
- 普通 Enter 只处理组字、不发送；未配置为发送键的 Ctrl/Alt/Win 组合照常交给宿主。
- 投递是否被宿主接收无法确认时**不重发**，保留内容并提供复制；面板不激活，键盘焦点留在宿主。

落点：

- 新 crate **`moodflow-buffer`**（纯逻辑）：块模型、编辑操作、目标修订号与投递授权、容量上限。
  不知道 TSF/Android 的存在；宿主以「目标 ID + 修订号」描述焦点。
- 协调器：Buffer 开启时把上屏路由改到 buffer；新增 Buffer 的表现层命令（实际落地为 `UiCommand::ShowBuffer(BufferView)` / `HideBuffer`，在
  `moodflow-ui-types`，遵守其三条红线）。
- 开关：出厂快捷键放**标量字段**（如 `keys.toggle_buffer`），不要放进 `keys.key_actions`
  ——否则用户删不掉（AGENTS.md「出厂预置的绑定/开关」一条）。

### 2.2 候选矩阵

契约（XSIME `CANDIDATE-MATRIX.md`，Windows/Linux 共用 `Shared/CandidateMatrix/navigation.hpp`）：

- 多于一页时 `↓` 展开，视口最多 3 行、每行一页，选中项前进时继续取页。
- 左右在整行内移动；行太长时滚动到选中列，不把候选挤成小格。
- 上下 / PageUp/PageDown / `-` `=` 在行间移动；首行 `↑` 或 Esc 收起并**保留组字**，再按 Esc
  才走普通取消。
- 空格上屏高亮项；1–9 只选当前行可见项；鼠标选择保留真实页号与下标。
- 失焦、上屏、新输入、密码框都会使旧矩阵投影和点击回调失效。

落点：导航状态机放 `moodflow-candidate`（纯逻辑，可单测）；键路由**必须**走现有
`keymap::NavKeys` + `apply_nav_key`，不新写硬编码分支（AGENTS.md「候选导航键」）。
`UpdateCandidates` 增加矩阵行数据；Windows 由 `moodflow-ui` 候选窗画，Android 键盘候选区按宽度排版。

### 2.3 并击

契约（XSIME `CHORD-KEYMAPS.md`）：

- 统一规则同时支持「同批按下」和「先左后右分开敲」；**一组键全部松开时结算**，组键间隔可配。
- 键位方案 = 左右键区 + 音节边界策略 + 映射表（组合 → 完整音节或拼音片段）；
  可复制内置方案、新建、导入/导出 JSON；空映射表可存草稿，不可应用。
- 启用并击扩展**不替换**当前普通方案。

落点：

- 新 crate **`moodflow-chord`**：按键批次结算、键位方案格式与校验、组合 → 拼音片段；输出喂给
  现有拼音引擎，不另起候选路径。
- ⚠ **Windows 风险最大的一项**：并击需要 keydown + keyup。按 AGENTS.md「C++ 吃键集必须 ⊆ Rust
  出字集」，`moodflow_tsf` 在并击方案下吃掉的每个键，Rust 侧必须在同条件下产出；结算时机在
  keyup，要先确认 TSF 现有 keyup 转发机制（目前只用于修饰键轻敲）能扩展到字母键。
- Android：XSIME 已在触屏上做了并击——左右手各一指，起点固定、终点可滑动，只松开一只手不出字，
  最后一指抬起时一次结算，按住的键和可接的键高亮，候选栏显示左右两半与合成预览。首批先做
  **物理键盘**并击（与 Windows 共用结算逻辑）；触屏并击按 XSIME 这套交互放在 P3 末尾，
  键盘面高度公式与左右分体间距可直接参考其 Android README。
- 内置键位数据（XSIME「飞耀」427 条映射）：先确认其许可证再决定是否收录（§6）。

### 2.4 剪贴板历史（Capsule 最小子集）

契约（XSIME Capsule「临时」模块）：最近复制的文本/链接列表，点击选中、回车或双击粘贴；
保存到长期收藏、按类型筛选。首批只做**文本与链接**、只存本机。

落点：

- 纯逻辑放新 crate **`moodflow-clipboard`**：历史条目、去重、保留期限、条数上限、敏感内容排除。
- 平台采集走注入：`HostServices` 现有剪贴板三方法只覆盖读写；「监听变化」属于有事件推送通路
  的平台状态，按分层策略**走事件注入**，不在 trait 里加轮询。
- Windows：`AddClipboardFormatListener`，尊重 `ExcludeClipboardContentFromMonitorProcessing`
  等敏感格式标记（密码管理器会设）。
- Android：10+ 只有**获得焦点的默认输入法**能读剪贴板，因此历史只在键盘可见期间采集；
  遵守 `ClipDescription.EXTRA_IS_SENSITIVE`。
- 存储：redb（`moodflow-store`），**不写日志正文**（日志规范）。

### 2.5 方案与词库导入

XSIME 五端导入同一份标准 Rime ZIP（含 Lua/OpenCC/多方案依赖）。Moodflow 没有 librime，
**不能**完整执行 Rime 方案（Lua、speller 代数规则无法等价），因此目标收窄为：

- **词库导入**：Rime `.dict.yaml`（含 `import_tables`）与 UTF-8 TXT/TSV → Moodflow 用户词库或
  码表层。`moodflow-dict` 已能读 `.dict.yaml`，复用其解析。
- **方案转换**：从 Rime 方案中抽取可等价表达的部分（码表、码长、简码、拼音词库）生成 `.wpkg`，
  无法表达的部分（Lua、复杂 speller、filter）**明确列为诊断项**，不静默丢弃。
- 继承 XSIME 的安全规则：忽略 `__MACOSX`、`build/`、`sync/`、`*.userdb*`，不导入他人设备私有状态；
  同一份 ZIP 两端（Windows/Android）结果一致，以 SHA-256 标识。
- 落点：`moodflow-transfer`（`.wpkg` 格式已在这里），导入后的数据文件一律经覆盖解析函数定位。

### 2.6 九宫格拼音（Android）

契约（XSIME `NineKeyPinyin.swift` 与 Android 端行为）：数字键序列 → 所有可能拼音音节组合 →
候选；选九宫格布局时自动用拼音，选其他中文方案则回到全键盘。

落点：T9 序列到拼音的展开放 `moodflow-engine`（纯逻辑，以现有拼音音节表为准）；
`moodflow_android/` 只做键盘布局。Windows 不提供。

## 3. 两边都有的功能：吸收 XSIME 的长处

逐项对照了 Moodflow 现有实现（2026-10-17 本仓 `main`）。§3.1–3.4 是**已核实的差距**，
直接进阶段计划；§3.5–3.8 是**对照与核查项**，先核查再决定改不改。

### 3.1 AI 密钥不随新地址发出（Hub，Windows）

- **XSIME**：修改 API 地址后必须重新填写密钥，旧密钥不会转发到新地址；密钥存系统凭据管理器。
- **Moodflow**：ADR 0012 第 3 条规定 `secrets_set` 里「没填的密钥沿用旧值」，界面也提示「留空
  表示不改」。于是只改地址、不改密钥时，旧密钥会发给新地址——填错一个中转地址，密钥就出网了。
- **吸收**：写新 ADR 修订 0012 第 3 条：`base_url` 的主机变了而 `api_key` 留空时拒绝保存，
  要求重新输入；只改模型或同主机改路径时仍可沿用。按本仓约定，ADR 的「影响」里列出要改的
  产品书章节（08 FR-AIG-06、10 第 5.1 节），再改代码（`moodflow_hub` 的 gateway/secrets）。

### 3.2 用户词库打不开时要让用户知道（两端）

- **XSIME**：后台检查学习词库，发现损坏或读写错误时写 WARNING，并在状态菜单和设置页提示；
  不读取、不记录词条正文，**不自动删除、隔离或重建**；文档指导先整体备份，再用文本快照恢复。
- **Moodflow**：`moodflow-coordinator/src/construct.rs` 的 `open_user_store` 打开
  `userdata.redb` 失败时只 `warn!`，协调器退化为「不落盘」继续运行。用户照常打字，但用户词、
  词频、短语从此**静默**不再保存，直到有人翻日志。
- **吸收**：
  - 打开失败记成协调器状态，经 `UiCommand` 发一次提示（Windows 状态泡/toast，Android 状态行），
    设置页常驻显示，并通过 `moodflow-host` 的 `Readiness` 让 Android 宿主也拿得到。
  - 绝不自动删除或覆盖原库；提示里给出「先备份整个用户目录」的步骤，能只读打开时提供导出
    （复用 `moodflow-transfer` 的备份）。
  - 写一页恢复文档，参照 XSIME `docs/user-dictionary-recovery.md` 的结构。

### 3.3 输入统计按 XSIME 的口径实现（二批）

- **XSIME**：分成两个数据集。「日常统计」只存不含正文的计数（按键分布、成文字数、活跃时长，
  活跃时间只累计不超过 10 秒的连续间隔）；「打字测速」是显式开始的练习，有明确计分口径
  （有效速度、过程正确率、用有界编辑距离对齐的最终字准），练习不计入日常统计。
- **Moodflow**（2026-10-17 更正）：日常统计**早已实现**，在 `moodflow-store::stats` 与 `stat_collector`
  （字符分类、24 小时分布、码长、选重、活跃时长、按方案/来源，速度模型 v2 修掉了三条结构性偏差），
  RPC `stats.summary/daily/clear/pruneBefore`。先前「`coordinator/src/stats.rs` 只有三行注释」的判断
  只看了一个遗留空文件，是错的。缺的只有打字测速。
- **吸收**：新增 `moodflow-typing-practice`（纯逻辑计分：有效速度、过程正确率、有界编辑距离字准）；
  练习期间经 `StatCollector::pause` 暂停日常统计，练习的输入不计入日常统计。

### 3.4 安装数据逐文件校验（两端）

- **XSIME**：打包时为数据文件生成逐文件 SHA-256 清单，只放行白名单类型，拒收用户库、`build/`、
  `sync/`、编译缓存；传输后、写入安装目录前都重新校验；遇到同名但内容不同的文件默认停手
  （fail-closed），不覆盖。
- **Moodflow**：本仓 Rust 代码里除 Hub 外没有 SHA-256 校验，数据打包没有逐文件清单
  （安装器源码不在本仓，`scripts/pack-installer.sh` 经 `INSTALLER_DIR` 引用）。
- **吸收**：数据组装时生成清单；Windows 安装器写入前校验；Android 首次解包资产时校验。
  与 §2.5 的方案导入共用同一套白名单与拒收规则。

### 3.5 五笔与混输：用 XSIME 的用例做对照测试

Moodflow 已有对应能力（混输、满码唯一精确上屏、临时拼音、码表编码提示）。把 XSIME
`docs/wubi.md` 里的行为写成回归测试，跑出差异再逐条决定：

- 五笔 86 下 `zni` 查到「你」，`znihao` 查到「你好」，拼音候选带五笔码注释（如「你好 (wqvb)」），注释不上屏。
- 混输下 `wq` 和 `ni` 都能找到「你」；相同字母时完整五笔匹配优先。
- 四码唯一自动上屏只对**五笔词条**生效；拼音候选和 `z` 引导的查字不触发，长拼音不被四码截断。

### 3.6 Windows TSF：补自动化测试与两处核查

Moodflow 已注册 `IMMERSIVESUPPORT`，UI-less 候选已实施（`docs/design/game-compat-tsf-uielement.md`
记为「未真机」），`moodflow-bridge` 已处理 AppContainer 访问。XSIME 多做了三件事：

- **TSF 端到端自动测试**：隔离的 TSF + 服务 + 引擎测试，含 AppContainer 与 LPAC 两种令牌，启动前
  先验证令牌的实际访问边界（其 `Invoke-RimesImeE2E.ps1 -AppContainer`）。Moodflow 的 e2e 只有
  Linux 版（`scripts/linux/e2e.sh`）。→ 在 P2 补一个 Windows 版，至少覆盖普通桌面宿主与 AppContainer。
- **LPAC 客户端**：本仓 bridge 里没有 LPAC 相关处理（未找到 `S-1-15-2-2` / LPAC 字样）。
  → 核查管道 ACL 是否放行 LPAC 进程（如新版搜索框），需要时补上，仍保留对客户端 SID/会话/PID 的校验。
- **无组字时不请求写文档**：XSIME 发现某些宿主会拒绝不必要的写编辑会话，空闲时轻敲 Shift 因此
  丢失上下文，于是「只有修饰键状态变化、无组字无上屏」时不请求写编辑。→ 核查 `moodflow_tsf` 是否同样处理。

### 3.7 Android 键盘细节（P3 对照清单）

Android 应用尚未开始写（§4.1），以下作为从零搭建时的设计输入。XSIME Android 已验证、值得对照的做法：

- 标点表**一份数据、多端共用**，用同一个夹具文件在两端测试（Moodflow 可用 `moodflow-assoc` 的
  `punct` 模块承担这个角色）；中文下 123/符号页输出全角标点，但保留半角 `.` 给小数；
  打开数字页后第一个键若是句读标点，输完自动回到字母页。
- 触摸热区连续等宽并延伸到键帽间隙，热区与键帽几何分开定义。
- 换输入框、换模式后，旧的触摸流和排队的点击作废，迟到的抬起不会落到另一个键上。
- 宿主可能把旧输入框里已显示的组字留下，**绝不**把它重放到下一个输入框。
- Buffer 接近容量上限时只绘制可见块；组字变化时不重新测量已确认的块。
- 实时翻译在确认输入 400 ms 后自动运行，新编辑取消旧结果；私密/密码框不显示 AI 入口。

### 3.8 其他对照项（先核查，不预设要改）

- **竖排候选放不下时分列**：XSIME 竖排在屏幕高度不够时增加列，保住全部候选。Moodflow 候选窗已按
  工作区钳制宽度，竖排溢出时的处理需核查。
- **联想历史有上限**：XSIME 本机学习的后继词有固定上限（600 个词 × 每词 8 个后继），按最近使用淘汰。
  核查 `moodflow-assoc` 的 History 源在 redb 里是否有上限。
- **离线翻译**：XSIME Windows 用私有 CPU 运行时加中英模型，首次使用下载约 182 MiB 并校验 SHA-256，
  之后离线、无需密钥；Android 随包带本地中英词典，AI 可选。作为二批翻译方案的候选（见 §8 第 3 条）。

## 4. 双平台分工

| 层 | Windows | Android |
|---|---|---|
| 纯逻辑（共用） | `moodflow-buffer` / `moodflow-chord` / `moodflow-clipboard` / `moodflow-candidate` 矩阵 / `moodflow-transfer` 导入 / `moodflow-engine` T9 | 同左 |
| 协调器接入 | `Coordinator`（`desktop-ui` feature） | headless `Coordinator`，经 `moodflow-mobile` 门面 |
| 呈现 | `moodflow-ui`：Buffer 浮窗、矩阵候选窗、剪贴板历史条 | `moodflow_android/`：键盘内 Buffer 栏、候选区矩阵、剪贴板面板、九宫格布局 |
| 投递 | TSF edit session（`moodflow_tsf`） | `InputConnection.commitText`，失败保留 |
| 焦点/目标失效 | TSF 上下文切换、焦点事件 | `onStartInput` / `onFinishInput` / `onWindowHidden` |
| 设置界面 | 邻仓 `moodflow-setting`（新增配置键要跑 `wind_setting_assets` 对账） | `moodflow_android/` 设置页 |

`moodflow-mobile` 只新增**与绑定框架无关**的能力（Buffer/剪贴板/矩阵的门面方法）；
JNI 类型投影在 Android 工程里（`moodflow_android/rust/moodflow-android`）。

### 4.1 仓库与分支：照 XSIME，单仓单主线

XSIME 的 Android 端在主仓 `platforms/android/`，与其他平台共用 `main`，只开短期功能分支；
每个平台有自己的 CI 工作流（按路径过滤触发）、自己的 `VERSION` 和自己的标签（`android-v*`、
`windows-v*`），Android 发布标记为非 latest，不抢占主发布页。本仓照此执行：

- **Android 应用在本仓从零搭建**，放根目录 `moodflow_android/`，与 `moodflow_tsf/`、`moodflow_macos/` 对位；
  不另开独立的 Android 仓库。核心一改坏 Android，在同一个 PR 里就会红，而不是几天后在
  另一个仓库才发现（`moodflow-mobile` 模块文档里记的正是这个代价）。
- **不设长期平台分支**：所有平台都合入 `main`，功能在短期分支上开发，合并后删除分支。
- **CI 按路径触发**：新增 Android 工作流，路径过滤覆盖 `moodflow_android/**`、`moodflow_input/crates/`
  中 Android 依赖闭包内的 crate，以及共享夹具；Windows 工作流同理只看自己的路径。
- **版本与标签按平台独立**：产品版本仍以 `docs/VERSION` 为唯一真源；Android 另设
  `moodflow_android/VERSION`，主次版本号必须与产品版本一致，补丁号可独立递增且不低于产品版本的补丁号；
  标签用 `android-vX.Y.Z`，发布时不设为 latest。
- **现状**（2026-10-18）：`moodflow_android/` 已建并移植 XSIME 键盘与设置界面（见 §9），CI 工作流 `android.yml`
  每次构建 debug APK（产物 `moodflow-android-debug-apk`）；`moodflow_android/VERSION` 与对齐检查已在 CI；
  尚未打过 `android-v*` 标签，也没有正式签名。AGENTS.md 的仓库地图与构建命令已同步。

## 5. 阶段与验收

每阶段分别记录：源提交、构建命令、测试命令、设备/宿主、通过项、**未验证项**。
代码通过、真机可用、可发版分开验收，不用百分比代替证据（沿用 XSIME 的交付记录习惯）。

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0 准备**（约 1 周） | 许可证核查（§6）；六项功能各写一页设计文档放 `docs/design/`；确认 TSF keyup 能否扩展到字母键；§3.1 的 ADR；§3.5 五笔对照测试；§3.6–3.8 核查项出结论 | 设计文档与 ADR 合入；并击 Windows 可行性有结论；对照测试的差异逐条有处理决定 |
| **P1 纯逻辑**（约 3 周） | 四个新/扩展 crate + T9 + 导入转换器；§3.2 用户库异常状态与提示命令；§3.4 数据清单生成与校验，全部带单测 | `cargo test` 全绿且耗时合理（防「数据缺失假绿」）；`cargo check-headless`、`cargo check-android` 通过；CI `cargo tree` 断言不破 |
| **P2 Windows**（约 3 周） | 协调器接入 + `moodflow-ui` 呈现 + TSF 投递；配置键与设置页；§3.1 Hub 密钥规则；§3.6 Windows e2e 与核查结果的修复 | 编译机原生 MSVC 产物（不用交叉产物验收）；真实宿主（记事本、Word、浏览器、企业微信）连续输入；换框/失焦/密码框不串投；换 AI 地址不带旧密钥；用户库损坏时有可见提示且原库未被改动；`dev.ps1 ci` 通过 |
| **P3 Android**（约 5 周，从零搭建，可与 P2 并行） | 新建 `moodflow_android/` 工程 + `moodflow-mobile` 门面 + 键盘 UI + 九宫格；§3.7 对照清单；末尾做触屏并击 | 真机：Buffer 换字段/隐藏键盘清理、密码框禁用、剪贴板仅前台采集、九宫格与全键盘切换、物理键盘并击、旧组字不重放到新输入框 |
| **P4 二批** | §3.3 日常统计与打字测速、Buffer 上的 AI 生成/润色/翻译（复用 Hub AI 配置） | 另立计划 |

建议顺序：**Buffer → 候选矩阵 → 剪贴板历史 → 导入 → 九宫格 → 并击**。
Buffer 是 AI/翻译二批的地基；并击风险最高（TSF keyup），放最后，不阻塞其他项。

## 6. 许可证与归属

- XSIME 自有代码为 **Apache-2.0**，本仓为 **MIT**。默认做法是**按契约自行实现、不复制代码**。
  确需移植某段代码时：保留 Apache-2.0 版权头，在 `NOTICE.md` 登记来源，并标注修改——
  Apache-2.0 代码可以进 MIT 项目，但那部分仍按 Apache-2.0 分发。
- XSIME 的方案、词库、键位数据各有许可证（雾凇拼音等为 GPL-3.0，见其 `THIRD_PARTY_NOTICES.md`
  与 `rime-data/licenses/`）。**不直接收录其数据文件**；需要的数据回到原始上游按现有构建时下载
  机制处理（同 rime-frost 的做法）。
- 二期新增的第三方数据与库（CC-CEDICT 等）见 [xsime-extended-plan.md](xsime-extended-plan.md) §6。
- 「飞耀」并击键位在收录前单独确认来源与许可。**已确认（2026-10-18）**：XSIME 的出处说明记载为发布者
  提供、声明不属受版权保护的功能性映射，按 XSIME 仓库许可证分发；已转格式收为
  `moodflow-mobile/data/chord-builtin.json`，登记见 `NOTICE.md`。
- **2026-10-18 维护者决定**：Android 键盘界面允许直接移植 XSIME 代码（不只按契约重写），照上面第一条
  保留 Apache-2.0 文件头并在 `NOTICE.md` 登记；引擎仍是 Moodflow 核心，不引入 librime。

## 7. 明确不做

> 2026-10-18 修订（ADR 0057）：Mailbox、截图 / 录屏 / OCR、笔记 / 资源 / 密码库、iCloud 同步（改为自选文件夹同步）、LaTeX、文献引用，
> 以及 Android 上的 AI、本机翻译词典、插件商店与社区页、宠物轮换，**移入二期计划** [xsime-extended-plan.md](xsime-extended-plan.md)。

- 集成 librime 或执行 Rime Lua。
- 收录 XSIME 的方案与词库数据（GPL-3.0）。
- 插件注入原生界面（ADR 0054 第 7 条）。
- 电音演奏、摩斯电码。
- 对 macOS/Linux 宿主做任何改动（本计划只覆盖 Windows 与 Android；纯逻辑 crate 天然可被
  macOS 复用，但接入不在本期）。

## 8. 待定问题

> 2026-10-17 补充：产品方向定为可扩展、带插件市场，见 [插件系统设计](plugin-system.md)。
> 本计划里的并击键位、方案导入落为内容包；二批的 AI/翻译落为 `BufferAction` 插件。

1. ~~Android 平台仓是否独立~~ **已定（2026-10-17）：照 XSIME 的做法，单仓单主线**，见 §4.1。
2. 剪贴板历史在 Windows 上是否要做成全局快捷键（跨输入法可用）？XSIME 的做法是唤出时先切到
   自己的输入法，这在 Windows 上需要 TSF 激活配置文件，复杂度高，首批建议只在 Moodflow 为当前
   输入法时可用。
3. 二批的翻译能力：XSIME 在 macOS 用 Apple 本机翻译，Windows 用私有 CPU 运行时加中英模型
   （约 182 MiB，按需下载、校验后离线），Android 随包带本地中英词典（§3.8）。Moodflow 选哪条
   （本机模型 vs. 走 Hub 的 AI 渠道），需要做安装包体积、隐私与可用性评估。

## 9. 进度（2026-10-17 起，最近更新 2026-10-18）

各阶段一览（「代码」= 合入或在 PR 中且 CI 通过；「真机」= 在真实设备 / 宿主上验收）：

| 阶段 | 代码 | 真机 | 所在 |
|---|---|---|---|
| P0 准备 | ✅ 许可证核查、设计文档（buffer.md、plugin-system.md）、ADR 0054/0055/0056、五笔对照、§3.6–3.8 核查 | — | `main` |
| P1 纯逻辑 | ✅ 全部（见下表） | 不适用 | `main`（PR #96） |
| P2 Windows | ✅ 剪贴板历史、Buffer、并击、打字测速、候选矩阵、LPAC | ⬜ 一项都没验 | `main`（PR #97、#98） |
| P3 Android | ✅ 骨架 + XSIME 界面移植 | ⬜ 一项都没验 | 骨架在 `main`；界面移植在 PR #100 |
| P4 二批 | 🟡 打字测速已做；Buffer 上的 AI 生成 / 润色 / 翻译未做 | — | — |

**已完成（纯逻辑与可在 Linux 上验证的部分）**

| 项 | 落点 | 验证 |
|---|---|---|
| §2.1 Buffer | `crates/moodflow-buffer`，设计见 `docs/design/buffer.md` | 26 个单测；可编译到 aarch64-linux-android |
| §2.2 候选矩阵 | `moodflow-candidate::matrix` | 11 个单测 |
| §2.3 并击 | `crates/moodflow-chord`（键位 JSON、两级校验、结算器） | 14 个单测；Android 编译通过 |
| §2.4 剪贴板历史 | `crates/moodflow-clipboard` | 10 个单测；Android 编译通过 |
| §2.5 方案与词库导入 | `moodflow-transfer::rime_import`；`moodflow_input schema import-rime <zip> <目录>` | 10 个单测（含用现有 `.wpkg` 导入器安装）；端到端跑通 |
| §2.6 九宫格拼音 | `moodflow-engine::pinyin::t9` | 9 个单测（`64426` → ni hao） |
| §3.1 换主机须重填密钥 | Hub gateway + ADR 0055；产品书 V1.5.2 | gateway / Hub 111 个测试 |
| §3.2 用户库打不开提示 | 协调器 `notify_store_unavailable` / `user_store_available`，Android 门面同名方法 | 2 个单测；协调器全量 2182 通过 |
| §3.4 数据逐文件清单 | `crates/moodflow-manifest`，`data_manifest build/verify`，dev.sh / dev.ps1 接入 | 6 个单测；仓库 `data/` 实测 |
| §3.5 五笔对照 | `moodflow-engine/tests/xsime_wubi_parity.rs` | 4 条，与 XSIME 行为一致 |
| §3.8 联想历史上限 | `moodflow-store::assoc_history`（每上文 8 条、整表 4800 条） | 2 个新单测 |

**P2 Windows 接入（2026-10-17，未真机验证）**

| 项 | 落点 | 说明 |
|---|---|---|
| 剪贴板历史 | `moodflow-ui::clipboard_monitor`、协调器 `handle_clipboard` | `[clipboard] enabled`（出厂关）；`keys.clipboard_panel` 打开菜单式面板；密码管理器排除标记、隐私环境不记 |
| Buffer | `moodflow-ui::buffer_panel`、协调器 `handle_buffer` | `keys.toggle_buffer` 绑定 / `keys.buffer_send` 发送；已知差距见 `buffer.md` §4 |
| 并击 | 协调器 `handle_chord`、TSF `_DispatchChordKeyUp` | `[chord] enabled` + 用户目录方案 JSON；并击键集合经 `CONFIG_KEY_CHORD_KEYS` 推给 TSF，集合非空时才转发字母键 keyup |
| 打字测速 | `moodflow-typing-practice`；Hub 看板「打字测速」页（`practice_score` 命令） | 练习页在 Hub 里，焦点在 Moodflow 自己窗口时核心不记日常统计（`moodflow::focus_in_own_app`），练习不计入日常统计 |
| 候选矩阵 | 协调器 `handle_matrix`、候选窗网格排版（`SetCandidateGrid`） | `ui.candidate.matrix`（出厂关）；横排多页时 ↓ 展开最多 3 行、每行一页；←→ 行内、↑↓/翻页行间，首行 ↑ 或 Esc 收起保留组字；导航仍经 `NavKeys` 归一 |

**核查结论**

- §3.6 LPAC：**已补**（2026-10-17）。管道、共享内存与命名事件共用的 SDDL 原先只放行 `WD`、`SY`、`BA`、`AC`，
  LPAC 宿主（如新版搜索框）不在 `AC` 里、连不上。现加 `(A;;GRGWGX;;;S-1-15-2-2)`：读、写与等待（事件等待需要
  SYNCHRONIZE），不给建实例与改 ACL；单测见 `moodflow-bridge/src/security.rs`（仅 Windows 编译）。
  **待真机确认** LPAC 宿主里能出字。现有 `WD`/`AC` 的 `GA` 收窄为读写须另行在真机回归后再改，本次未动。
- §3.6 无组字时的写编辑会话：`UpdateComposition` 对文本与光标未变的请求直接跳过，模式切换不经它，预计已满足；需真机抓日志确认。
- §3.8 竖排溢出分列：候选窗按工作区钳制宽度与旋转态高度，未见「放不下时增加列」的逻辑，需真机看长竖排是否被截断。

**需要真机或本会话没有的条件才能推进**（2026-10-18 更新：P2 的代码已经写完，剩下的是验收）

- P2 Windows **真机验收**（§5 P2 一栏）：原生 MSVC 产物、记事本 / Word / 浏览器 / 企业微信连续输入、换框 / 失焦 / 密码框不串投、
  并击不吞键、LPAC 宿主出字。需要 Windows 桌面。
- P3 Android **真机验收**（§5 P3 一栏）：设备行为一项都还没验。需要至少两台不同分辨率的真机和一个物理键盘。
- §3.6 Windows TSF 端到端自动测试（含 AppContainer / LPAC 令牌）：需要 Windows 环境搭测试夹具。

## P3 Android 骨架（2026-10-17；部分缺口已在 2026-10-18 补上，见本节末尾的标注）

**已搭好**（`moodflow_android/`，构建与约定见 AGENTS.md「Android」一节）

| 项 | 落点 |
|---|---|
| 门面 | `moodflow-mobile` 新增 `buffer` / `clipboard` / `chord` / `t9` 模块与 `focus_gained_with`、`feed_text`、`replace_composition`（19 个单测） |
| JNI 绑定 | `moodflow_android/rust/moodflow-android`：独立 workspace，cdylib，结构化结果编成 `String[]`，日志接 logcat（8 个单测） |
| 纯 Kotlin 逻辑 | `imelogic`：字段隐私判据、输入框代际与 Buffer 目标、`Wire` 解码、键码映射、九宫格串、编辑指令派发（29 个 JVM 单测） |
| 应用 | `InputMethodService`：全键盘 / 九宫格 / 符号 / 数字布局，候选栏，Buffer 栏（绑定、逐块 / 全部投递、两段式授权），剪贴板面板（仅键盘可见时采集），物理键盘按键与并击接入，最小设置页（含并击方案导入） |
| 隐私 | 密码类与 `IME_FLAG_NO_PERSONALIZED_LEARNING` 字段完全绕过引擎、禁 Buffer、禁剪贴板采集；核心焦点同时按密码框上报 |
| CI | `android.yml`：产品名检查、版本号对齐、绑定层 fmt/clippy/单测、`assembleDebug testDebugUnitTest :imelogic:test`（含 JNI 契约对账）、APK 内 `.so` 与出厂数据核对、上传 debug APK |

**本机验证过的**：`cargo test -p moodflow-mobile`、绑定层 `cargo check/clippy --target aarch64-linux-android`
与 host 单测、`:imelogic:test`、JNI 契约测试（临时挂到 JVM 模块上跑通）、`app` 全部 Kotlin 源码对
`android-all`（API 34）编译通过。本机无 Android SDK/NDK，AGP 构建、`.so` 链接与 APK 打包只由 CI 验证。

**未验证（需真机，即 §5 P3 的验收项）**：Buffer 换字段 / 隐藏键盘清理、密码框禁用、剪贴板仅前台采集、
九宫格与全键盘切换、物理键盘并击、旧组字不重放到新输入框、核心在设备上的冷启动与出厂数据解包。

**已知缺口**

- 核心没有「本字段不学习但允许中文」的开关，所以无痕字段（`NO_PERSONALIZED_LEARNING`）暂时只能直接上屏英文；
  要放开需先在协调器加按字段关学习的开关。
- 九宫格只取第一种切分喂引擎（多切分按引擎候选质量排序未做）；部分上屏后剩余数字串与核心组字不同步。
- ~~出厂数据只含仓库 `data/`，APK 里没有拼音 / 五笔词库~~ **已补（2026-10-18，`main` 2840af6f）**：CI 先跑
  `MOODFLOW_BUILD_LOCAL=1 ./scripts/dev.sh gd` 组装 `build_dev/data`（按周缓存），Gradle `copyMoodflowData` 优先用它
  （也可 `-Pmoodflow.dataDir=` 指定），CI 检查 APK 里有 `rime_frost.dict.yaml` 与 `cn_dicts/base.dict.yaml`。
  §3.4 的清单校验**仍未**接到 Android 解包。
- ~~剪贴板历史先落 JSON 文件（`filesDir/rust/clipboard.json`）~~ **已改（PR #100）**：改走协调器，与 Windows 同一份
  `clipboard_history.json`。旧 `clipboard.json` 里的条目**没有迁移**。
- ~~触屏并击、候选矩阵、热区几何均未做；设置页只有宿主开关~~ **已做（PR #100，XSIME 界面移植）**，见下节。
  标点数据两端共用（§3.7 第一条）仍未核查。

## P3 Android：XSIME 界面移植（2026-10-18，PR #100，评审中、未合入）

键盘与设置页的界面直接移植 XSIME Android（Apache-2.0，文件清单见 `NOTICE.md`），引擎换成 Moodflow 核心。

| 项 | 落点 | 接到核心的方式 |
|---|---|---|
| 键区几何与绘制 | `imelogic/…/keyboard/KeyboardLayout` 等 + `app/…/ui/KeyboardSurface`、`KeyButton` | 26 键 / 九宫格 / 数字 / 符号 / 表情页，长按删除连删、上滑出数字；字符经核心 `char_strokes` 的按法送引擎 |
| 候选行 | `MoodflowImeService` | 每页 9 个（宿主翻页，`candidates(offset, 9)` 拉取）、点选 `select_candidate`、左侧设置面板、右侧 Buffer 开关、展开成矩阵（`CandidateMatrixView`，按需续取） |
| 配色 | `KeyboardTheme`（18 套）、`KeyboardAppearancePanel` | 宿主偏好；方案列表取核心已装方案 |
| Buffer 轨 | `BufferRail` | `moodflow-mobile::buffer`（换字段丢弃、同字段重启暂停、两段式投递、被拒保留可重试、隐藏键盘清空） |
| 剪贴板 | `TextClipboardPanel` | **改走协调器**：`UiEvent::ClipboardChanged` → `handle_clipboard`，与 Windows 同一份 `clipboard_history.json`、同一道 `clipboard.enabled` 与隐私闸门；门面新增列表 / 收藏 / 删除 / 清空。原 Android 专用的 `mobile::clipboard` JSON 存储删除 |
| 触屏并击 | `ChordSurface` / `ChordGesture` / `ChordProfile` | 手势模型在宿主，整组键交 `MobileChord::settle_batch` 结算；内置方案 `MobileChord::builtin`（XSIME 427 条 + 展开 14 条），全拼经 `feed_text` 进拼音引擎，完整音节之间补隔音符 |
| 九宫格 | 键位用 XSIME 的电话布局，「选拼音」行与分隔键 | `moodflow-mobile::t9`；退格按 XSIME 规则（删钉住音节的分隔符时还原成数字） |
| 设置 App | `SettingsActivity`（XSIME `SetupActivity` 的分组导航） | 方案 = 核心 `SELECT_SCHEMA`（落 `schema.active`）；剪贴板 = `clipboard.enabled`；插件 = `plugins.enabled` / `plugins.safe_mode` + `MobileCore::plugins`（已装、启用、加载、自动停用）；打字测速 = `mobile::practice`（与 Hub 同一套计分） |
| 核心共用 | `core/CoreHolder` | 键盘服务与设置页共用进程内唯一的核心（词库锁、环境变量挂载只能一份） |

**移动端强制项**：`chord.enabled`（物理键盘并击）在移动端恒关——软键盘只送按下不送松开，开着会吞掉字母。

**没有移植**：XSIME 的 AI（OpenAI 兼容服务、Mock）、本机翻译词典与 Buffer 插件快捷栏、官方插件商店与社区页、
宠物轮换动画、Rime 方案 ZIP 导入、并击映射的导入与编辑、`ExplicitClipboardImport`（显式粘贴到 Buffer，改为面板点条目）。

**本机验证过的**：`cargo test -p moodflow-mobile`、协调器剪贴板测试、门面 / 绑定层 clippy（含 aarch64-linux-android）、
`:imelogic:test`（含移植的 XSIME 几何、九宫格、并击手势测试）、`app` 全部 Java/Kotlin 对 android-all API 34 编译通过、
JNI 契约测试。AGP 构建与 APK 由 CI 验证；手势、连删、上滑、高度适配、剪贴板前台采集等一律需真机。

**已知缺口（门面 / 核心）**

- 九宫格「选拼音」只能钉住开头的音节（`t9::pin_leading`），后续音节的拼法选择要门面支持按位置钉住。
- ~~打字测速输入计入日常统计~~ **已补（A2，2026-10-22）**：Android 焦点事件携带应用包名，
  `com.moodflow.ime` 加入 `BUILTIN_EXCLUDED`，日常统计与 Moodflow 感知均排除此应用。
- 插件安装：Android 没有插件市场入口，只能列出与启停已装插件。
- 无痕字段（`IME_FLAG_NO_PERSONALIZED_LEARNING`）仍只能直接上屏英文：核心没有「本字段不学习但允许中文」的开关。
- 触屏并击只支持一次手势结算，XSIME 的「两段手势组合成一个音节」未做；并击方案的导入与编辑界面未移植。
- Rime 方案 ZIP 导入在 Android 上没有入口（Windows 只有命令行）。
- APK 只有 CI 产出的 debug 包：没有正式签名、没有 `android-v*` 发布流程。

## 10. 未完成项汇总（2026-10-18）

按「先让已写的东西被验证，再补功能」排序。「条件」一列说明推进它需要什么。

### 10.1 验收（最优先，代码已写完）

| # | 项 | 条件 | 关联 |
|---|---|---|---|
| V1 | Windows 真机验收：原生 MSVC 产物；记事本、Word、浏览器、企业微信连续输入；Buffer 换框 / 失焦 / 密码框不串投；剪贴板历史不记密码管理器内容；候选矩阵展开 / 收起；并击不吞键；用户库损坏时有提示且原库未改动；换 AI 地址不带旧密钥；`dev.ps1 ci` | Windows 桌面 | §5 P2 |
| V2 | LPAC 宿主（新版搜索框）里能出字；之后再把 `WD` / `AC` 的 `GA` 收窄为读写 | Windows 桌面 | §9 核查结论 |
| V3 | 无组字时不请求写编辑会话：抓日志确认 | Windows 桌面 | §3.6 |
| V4 | 竖排候选放不下时是否被截断；被截断则实现「增加列」 | Windows 桌面 | §3.8 |
| V5 | Android 真机验收：键位与高度适配（横竖屏、不同分辨率）、长按连删、上滑数字、九宫格与全键盘切换、候选栏翻页与矩阵、Buffer 换字段 / 隐藏键盘清空、密码框禁用 Buffer 与剪贴板、剪贴板只在键盘可见时采集、触屏并击、物理键盘输入、旧组字不重放到新输入框、核心冷启动与词库解包耗时 | 至少两台真机 + 物理键盘 | §5 P3 |
| V6 | PR #100 评审与合入 | 维护者 | — |

### 10.2 Windows 功能缺口

| # | 项 | 说明 |
|---|---|---|
| W1 | Buffer 按输入框区分目标 | TSF 在 `FocusGainedPayload` 里带文档 / 上下文身份；现在同进程换框识别不到，只能失焦即暂停（buffer.md §4） |
| W2 | 推送上屏回执 | 菜单「全部发送」走推送通道，没有回执，「结果不明 → 暂停不重发」区分不了 |
| W3 | 英文直输进 Buffer | 英文直输不经协调器出字，不会合进 Buffer |
| W4 | 长按逐块投递、节奏投递 | 长按约 1.2 秒（可配 500–3000 ms）逐块投递按下时冻结的队列；可选 1/2/3/5 秒节奏；最后一块后可自动关闭 |
| W5 | Windows TSF 端到端自动测试 | 隔离的 TSF + 服务 + 引擎，覆盖普通桌面、AppContainer、LPAC 令牌 |
| W6 | 剪贴板历史全局可用 | 现在只在 Moodflow 是当前输入法时可用；全局快捷键需要 TSF 激活配置文件（§8 待定问题 2） |
| W7 | Rime 导入的界面入口 | 现在只有命令行 `moodflow_input schema import-rime` |
| W8 | 并击方案编辑界面 | 复制内置方案、新建、导入 / 导出 JSON；现在只能手放 `chord_keymap.json` |
| W9 | 安装器写入前按清单校验 | 安装器源码不在本仓（`INSTALLER_DIR`），清单已能生成 |
| W10 | 用户词库恢复文档与只读导出 | 参照 XSIME `docs/user-dictionary-recovery.md`；能只读打开时复用 `moodflow-transfer` 导出 |

### 10.3 Android 功能缺口

| # | 项 | 说明 |
|---|---|---|
| A1 | 九宫格按位置钉住音节 | 现在只能钉住开头的音节（`t9::pin_leading`）；多切分按候选质量排序未做 |
| A2 | ✅ 练习不计入日常统计 | Android 焦点上报包名，`com.moodflow.ime` 在 `BUILTIN_EXCLUDED` 中 |
| A3 | 无痕字段允许中文 | 协调器加「本字段不学习」开关 |
| A4 | 插件安装 | 没有市场入口，只能列出与启停已装插件 |
| A5 | 两段手势并击、并击方案导入与编辑 | XSIME 有，未移植 |
| A6 | 旧 `clipboard.json` 迁移 | 骨架期存的条目未导入协调器历史 |
| A7 | 解包时按清单校验数据 | §3.4 |
| A8 | Rime 方案导入入口 | — |
| A9 | 标点数据两端共用、中文 123 页全角标点保留半角小数点 | §3.7；数字页句读后回字母页已有（`KeyboardLayout.returnsToLetters`），输入框代际已有（`FieldTracker`） |
| A10 | 正式签名与发版 | 签名密钥、`android-vX.Y.Z` 标签、发布说明；目前只有 debug APK |
| A11 | Android 的 Hub / 陪伴 | 不在本期（ADR 0056 第 4 条），以后另行决定 |

### 10.4 二批（P4）

| # | 项 | 说明 |
|---|---|---|
| B1 | Buffer 上的 AI 生成 / 润色 | 复用 Hub 的 AI 配置；落为 `BufferAction` 插件（plugin-system.md） |
| B2 | 实时翻译 | 先定路线：本机模型（XSIME Windows 约 182 MiB）还是走 Hub AI 渠道；评估包体积、隐私、可用性（§8 待定问题 3） |
| B3 | 插件系统剩余部分 | 见 [plugin-system.md](plugin-system.md) §11 |

### 10.5 二期（ADR 0057）

工作区（Mailbox）、Capsule 捕获与资料库、密码库、同步、LaTeX、文献引用、Android AI 与翻译、插件商店页与社区页、键盘宠物：
全部未开工，计划、契约与工作量（约 1,410 小时）见 [xsime-extended-plan.md](xsime-extended-plan.md)。
