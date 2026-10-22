# Moodflow — 现状与路线

> **最近核对：2026-10-18**（对照代码逐项核查，证据见各节链接）。本文是全仓的**现状总表 + 未完成项总清单**；
> 单项功能的细节以各自的设计文档为准。上一版（2026-06-17）只覆盖输入法核心的移植进度，已整体改写；
> 项目最初的蓝图见 [MIGRATION_PLAN.md](./MIGRATION_PLAN.md)（其 Phase 0–6 复选框早已过时，仅供查技术选型）。
> 需求、范围与验收以产品书为准：[young8290/xinqing](https://github.com/young8290/xinqing/tree/main/docs/product-book)（V1.6 起收录本页的 V1.x 内容）。

## 1. 一句话现状

- **Windows 输入法核心**：功能完整（拼音 / 双拼 / 五笔 / 混输、命令栏、快捷输入、反查、简繁、按应用配置、主题、导入导出、诊断 HUD）。
- **Moodflow 陪伴（Hub）**：产品书 V1.0 的 P0 / P1 功能大多已实现，多项未经真机与真实 AI 接口验收（见各角色交接文档）。
- **XSIME 吸收（ADR 0056）**：纯逻辑全部完成；Windows 接入的代码已合入；**一项都没在真机上验收**。
- **Android（ADR 0056）**：骨架与词库打包在 `main`；键盘与设置换成 XSIME 界面在 PR #100（CI 全绿、未合入、**未真机验证**）。
- **XSIME 吸收二期（ADR 0057，2026-10-18 立项）**：工作区（Mailbox）、Capsule 捕获与资料库、密码库、同步、LaTeX、文献引用、
  Android AI 与翻译、插件商店页、键盘宠物——**全部未开工**，计划见 [xsime-extended-plan.md](design/xsime-extended-plan.md)。
- **插件（ADR 0054）**：沙箱、注册表、市场客户端、协调器接入已完成；插件管理页、市场仓库、签名密钥未做。
- **最大的风险**：本仓的开发会话没有 Windows 桌面和 Android 真机，大量功能只经过 Linux 测试与 CI 编译。**下一步的重心是真机验收**，不是继续加功能。

## 2. 仓库规模（2026-10-18）

| 部分 | 规模 | 测试 |
|---|---|---|
| `moodflow_input/`（Rust 输入核心，37 个 crate + 4 个 app） | 约 39.9 万行 | 约 6,650 个 `#[test]` |
| 其中最大的 crate | coordinator 12.8 万、engine 6.0 万、ui 5.0 万、config 3.5 万 | 2,244 / 996 / 710 / 650 |
| `moodflow_hub/`（Tauri：core、gateway、src-tauri + Vue 前端） | Rust 约 4.5 万行 | Rust 约 570 个 + 前端单测 |
| `moodflow_tsf/`（C++ TSF） | 约 2.7 万行 | 11 个策略测试 |
| `moodflow_linux/`（Fcitx5 C++） | 约 1.0 万行 | 13 个测试文件 |
| `moodflow_macos/`（Swift IMKit） | 约 0.84 万行 | 有 |
| `moodflow_android/`（Kotlin / Java + JNI Rust） | Kotlin 约 0.36 万行、绑定 Rust 约 0.17 万行 | `imelogic` 59 个 JVM 单测 |
| 全仓 Rust | 约 44.8 万行 | — |

（`#[test]` 按 grep 计，可能略多。）

## 3. 平台现状

| 平台 | 形态 | 现状 | 主要缺口 |
|---|---|---|---|
| **Windows 10/11** | TSF 输入法 + Rust 服务 + Hub | 主平台，功能最全；安装包流水线在做（交接 A-10） | 新功能未真机验收；TSF 端到端自动测试没有；安装器在外部仓 |
| **Android** | `InputMethodService` + 设置应用 + JNI 直调 headless 协调器 | 预览；只有 CI 产出的 debug APK | 未真机验证；没有正式签名与发布；无 Hub；缺口见 §6.3 |
| macOS | Swift IMKit | 能用，有缺口；不在当前计划内 | 诊断 HUD 缺；英文模式打字统计延后；部分 `UiCommand` 未转发（`moodflow_macos/AGENTS.md` 记 37/51，枚举现已 62 个） |
| Linux | Fcitx5 插件（X11）+ deb 打包 | 能用（2026-10-02 合入 main）；不在当前计划内 | 原生 Wayland 候选窗未做（XWayland 下位置不对）；不支持多显示器工作区；无软键盘、HUD、按应用配置、全局热键（设计如此） |

## 4. 子系统现状（取代旧版「差距全景」表）

| 子系统 | 状态 | 证据 | 剩余 |
|---|---|---|---|
| 协调器 | ✅ | `moodflow-coordinator`，每个功能一个 `handle_*.rs` | — |
| 界面（候选窗 / 工具栏 / Tooltip / 状态泡 / Toast / 菜单 / 软键盘 / HUD） | ✅ | `moodflow-ui`；文本后端 DirectWrite / CoreText / fontconfig；工具栏自定义 2026-08-26 真机验证 | 主题 `border.style` 的虚线 / 点线解析了但画成实线 |
| 引擎（拼音 lattice + bigram、双拼、码表、混输、模糊音、T9、通配、简拼） | ✅ | `moodflow-engine`；模糊音 13 组规则在 `[schema.pinyin.fuzzy]`（出厂关） | 模糊音结果的排序质量未专门调优（待评估） |
| 词典 | ✅ | `moodflow-dict`：composite、trie、gramdb、草稿层、反查索引、权重归一 | — |
| 用户数据存储 | ✅ | `moodflow-store`：redb 的 user / temp / freq / shadow / stats / draft / assoc_history / abbrev_index | — |
| 配置 | ✅ | `moodflow-config`：三层合并、字段注册表、补丁、段级降级、写回闸门 | [deferred-config-features.md](deferred-config-features.md) 里的遗留项（§6.6） |
| 方案（schema 级按键 / 标点 / 候选 / 短语） | ✅ | `moodflow-config/src/schema.rs`；`schema-scoped-behavior.md` 四阶段已实施 | 未真机 |
| 主题 | 🟡 几乎完成 | `moodflow-theme`；`redesign/theme-migration-plan.md` T1–T6 全部完成；7 套内置主题 | 虚线 / 点线边框绘制；编辑器侧在外部仓 MoodflowThemeEditor |
| 命令栏（DSL + 内置函数） | 🟡 大部分 | `moodflow-cmdbar`：74 个内置函数 | 5 个仍是桩：`ask`、`pick`（`funcs/action.rs`）、`t2s`、`s2t`、`pinyin`（`funcs/text.rs`，原样返回；`moodflow-transform` 已有真正的转换器可接） |
| 设置 RPC | ✅ | `crates/moodflow-rpc`（system / config / compat / keys）+ `moodflow-webdata`（dict / temp / freq / shadow / stats / phrase / theme / schema / backup / scheme），热重载 `config.reload` | `apps/rpc` 是 22 行的废桩，应删除；设置界面在外部仓 `moodflow-setting`，未在本仓核对 |
| 表现层协议 | ✅ | `moodflow-ui-types` 的 `UiCommand`（62 个变体） | — |
| 简繁转换 | ✅ | `moodflow-transform/src/s2t.rs`（OpenCC），`[input.s2t]`，`keys.toggle_s2t` | 命令栏的 `s2t()` / `t2s()` 未接 |
| IPC / 桥接 | ✅ | `moodflow-ipc`、`moodflow-bridge`（管道、UDS、共享内存、命名事件、SDDL 含 LPAC） | LPAC 真机确认（§6.1） |
| 按键注入 | ✅ | `moodflow-keys/src/key_inject.rs` | Linux 不走 XTest（设计如此）；macOS 不能设 Caps Lock（系统限制） |
| 备份 / 导入导出 | ✅ | `moodflow-transfer`：bundle、merge、方案包 `.wpkg`、主题包、Rime 导入；`backup.*`、`dict.*`、`scheme.*`、`theme.*` RPC | `import-export-backup-plan-p4.md` 剩 1 项未勾（待核） |
| 按应用配置（前台窗口） | ✅ 核心 | `compat.toml` `[[apps]]`、`coordinator/app_schema.rs`、`compat.*` RPC | 设置端兼容页（`compat-settings-ui.md` 待写实现计划）；`set_shuangpin_layout` 会从旧配置重置当前方案（已知缺陷） |
| 系统字体枚举 | ✅ | `system.fonts` RPC（fontdb） | — |
| 性能 / 系统信息 | 🟡 | 按键耗时埋点 `moodflow/perf.rs`（`MF_PERF=1`）、`watchdog.rs`、`tools/typer`；`system.info` / `system.status` | 没有进程 CPU / 内存监控（是否需要待定）；TC-PERF-01 真机未出数 |
| 端到端测试 | 🟡 | `apps/service/tests/uds_e2e.rs`、协调器集成测试、渲染 golden、Linux / TSF 策略测试、`tools/typer`、`tools/mf-sim` | **没有驱动真实宿主的 Windows TSF 端到端测试** |
| 按键 / 按钮自定义（旧 S5） | ✅ | `ui.toolbar.items`、`keys.session_actions`、`keys.key_actions` + 方案级 `[key_actions]` | — |
| 方案切换重载 | 🟡 可能已 ✅ | 冷方案异步加载 + 代际校验、`schema_generation`、预热 | 见按应用配置一行的已知缺陷 |
| Tooltip / 状态提示 | ✅ | `coordinator/tooltip.rs`、`handle_tooltip.rs`、状态泡 `items` | — |
| 剪贴板历史 | 🟡 | `moodflow-clipboard` + `clipboard_monitor` + `handle_clipboard`；Android 同走协调器 | 未真机；存 JSON（未用 redb）；无跨输入法全局热键 |
| Buffer / 候选矩阵 / 并击 / 九宫格 / 打字测速 | 🟡 | 见 [XSIME 吸收计划](design/xsime-feature-port-plan.md) §9 | 未真机；缺口见 §6.2、§6.3 |
| 插件 | 🟡 | `moodflow-plugin-api` / `-host` / `-registry`、`moodflow-market`、`plugin_tool` | 见 §6.4 |
| Moodflow 感知与 MFP（`moodflow-tap`、`crates/mfp`） | ✅ 代码 | 交接 A-02～A-07 | 命名管道与 ACL 未在真 Windows 上跑过 |

## 5. 里程碑回顾

| 时间 | 里程碑 |
|---|---|
| 2026-06-16/17 | 输入核心重设计阶段 A（engine / dict / store / coordinator / config-schema 五份差分）、阶段 B（redb 落地、CompositeDict、拼音 lattice + bigram、词频与权重解耦）、按键管线 S0–S4 |
| 2026-06 下旬 | 模式 / 方案统一 M1–M5（`redesign/mode-scheme-unify.md`）；Web 设置数据桥（`moodflow-webdata`） |
| 2026-07～10（具体日期见各设计文档的状态行） | 主题 v3 迁移 T1–T6、工具栏自定义（08-26 真机验证）、按应用配置、码表通配 / 反查 / 整句、辅助码、导入导出 P1–P4、诊断 HUD、macOS IMKit 端 |
| 2026-10-02 | Linux（Fcitx5）端合入 main |
| 2026-10-03～15 | Moodflow 陪伴：产品书 V1.0～V1.5；Hub（桌宠、暖心话、对话与危机安全、日程待办、温柔改写、晚间小结、周信、日记、看板、设置、导入导出与删除、专注计时、晴天收集）；ADR 0007–0042 |
| 2026-10-17 | 改名 Moodflow（ADR 0045）；桌宠小组件（ADR 0044）；XSIME 吸收计划、插件系统设计、ADR 0054 / 0055；P1 纯逻辑全部完成（PR #96）；插件 M1–M3；Windows 接入剪贴板 / Buffer / 并击、LPAC 放行；Android 骨架与 CI（PR #97） |
| 2026-10-18 | Windows 候选矩阵、Hub 设置新字段与快捷键录制框、Hub 打字测速页（PR #98）；APK 打包完整词库；Android 换 XSIME 界面（PR #100，评审中）；ADR 0056 |

## 6. 未完成项总清单

排序原则：**先验收已写的，再补缺口，最后做新功能**。每项后面的链接是细节所在。

### 6.1 验收（最优先）

| # | 项 | 需要 | 细节 |
|---|---|---|---|
| 1 | 评审并合入 PR #100（Android XSIME 界面） | 维护者 | — |
| 2 | Windows 真机验收 XSIME 功能：Buffer、剪贴板历史、并击不吞键、候选矩阵、用户库异常提示、换 AI 地址须重填密钥；记事本 / Word / 浏览器 / 企业微信 | Windows 桌面 | [计划 §10.1](design/xsime-feature-port-plan.md) V1 |
| 3 | LPAC 宿主（新版搜索框）出字；之后收窄 `WD` / `AC` 的 `GA` | Windows 桌面 | 计划 §10.1 V2 |
| 4 | 无组字时不请求写编辑会话（抓日志）；竖排候选是否被截断 | Windows 桌面 | 计划 §10.1 V3、V4 |
| 5 | Android 真机验收（键位高度、连删、上滑、九宫格、矩阵、Buffer、剪贴板前台采集、触屏并击、物理键盘、旧组字不重放、冷启动耗时） | 两台以上真机 + 物理键盘 | 计划 §10.1 V5 |
| 6 | Moodflow 陪伴的真机与真实接口验收：并装、命名管道与 ACL、改写替换、锁屏与 UAC、工具栏、TC-PERF-01、兼容矩阵 | Windows 桌面、真实 AI 配置 | [交接 A](moodflow/handover/A-输入法.md) §8；[C 收尾清单](moodflow/handover/C-独立开发收尾清单.md) |
| 7 | 安装包真机试装（Win10 无 WebView2 代装、完成页、协议全文、快捷方式） | Windows 桌面 | 交接 A-10 |
| 8 | 真实 API 评测报告与人工评审（E-COMFORT 等、改写盲评、周信评审、安全用例） | 真实 AI 配置、E / D 角色 | C 收尾清单 |

### 6.2 Windows 输入功能缺口

| # | 项 | 细节 |
|---|---|---|
| 1 | Buffer 按输入框区分目标（TSF 焦点事件带文档身份） | 计划 §10.2 W1、[buffer.md](design/buffer.md) §4 |
| 2 | 推送上屏回执；英文直输进 Buffer；长按逐块 / 节奏投递 | W2–W4 |
| 3 | Windows TSF 端到端自动测试（普通桌面、AppContainer、LPAC） | W5 |
| 4 | 剪贴板历史跨输入法全局可用（待定） | W6 |
| 5 | Rime 导入、并击方案编辑的界面入口 | W7、W8 |
| 6 | 安装器按数据清单校验；用户词库恢复文档与只读导出 | W9、W10 |
| 7 | 命令栏 5 个桩函数（`ask`、`pick`、`t2s`、`s2t`、`pinyin`） | §4 命令栏一行 |
| 8 | 主题虚线 / 点线边框 | `redesign/theme-toml-migration-plan.md` §2 |
| 9 | `set_shuangpin_layout` 从旧配置重置当前方案 | `design/per-app-overrides.md` |
| 10 | 删除废桩 `apps/rpc` | §4 设置 RPC 一行 |

### 6.3 Android 缺口

| # | 项 | 细节 |
|---|---|---|
| 1 | 九宫格按位置钉住音节、多切分排序 | 计划 §10.3 A1 |
| 2 | ✅ 练习不计入日常统计（Android 焦点上报包名并加入内置排除） | A2 |
| 3 | 无痕字段允许中文（协调器「本字段不学习」开关） | A3 |
| 4 | 插件安装（与 Google Play 政策核查一起做） | A4、[插件 §11](design/plugin-system.md) P8 |
| 5 | 两段手势并击、并击方案导入与编辑、Rime 导入入口 | A5、A8 |
| 6 | 旧 `clipboard.json` 迁移；解包按清单校验 | A6、A7 |
| 7 | 标点数据两端共用；123 页全角标点保留半角小数点 | A9 |
| 8 | 正式签名、`android-vX.Y.Z` 发布流程 | A10 |
| 9 | Android 上的 Hub / 陪伴 | 不在本期（ADR 0056），以后另行决定 |

### 6.4 插件系统

见 [plugin-system.md](design/plugin-system.md) §11：内置功能改走注册表、内容包安装、设置声明渲染、未接的扩展点（联想来源、候选过滤 / 重排、缓冲区动作、事件订阅、声明式面板）、两端插件管理页、`moodflow-plugins` 索引仓库与审核 CI、正式签名密钥、Android 安装与政策核查、`BufferAction` 联网插件、L2 外部进程插件。

### 6.5 二批新功能

| # | 项 | 细节 |
|---|---|---|
| 1 | Buffer 上的 AI 生成 / 润色（复用 Hub AI 配置，做成 `BufferAction` 插件） | 计划 §10.4 B1 |
| 2 | 实时翻译（先定本机模型还是 Hub AI 渠道） | B2 |

### 6.5b XSIME 吸收二期（ADR 0057，全部未开工）

详见 [xsime-extended-plan.md](design/xsime-extended-plan.md)，工作量粗估约 1,410 小时。

| 阶段 | 内容 | 估算（h） |
|---|---|---|
| Q0 立约 | ADR 0057（已写）、产品书 V1.7（已写）、许可证清单、Android AI 网关路线、密码库与同步加密方案交 E 评审 | 30 |
| Q1 资料库 | 笔记、资源、文献条目与单条格式化（GB/T 7714 等）、DOI / BibTeX、本机中英词典翻译（CC-CEDICT） | 220 |
| Q2 密码库 | 真正静态加密（DPAPI / Windows Hello、Android Keystore / 生物识别、可选 Argon2id 主口令）、TOTP、15 / 60 秒遮蔽、60 秒清剪贴板 | 120 |
| Q3 捕获 | Windows 辅助进程：截图、滚动截图、OCR、二维码、贴图、标注、录屏、GIF；Android 对图片 OCR / 识码 | 380 |
| Q4 工作区 | 对话（共用 AI 网关与危机检测）、收件（本机回环 + MCP 只推不读、Android 分享）、终端（Windows ConPTY） | 220 |
| Q5 同步 | 用户自选文件夹 / WebDAV，默认端到端加密，墓碑、冲突副本、离线媒体不当作删除 | 200 |
| Q6 AI 动作 | Android AI 网关与快捷栏（翻译 / 快问 / 润色 / 作诗 / 画画）、LaTeX（AI + 离线 Unicode + 预览）、文献 Reference（防编造） | 180 |
| Q7 商店与宠物 | 两端插件商店页与社区页（依赖插件 M3）、Android 键盘宠物（用晴晴等桌宠素材） | 60 |

仍不做：摩斯电码、电音演奏；规则不变：不集成 librime、不收录 XSIME 的 GPL 数据、插件不带原生界面、macOS / Linux 宿主不在计划内。

### 6.6 配置与设置端遗留

| # | 项 | 细节 |
|---|---|---|
| 1 | 未实现的配置项：`input.auto_pair.blacklist`、`temp_pinyin.z_include_on_commit`、各 `*accent_color` / `mode_accent_border`、`ui.font.gdi_*` / `menu_*`、`ui.theme.editor_auto_start`、`features.stats.retain_days`、`alpha_providers.*` | [deferred-config-features.md](deferred-config-features.md) |
| 2 | 代码默认与出厂数据不一致：`chinese_pairs`（代码有「」『』、数据没有）、`english_pairs` 的 `<>`、`url.prefixes` 的 `bbs.` | 同上 |
| 3 | 设置端页面：兼容（按应用）页、辅助码来源行 | `design/compat-settings-ui.md`、`design/aux-code-settings-ui.md`（设置界面在外部仓 `moodflow-setting`） |
| 4 | 内置符号组文件 `data/system.symbols.toml` | `design/text-mapping-tables.md`（未实施） |
| 5 | 方案级用户数据 | `design/schema-scoped-user-data.md`（设计草案，待拍板） |

### 6.7 陪伴（Hub）侧

由各角色交接文档维护，本页不重复：[A 输入法](moodflow/handover/A-输入法.md)、[B 感知算法](moodflow/handover/B-感知算法.md)、
[C AI 与后端](moodflow/handover/C-AI与后端.md)、[C 收尾清单](moodflow/handover/C-独立开发收尾清单.md)、
[D 前端与视觉](moodflow/handover/D-前端与视觉.md)、[E 产品与质量](moodflow/handover/E-产品与质量.md)。
待评审的 ADR（如 0009）也记在那里。

### 6.8 过期文档

以下文档的状态行落后于代码（代码已实现，文档还写着「未实施」等），**以代码为准**，改动相关代码时顺手更新：
`design/session-key-actions.md`、`design/input-diagnostics-hud.md`（Windows 已实现，macOS / Linux 缺）、`design/auto-phrase-draft-layer.md`、
`design/pinyin-entry-boundary-contract.md`（部分实现）、`design/per-app-overrides.md`、`redesign/settings-status-board.md`（W3–W10 的核心 RPC 都已有）。
另外，`docs/design/*-plan.md` 里的复选框普遍没有勾选，不能当进度看。

## 7. 贯穿原则

- 每个改动用 `moodflow_input/scripts/dev.sh ci`（fmt-check + clippy + test）把关；Android 改动另过 `cargo check-android` 与 Android CI。
- **代码通过、真机可用、可发版分开验收**，不用百分比代替证据；没有真机验证的一律写明「未真机」。
- 新功能先进零平台依赖的纯逻辑 crate，再接各平台宿主（AGENTS.md「平台分层策略」）。
- 不照搬参考项目（Go 版、XSIME）的坏设计；吸收的是行为契约。移植代码时保留版权头并登记 `NOTICE.md`。
- 小改动直接提交 `main`，大改动和新模块走 PR；提交信息不带 AI 署名（AGENTS.md「提交纪律」）。

## 8. 文档索引

| 想知道 | 看 |
|---|---|
| 协作约定、构建、测试 | [AGENTS.md](../AGENTS.md) |
| XSIME 吸收与 Android 的计划、进度、未完成项 | [design/xsime-feature-port-plan.md](design/xsime-feature-port-plan.md) |
| XSIME 吸收二期（工作区、Capsule、密码库、同步、LaTeX、文献） | [design/xsime-extended-plan.md](design/xsime-extended-plan.md)、[ADR 0057](adr/0057-扩大XSIME吸收范围.md) |
| 插件系统 | [design/plugin-system.md](design/plugin-system.md)、[ADR 0054](adr/0054-插件信任模型与开放边界.md) |
| 为什么这样决定 | [adr/](adr/)（0007 起；0056 为 XSIME 与 Android） |
| 输入核心重设计 | [redesign/](redesign/) |
| 单项功能设计 | [design/](design/) |
| 陪伴部分与交接 | [moodflow/README.md](moodflow/README.md) |
| 第三方代码与数据 | [NOTICE.md](../NOTICE.md) |
