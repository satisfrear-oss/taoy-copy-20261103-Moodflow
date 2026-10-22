//! Moodflow：协调器钩子接到 `moodflow-tap` 的端到端测试（A-04～A-08，产品书 18 的 FR-SEN-01～06、
//! FR-OPS-03、FR-IME-02、FR-ENT-01/02/03、FR-RWR-01～04）。
//!
//! 装一个本机 TCP 的 Tap，测试扮演 Hub 握手并下发 `cfg{collect:true}`，再经 `MessageHandler`
//! 驱动 headless 协调器，断言 Hub 收到的上行事件。Tap 是进程级单例，所以整个文件只有一个测试。
//! 不依赖 build_dev 词库：没有词库时字母键照样进组字缓冲，Esc 照样撤销。

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mfp::Up;
use moodflow_bridge::handler::KeyAction;
use moodflow_bridge::handler::{FocusData, KeyEventData, MessageHandler};
use moodflow_config::Config;
use moodflow_coordinator::Coordinator;
use moodflow_coordinator::host_services::HostServices;
use moodflow_coordinator::moodflow::{self, HubView};
use moodflow_ipc::protocol::EVENT_KEY_DOWN;
use moodflow_ipc::protocol::{MOD_ALT, MOD_CTRL};
use moodflow_tap::guard::{GuardState, HubGuard, Launcher};
use moodflow_tap::{
    ByeReason, CompOp, Down, Endpoint, KeyKind, MoodState, OpenTarget, PauseBy, RewriteFailReason,
    RewriteOutcome, RewriteSource, RewriteStyle, Scope, Tap, TapConfig,
};
use moodflow_ui_types::{MenuCmd, MoodflowCell, MoodflowWeather, ToolbarAction, UiCommand};

const VK_A: u32 = 0x41;
const VK_B: u32 = 0x42;
const VK_P: u32 = 0x50;
const VK_R: u32 = 0x52;
const VK_1: u32 = 0x31;
const VK_TAB: u32 = 0x09;
const VK_SPACE: u32 = 0x20;
const VK_LEFT: u32 = 0x25;
const VK_LCONTROL: u32 = 0xA2;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const PASSWORD_MASK: u64 = 1 << 31;

fn key(k: u32) -> KeyEventData {
    KeyEventData {
        key_code: k,
        scan_code: 0,
        modifiers: 0,
        event_type: EVENT_KEY_DOWN,
        toggles: 0,
        event_seq: 0,
        prev_char: 0,
    }
}

fn focus(pid: u32, name: &str, input_scope_mask: u64) -> FocusData {
    FocusData {
        x: 100,
        y: 100,
        height: 20,
        composition_start_x: 0,
        composition_start_y: 0,
        client_token: u64::from(pid) << 32,
        input_scope_mask,
        disabled: false,
        reason: 0,
        caret_source: moodflow_ipc::protocol::caret_source::GUI_CARET,
        bundle_id: name.into(),
        window_class: String::new(),
    }
}

/// 测试用剪贴板：改写的第二来源（FR-RWR-01）。
#[derive(Default)]
struct Clip(Mutex<String>);

impl HostServices for Clip {
    fn clipboard_get_text(&self) -> anyhow::Result<String> {
        Ok(self.0.lock().unwrap().clone())
    }
}

struct Count(Arc<AtomicU32>);

impl Launcher for Count {
    fn launch(&mut self) -> std::io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// 目前为止 UI 通道上收到的 Moodflow 提示（状态泡指令里的文字与时长）。
fn tips(ui: &Receiver<UiCommand>) -> Vec<(String, u64)> {
    ui.try_iter()
        .filter_map(|c| match c {
            UiCommand::ShowStatusTip {
                text, duration_ms, ..
            } => Some((text, duration_ms)),
            _ => None,
        })
        .collect()
}

/// 用户层 config.toml 里 `moodflow.enabled` 的写盘值；未写则 `None`。
fn user_enabled(user: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(user.join("config.toml")).ok()?;
    let v: toml::Value = toml::from_str(&text).unwrap();
    v.get("moodflow")?.get("enabled")?.as_bool()
}

fn wait_until(mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ok() {
        assert!(Instant::now() < deadline, "等待超时");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Hub(TcpStream);

impl Hub {
    fn send(&mut self, d: &Down) {
        mfp::write_frame(&mut self.0, d).unwrap();
    }

    /// 下一条非心跳消息。
    fn recv(&mut self) -> Up {
        loop {
            match mfp::read_frame(&mut self.0) {
                Ok(Some(body)) => match mfp::decode_up(&body).unwrap() {
                    Up::Hb { .. } => continue,
                    up => return up,
                },
                Ok(None) => panic!("连接已关闭"),
                Err(mfp::FrameError::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    panic!("等待上行消息超时")
                }
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    /// 收到满足条件的消息为止，返回途中跳过的消息（供断言“没有某类消息”）。
    fn until(&mut self, mut ok: impl FnMut(&Up) -> bool) -> (Up, Vec<Up>) {
        let mut skipped = Vec::new();
        loop {
            let up = self.recv();
            if ok(&up) {
                return (up, skipped);
            }
            skipped.push(up);
        }
    }
}

#[test]
fn coordinator_events_reach_hub() {
    // 菜单“关闭 Moodflow 功能”会写用户配置：用户目录重定向到临时目录（同 password_force_english_persist）。
    // ⚠️ 目录名带 pid：多会话并行跑测试时固定名会互删夹具。
    let tmp = std::env::temp_dir().join(format!("moodflow_coord_moodflow-{}", std::process::id()));
    let user = tmp.join("UserData");
    let conf = tmp.join("datadir.conf");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("install").join("data")).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(&conf, user.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: 本文件仅此一个测试，env 在任何 OnceLock 初始化之前设置，无并发读者。
    unsafe {
        std::env::set_var("MOODFLOW_DATADIR_CONF", &conf);
        std::env::set_var("MOODFLOW_INSTALL_ROOT", tmp.join("install"));
    }
    assert_eq!(
        Config::user_config_dir(),
        Some(user.clone()),
        "前置条件：用户目录须已重定向，否则本测试会写真实用户配置"
    );

    let tap = Tap::start(
        TapConfig::new("0.1.0-test", Endpoint::Tcp("127.0.0.1:0".parse().unwrap())),
        Box::new(moodflow::downlink),
    )
    .unwrap();
    assert!(moodflow::install(Arc::clone(&tap)));
    // Hub 守护（A-06）：拉起动作换成计数，连接状态取自 Tap
    let launches = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&tap);
    let guard = HubGuard::start(
        Box::new(move || probe.is_linked()),
        Box::new(Count(Arc::clone(&launches))),
        true,
    )
    .unwrap();
    assert!(moodflow::install_guard(Arc::clone(&guard)));
    moodflow::watch_link(&guard);
    wait_until(|| launches.load(Ordering::SeqCst) == 1);
    assert_eq!(guard.state(), GuardState::Launching);

    let mut c = Config::default();
    c.input.default.chinese_mode = true;
    let (coord, ui) = Coordinator::new_headless_with_ui(c, None);
    let clip = Arc::new(Clip::default());
    coord.set_host_services(Arc::clone(&clip) as Arc<dyn HostServices>);
    moodflow::attach(&coord);

    let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut hub = Hub(s);
    hub.send(&Down::Hello {
        v: 1,
        hub_ver: "test".into(),
    });
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    hub.send(&Down::Cfg {
        collect: true,
        send_text: false,
        rewrite: false,
        app_blocklist: None,
        app_allowlist: None,
    });

    wait_until(|| guard.state() == GuardState::Connected);
    assert_eq!(launches.load(Ordering::SeqCst), 1, "连上后不再拉起");

    // 下行 mood、badge、pending 记进 hub_view，供工具栏与菜单显示
    hub.send(&Down::Mood {
        state: MoodState::Hesitant,
        offline: false,
    });
    hub.send(&Down::Badge { on: true });
    hub.send(&Down::Pending { count: 2 });
    wait_until(|| moodflow::hub_view().pending == 2);
    assert_eq!(
        moodflow::hub_view(),
        HubView {
            mood: Some(MoodState::Hesitant),
            offline: false,
            badge: true,
            pending: 2,
        }
    );
    // 主菜单“Moodflow”分组（FR-ENT-01）
    let labels = coord.debug_main_menu_labels();
    for want in [
        "和晴晴聊聊",
        "情绪看板",
        "待确认日程与待办（2）",
        "暂停感知",
        "Moodflow 设置",
        "关闭 Moodflow 功能",
    ] {
        assert!(labels.iter().any(|l| l == want), "缺 {want}：{labels:?}");
    }
    assert!(!labels.iter().any(|l| l.contains("点击重试")));
    coord.debug_run_menu_cmd(MenuCmd::MoodflowOpen(1));
    match hub.recv() {
        Up::Open { target, .. } => assert_eq!(target, OpenTarget::Dashboard),
        other => panic!("{other:?}"),
    }

    // 工具栏天气按钮（FR-ENT-02）：多云 + 小圆点；“未知”保持上一状态
    let cell = |weather, dot, dim| Some(MoodflowCell { weather, dot, dim });
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        cell(MoodflowWeather::Cloudy, true, false)
    );
    hub.send(&Down::Mood {
        state: MoodState::Unknown,
        offline: true,
    });
    wait_until(|| moodflow::hub_view().offline);
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        cell(MoodflowWeather::Cloudy, true, false)
    );
    hub.send(&Down::Mood {
        state: MoodState::Low,
        offline: false,
    });
    hub.send(&Down::Badge { on: false });
    wait_until(|| !moodflow::hub_view().badge);
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        cell(MoodflowWeather::Rain, false, false)
    );
    // 右键：暂停感知、情绪看板，末尾是回主菜单的“更多…”；左键：打开对话
    assert_eq!(
        coord.debug_toolbar_cell_menu_labels(ToolbarAction::Moodflow),
        Some(vec![
            "暂停感知".to_string(),
            "情绪看板".to_string(),
            String::new(),
            "更多…".to_string(),
        ])
    );
    coord.debug_toolbar_click(ToolbarAction::Moodflow);
    match hub.recv() {
        Up::Open { target, .. } => assert_eq!(target, OpenTarget::Chat),
        other => panic!("{other:?}"),
    }

    // 焦点：进程名来自缓存，闸门打开后先补发当前焦点
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let (up, _) = hub.until(|u| matches!(u, Up::Focus { .. }));
    match up {
        Up::Focus {
            app,
            scope,
            blocked,
            ..
        } => {
            assert_eq!(app.as_deref(), Some("notepad.exe"));
            assert_eq!(scope, Scope::Normal);
            assert!(!blocked);
        }
        _ => unreachable!(),
    }

    // Android Hub 以包名上报焦点（client token 没有 PID）：同一内置名单必须排除感知和日常统计。
    coord.handle_focus_gained(&focus(0, "COM.MOODFLOW.IME", 0));
    match hub.until(|u| matches!(u, Up::Focus { .. })).0 {
        Up::Focus { app, blocked, .. } => {
            assert_eq!(app, None, "命中排除名单时不应把应用名发给 Hub");
            assert!(blocked, "Hub 自身输入必须命中内置排除");
        }
        _ => unreachable!(),
    }
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let _ = hub.until(|u| matches!(u, Up::Focus { .. }));

    // 两个字母：key 先于 comp，第二个键 in_comp
    coord.handle_key_event_policed(&key(VK_A));
    match hub.recv() {
        Up::Key {
            kind, vk, in_comp, ..
        } => {
            assert_eq!(kind, KeyKind::Letter);
            assert_eq!(vk, Some(VK_A as u8));
            assert!(!in_comp);
        }
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, len, .. } => {
            assert_eq!(op, CompOp::Update);
            assert_eq!(len, Some(1));
        }
        other => panic!("{other:?}"),
    }
    // 光标旁气泡（FR-ENT-03）：正在组字时不显示
    hub.send(&Down::Tip {
        text: "📅 识别到日程".into(),
        ms: 1500,
    });
    hub.send(&Down::Pending { count: 3 });
    wait_until(|| moodflow::hub_view().pending == 3);
    assert!(tips(&ui).is_empty(), "组字时不该弹 Moodflow 提示");

    coord.handle_key_event_policed(&key(VK_B));
    match hub.recv() {
        Up::Key { in_comp, .. } => assert!(in_comp),
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, len, .. } => {
            assert_eq!(op, CompOp::Update);
            assert_eq!(len, Some(2));
        }
        other => panic!("{other:?}"),
    }

    // Esc 撤销组字
    coord.handle_key_event_policed(&key(VK_ESCAPE));
    match hub.recv() {
        Up::Key { kind, in_comp, .. } => {
            assert_eq!(kind, KeyKind::Esc);
            assert!(in_comp);
        }
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, .. } => assert_eq!(op, CompOp::Cancel),
        other => panic!("{other:?}"),
    }

    // 组字撤销后：提示照常显示，时长取 Hub 给的值；10 秒内第二条不显示
    hub.send(&Down::Tip {
        text: "📅 识别到日程".into(),
        ms: 1500,
    });
    hub.send(&Down::Pending { count: 4 });
    wait_until(|| moodflow::hub_view().pending == 4);
    assert_eq!(tips(&ui), vec![("📅 识别到日程".to_string(), 1500)]);
    hub.send(&Down::Tip {
        text: "📝 识别到待办".into(),
        ms: 2000,
    });
    hub.send(&Down::Pending { count: 5 });
    wait_until(|| moodflow::hub_view().pending == 5);
    assert!(tips(&ui).is_empty(), "10 秒内最多 1 条");

    // 回车上屏编码：key 之后只有一条 commit（send_text 关着，不带原文）；统计兜底不重报
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_B));
    coord.handle_key_event_policed(&key(VK_RETURN));
    let (up, _) = hub.until(|u| {
        matches!(
            u,
            Up::Key {
                kind: KeyKind::Enter,
                ..
            }
        )
    });
    assert!(matches!(up, Up::Key { in_comp: true, .. }));
    match hub.recv() {
        Up::Commit {
            chars, text, src, ..
        } => {
            assert_eq!(chars, 2);
            assert_eq!(text, None);
            assert_eq!(src, "raw_input");
        }
        other => panic!("{other:?}"),
    }

    // 密码框：上报 scope 后闸门关闭，之后的按键不出管道
    coord.handle_focus_gained(&focus(4343, "Login.exe", PASSWORD_MASK));
    match hub.recv() {
        Up::Focus { scope, .. } => assert_eq!(scope, Scope::Password),
        other => panic!("{other:?}"),
    }
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_ESCAPE));

    // 回到普通框：焦点之前不应夹着密码框里的按键
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let (_, skipped) = hub.until(|u| matches!(u, Up::Focus { .. }));
    assert!(skipped.is_empty(), "密码框里的事件漏出：{skipped:?}");

    // 安全桌面（C-PLT-05）：锁屏、UAC 提权框里焦点和按键都不出管道，回来后补发焦点
    coord.handle_focus_gained(&focus(4444, "LogonUI.exe", 0));
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_ESCAPE));
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let (up, skipped) = hub.until(|u| matches!(u, Up::Focus { .. }));
    assert!(skipped.is_empty(), "安全桌面里的事件漏出：{skipped:?}");
    assert!(matches!(up, Up::Focus { app: Some(ref a), .. } if a == "notepad.exe"));

    // 无痕快捷键（A-05，FR-SEN-06）：Ctrl+Alt+P 被吃掉，Hub 收到 pause_changed{hotkey}，
    // 之后的按键不出管道
    let mut ctrl_alt_p = key(VK_P);
    ctrl_alt_p.modifiers = MOD_CTRL | MOD_ALT;
    assert!(matches!(
        coord.handle_key_event_policed(&ctrl_alt_p),
        KeyAction::Consumed
    ));
    let (up, _) = hub.until(|u| matches!(u, Up::PauseChanged { .. }));
    assert!(matches!(
        up,
        Up::PauseChanged {
            on: true,
            by: PauseBy::Hotkey,
            ..
        }
    ));
    assert!(tap.paused());
    // 无痕时天气按钮淡显
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        cell(MoodflowWeather::Rain, false, true)
    );
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_ESCAPE));

    // 菜单恢复：pause_changed{menu}，随后补发焦点；中间不应夹着无痕期间的按键
    coord.debug_run_menu_cmd(MenuCmd::MoodflowTogglePause);
    let (up, skipped) = hub.until(|u| matches!(u, Up::PauseChanged { .. }));
    assert!(skipped.is_empty(), "无痕期间的事件漏出：{skipped:?}");
    assert!(matches!(
        up,
        Up::PauseChanged {
            on: false,
            by: PauseBy::Menu,
            ..
        }
    ));
    assert!(matches!(hub.recv(), Up::Focus { .. }));
    assert!(!tap.paused());

    // 温柔改写（A-08）：Hub 下发同意 ⑥ 之后，上屏的文字记进“最近上屏”
    hub.send(&Down::Cfg {
        collect: true,
        send_text: false,
        rewrite: true,
        app_blocklist: None,
        app_allowlist: None,
    });
    wait_until(|| tap.rewrite_enabled());
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_B));
    coord.handle_key_event_policed(&key(VK_RETURN));
    hub.until(|u| matches!(u, Up::Commit { .. }));
    let _ = tips(&ui);
    let phase = || coord.debug_moodflow_rewrite_phase();
    let mut ctrl_alt_r = key(VK_R);
    ctrl_alt_r.modifiers = MOD_CTRL | MOD_ALT;
    let enter_rewrite = |hub: &mut Hub| -> (u32, RewriteSource, String, Option<u32>) {
        assert!(matches!(
            coord.handle_key_event_policed(&ctrl_alt_r),
            KeyAction::Consumed
        ));
        let (up, skipped) = hub.until(|u| matches!(u, Up::RewriteReq { .. }));
        assert!(
            !skipped.iter().any(|u| matches!(
                u,
                Up::Key {
                    kind: KeyKind::Other,
                    ..
                }
            )),
            "改写快捷键本身不报 key，报了会先清掉最近上屏：{skipped:?}"
        );
        match up {
            Up::RewriteReq {
                req_id,
                source,
                text,
                style,
                replace_len,
                ..
            } => {
                assert_eq!(style, RewriteStyle::Gentle, "每次进入都从“更温和”开始");
                (req_id, source, text, replace_len)
            }
            _ => unreachable!(),
        }
    };
    let done = |hub: &mut Hub| match hub.until(|u| matches!(u, Up::RewriteDone { .. })).0 {
        Up::RewriteDone {
            req_id,
            chosen,
            outcome,
            ..
        } => (req_id, chosen, outcome),
        _ => unreachable!(),
    };

    // 来源一：最近上屏，替换原文（UTF-16 长度）
    let (req, source, text, replace_len) = enter_rewrite(&mut hub);
    assert_eq!(
        (source, text.as_str(), replace_len),
        (RewriteSource::Recent, "ab", Some(2))
    );
    assert_eq!(phase().as_deref(), Some("waiting:更温和"));
    hub.send(&Down::RewriteResult {
        req_id: req,
        style: RewriteStyle::Gentle,
        cands: vec!["甲".into(), "乙".into()],
    });
    wait_until(|| phase().as_deref() == Some("showing:更温和:甲|乙"));
    // 单按 Ctrl（为按 Ctrl+数字）不退出
    let mut ctrl = key(VK_LCONTROL);
    ctrl.modifiers = MOD_CTRL;
    coord.handle_key_event_policed(&ctrl);
    assert_eq!(phase().as_deref(), Some("showing:更温和:甲|乙"));
    // Tab 换风格：重新请求，迟到的旧结果不打扰
    assert!(matches!(
        coord.handle_key_event_policed(&key(VK_TAB)),
        KeyAction::Consumed
    ));
    let req2 = match hub.until(|u| matches!(u, Up::RewriteReq { .. })).0 {
        Up::RewriteReq {
            req_id,
            style,
            text,
            replace_len,
            ..
        } => {
            assert_eq!(style, RewriteStyle::Polite);
            assert_eq!((text.as_str(), replace_len), ("ab", Some(2)));
            assert_ne!(req_id, req);
            req_id
        }
        _ => unreachable!(),
    };
    assert_eq!(phase().as_deref(), Some("waiting:更礼貌得体"));
    hub.send(&Down::RewriteResult {
        req_id: req,
        style: RewriteStyle::Gentle,
        cands: vec!["迟到".into()],
    });
    hub.send(&Down::RewriteResult {
        req_id: req2,
        style: RewriteStyle::Polite,
        cands: vec!["丙".into()],
    });
    wait_until(|| phase().as_deref() == Some("showing:更礼貌得体:丙"));
    // 候选只有 1 个，按 2 不动
    assert!(matches!(
        coord.handle_key_event_policed(&key(VK_1 + 1)),
        KeyAction::Consumed
    ));
    assert!(phase().is_some());
    match coord.handle_key_event_policed(&key(VK_1)) {
        KeyAction::ReplaceBackward { count, text } => assert_eq!((count, text.as_str()), (2, "丙")),
        other => panic!("{other:?}"),
    }
    assert_eq!(phase(), None);
    assert_eq!(
        done(&mut hub),
        (req2, Some(0), RewriteOutcome::Replaced),
        "rewrite_done 不带文字"
    );
    assert!(
        tips(&ui).iter().any(|(t, _)| t == "已改写，Ctrl+Z 可撤销"),
        "替换后提示可撤销"
    );

    // 来源二：光标挪过（最近上屏作废），读剪贴板，插入
    *clip.0.lock().unwrap() = "  今天好累啊  ".into();
    coord.handle_key_event_policed(&key(VK_LEFT));
    let (req, source, text, replace_len) = enter_rewrite(&mut hub);
    assert_eq!(
        (source, text.as_str(), replace_len),
        (RewriteSource::Clipboard, "今天好累啊", None)
    );
    hub.send(&Down::RewriteResult {
        req_id: req,
        style: RewriteStyle::Gentle,
        cands: vec!["丁".into(), "戊".into()],
    });
    wait_until(|| phase().as_deref() == Some("showing:更温和:丁|戊"));
    match coord.handle_key_event_policed(&key(VK_SPACE)) {
        KeyAction::InsertText { text, .. } => assert_eq!(text, "丁", "空格选第 1 个"),
        other => panic!("{other:?}"),
    }
    assert_eq!(done(&mut hub), (req, Some(0), RewriteOutcome::Inserted));

    // Ctrl+数字只复制
    coord.handle_key_event_policed(&key(VK_LEFT));
    let (req, ..) = enter_rewrite(&mut hub);
    hub.send(&Down::RewriteResult {
        req_id: req,
        style: RewriteStyle::Gentle,
        cands: vec!["己".into(), "庚".into()],
    });
    wait_until(|| phase().is_some_and(|p| p.starts_with("showing")));
    let mut ctrl_2 = key(VK_1 + 1);
    ctrl_2.modifiers = MOD_CTRL;
    let _ = ui.try_iter().count();
    assert!(matches!(
        coord.handle_key_event_policed(&ctrl_2),
        KeyAction::Consumed
    ));
    assert!(
        ui.try_iter()
            .any(|c| matches!(c, UiCommand::CopyToClipboard(ref t) if t == "庚"))
    );
    assert_eq!(done(&mut hub), (req, Some(1), RewriteOutcome::Copied));

    // 表外的键：退出模式再照常处理；要交给宿主的键改成由 C++ 重放（它已经吃了这个键）
    let (req, ..) = enter_rewrite(&mut hub);
    assert!(matches!(
        coord.handle_key_event_policed(&key(VK_LEFT)),
        KeyAction::ClearCompositionThenPassThrough
    ));
    assert_eq!(phase(), None);
    assert_eq!(done(&mut hub), (req, None, RewriteOutcome::Cancelled));
    let (req, ..) = enter_rewrite(&mut hub);
    assert!(!matches!(
        coord.handle_key_event_policed(&key(VK_A)),
        KeyAction::Consumed | KeyAction::PassThrough | KeyAction::ClearCompositionThenPassThrough
    ));
    assert_eq!(done(&mut hub), (req, None, RewriteOutcome::Cancelled));
    let (up, _) = hub.until(|u| matches!(u, Up::Comp { .. }));
    assert!(
        matches!(
            up,
            Up::Comp {
                op: CompOp::Update,
                ..
            }
        ),
        "字母进了组字"
    );
    coord.handle_key_event_policed(&key(VK_ESCAPE));
    // 组字时按改写快捷键：先上屏再改写
    coord.handle_key_event_policed(&key(VK_A));
    let _ = tips(&ui);
    assert!(matches!(
        coord.handle_key_event_policed(&ctrl_alt_r),
        KeyAction::Consumed
    ));
    assert_eq!(phase(), None);
    assert_eq!(tips(&ui), vec![("先上屏再改写哦".to_string(), 2500)]);
    coord.handle_key_event_policed(&key(VK_ESCAPE));

    // Esc 取消
    let (req, ..) = enter_rewrite(&mut hub);
    assert!(matches!(
        coord.handle_key_event_policed(&key(VK_ESCAPE)),
        KeyAction::Consumed
    ));
    assert_eq!(done(&mut hub), (req, None, RewriteOutcome::Cancelled));

    // 危机内容：不改写，退出并邀请聊聊（FR-RWR-05 第 3 条）
    let (req, ..) = enter_rewrite(&mut hub);
    let _ = tips(&ui);
    hub.send(&Down::RewriteFail {
        req_id: req,
        reason: RewriteFailReason::Crisis,
    });
    assert_eq!(done(&mut hub), (req, None, RewriteOutcome::Failed));
    assert_eq!(phase(), None);
    assert!(tips(&ui).iter().any(|(t, _)| t.contains("和晴晴聊聊")));

    // 其余失败：候选框里提示 1.5 秒后退出
    let (req, ..) = enter_rewrite(&mut hub);
    hub.send(&Down::RewriteFail {
        req_id: req,
        reason: RewriteFailReason::Budget,
    });
    wait_until(|| phase().as_deref() == Some("failed:今天的改写次数用完了"));
    assert_eq!(done(&mut hub), (req, None, RewriteOutcome::Failed));
    assert_eq!(phase(), None);

    // 没同意 ⑥：不进模式，提示并请 Hub 打开设置问一次
    hub.send(&Down::Cfg {
        collect: true,
        send_text: false,
        rewrite: false,
        app_blocklist: None,
        app_allowlist: None,
    });
    wait_until(|| !tap.rewrite_enabled());
    let _ = tips(&ui);
    assert!(matches!(
        coord.handle_key_event_policed(&ctrl_alt_r),
        KeyAction::Consumed
    ));
    match hub.until(|u| matches!(u, Up::Open { .. })).0 {
        Up::Open { target, .. } => assert_eq!(target, OpenTarget::Settings),
        _ => unreachable!(),
    }
    assert_eq!(phase(), None);
    assert_eq!(
        tips(&ui),
        vec![("要先在 Moodflow 里同意“温柔改写”".to_string(), 2500)]
    );

    // Hub 正常退出（发 bye）：守护不重拉，菜单出现“点击重试”，点了才再拉起
    hub.send(&Down::Bye {
        reason: ByeReason::Shutdown,
    });
    wait_until(|| guard.state() == GuardState::HubQuit);
    assert_eq!(moodflow::hub_view(), HubView::default());
    // Hub 没连着：天气按钮淡显，回到默认的晴；左键改为重新拉起
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        cell(MoodflowWeather::Clear, false, true)
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(launches.load(Ordering::SeqCst), 1, "Hub 自己退出不重拉");
    let labels = coord.debug_main_menu_labels();
    assert!(
        labels.iter().any(|l| l == "Moodflow 组件未运行，点击重试"),
        "{labels:?}"
    );
    coord.debug_run_menu_cmd(MenuCmd::MoodflowRetryHub);
    wait_until(|| launches.load(Ordering::SeqCst) == 2);

    // 新的 Hub 连上
    let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut hub = Hub(s);
    hub.send(&Down::Hello {
        v: 1,
        hub_ver: "test".into(),
    });
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    wait_until(|| guard.state() == GuardState::Connected);

    // 菜单“关闭 Moodflow 功能”（FR-IME-02）：写盘，Hub 收到 bye{disabled}，守护不再拉起，
    // 菜单只剩“开启 Moodflow 功能”
    coord.debug_run_menu_cmd(MenuCmd::MoodflowToggleEnabled);
    let (up, _) = hub.until(|u| matches!(u, Up::Bye { .. }));
    assert!(matches!(
        up,
        Up::Bye {
            reason: ByeReason::Disabled,
            ..
        }
    ));
    assert_eq!(user_enabled(&user), Some(false));
    wait_until(|| guard.state() == GuardState::Idle);
    let labels = coord.debug_main_menu_labels();
    assert!(
        labels.iter().any(|l| l == "开启 Moodflow 功能"),
        "{labels:?}"
    );
    assert!(!labels.iter().any(|l| l == "和晴晴聊聊"));
    assert_eq!(
        coord.debug_moodflow_toolbar_cell(),
        None,
        "Moodflow 关着，天气按钮不画"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(launches.load(Ordering::SeqCst), 2, "关掉后不拉起");

    // 再打开：立即拉起，新的 Hub 照常连上
    coord.debug_run_menu_cmd(MenuCmd::MoodflowToggleEnabled);
    assert_eq!(user_enabled(&user), Some(true));
    wait_until(|| launches.load(Ordering::SeqCst) == 3);
    let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut hub = Hub(s);
    hub.send(&Down::Hello {
        v: 1,
        hub_ver: "test".into(),
    });
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    wait_until(|| guard.state() == GuardState::Connected);

    // 核心退出：Hub 收到 bye
    moodflow::shutdown();
    let (up, _) = hub.until(|u| matches!(u, Up::Bye { .. }));
    assert!(matches!(up, Up::Bye { .. }));
    let _ = std::fs::remove_dir_all(&tmp);
}
