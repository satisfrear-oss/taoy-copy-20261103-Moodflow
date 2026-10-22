//! 隐私闸门（04 FR-SEN-05，17 第 1.3 节真值表）。
//!
//! 字段全是原子量：MFP 读线程收到 `cfg` / `pause`、协调器报告焦点与安全桌面时写入，
//! 钩子只读，不加锁。

use std::sync::atomic::{AtomicBool, Ordering};

/// 出厂黑名单（FR-SEN-05，用户可改）。
pub const DEFAULT_BLOCKLIST: &[&str] = &[
    "*bank*",
    "*pay*",
    "KeePass*.exe",
    "1Password*.exe",
    "Bitwarden*.exe",
    "mstsc.exe",
];

/// 内置排除，不随黑名单修改（FR-SEN-05“内置排除”）：Hub 自身输入不进入感知。
pub const BUILTIN_EXCLUDED: &[&str] = &["moodflow_hub.exe", "com.moodflow.ime"];

const RELAXED: Ordering = Ordering::Relaxed;

#[derive(Debug, Default)]
pub struct Gate {
    enabled: AtomicBool,
    collect: AtomicBool,
    send_text: AtomicBool,
    rewrite: AtomicBool,
    paused: AtomicBool,
    /// 当前输入框 `scope` 为 `password` / `disabled`。
    scope_closed: AtomicBool,
    /// 当前进程命中黑名单、内置排除，或白名单非空且不在其中。
    app_blocked: AtomicBool,
    secure_desktop: AtomicBool,
}

impl Gate {
    /// 采集事件（key、commit、comp、cand）能否发出：真值表最后一行。
    pub fn allow(&self) -> bool {
        self.link_open() && !self.scope_closed.load(RELAXED) && !self.app_blocked.load(RELAXED)
    }

    /// `focus`、`ime` 能否发出：真值表中 scope 与名单两列关闭时仍要发一次 `focus` 显示“闭眼”。
    pub fn link_open(&self) -> bool {
        self.enabled.load(RELAXED)
            && self.collect.load(RELAXED)
            && !self.paused.load(RELAXED)
            && !self.secure_desktop.load(RELAXED)
    }

    /// `commit.text` 能否附带（FR-SEN-02 第 2 条）。
    pub fn send_text(&self) -> bool {
        self.send_text.load(RELAXED)
    }

    /// 改写能否使用，也决定“最近上屏”缓冲是否记录：同意 ⑥，且不在无痕、密码框、名单应用、安全桌面中。
    pub fn rewrite(&self) -> bool {
        self.enabled.load(RELAXED)
            && self.rewrite.load(RELAXED)
            && !self.paused.load(RELAXED)
            && !self.secure_desktop.load(RELAXED)
            && !self.scope_closed.load(RELAXED)
            && !self.app_blocked.load(RELAXED)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(RELAXED)
    }

    /// 只看同意 ⑥（Hub 下发的 `cfg.rewrite`），不看场景：改写快捷键据此区分“没同意”与“这里不能用”。
    pub fn rewrite_consented(&self) -> bool {
        self.rewrite.load(RELAXED)
    }

    pub fn paused(&self) -> bool {
        self.paused.load(RELAXED)
    }

    /// 当前进程是否命中名单（不看同意状态）。插件与剪贴板历史共用这一判断。
    pub fn app_blocked(&self) -> bool {
        self.app_blocked.load(RELAXED)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, RELAXED);
    }

    pub fn set_consent(&self, collect: bool, send_text: bool, rewrite: bool) {
        self.collect.store(collect, RELAXED);
        self.send_text.store(send_text, RELAXED);
        self.rewrite.store(rewrite, RELAXED);
    }

    /// Hub 断开：同意状态以 Hub 为准，没连上就当没有同意。
    pub fn clear_consent(&self) {
        self.set_consent(false, false, false);
    }

    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, RELAXED);
    }

    pub fn set_focus(&self, scope_closed: bool, app_blocked: bool) {
        self.scope_closed.store(scope_closed, RELAXED);
        self.app_blocked.store(app_blocked, RELAXED);
    }

    pub fn set_app_blocked(&self, on: bool) {
        self.app_blocked.store(on, RELAXED);
    }

    pub fn set_secure_desktop(&self, on: bool) {
        self.secure_desktop.store(on, RELAXED);
    }
}

/// 应用黑白名单：通配符 `*` / `?`，忽略大小写（FR-SEN-05 第 5、6 条）。
#[derive(Debug, Clone, Default)]
pub struct AppFilter {
    block: Vec<Vec<char>>,
    allow: Vec<Vec<char>>,
}

impl AppFilter {
    pub fn new<S: AsRef<str>>(blocklist: &[S], allowlist: &[S]) -> Self {
        let compile = |list: &[S]| {
            list.iter()
                .map(|p| p.as_ref().trim())
                .filter(|p| !p.is_empty())
                .map(fold)
                .collect()
        };
        Self {
            block: compile(blocklist),
            allow: compile(allowlist),
        }
    }

    pub fn set_blocklist<S: AsRef<str>>(&mut self, list: &[S]) {
        self.block = Self::new(list, &[]).block;
    }

    pub fn set_allowlist<S: AsRef<str>>(&mut self, list: &[S]) {
        self.allow = Self::new(&[], list).allow;
    }

    /// 进程名未知（`None`）时只有白名单非空才算关闭。
    pub fn blocked(&self, app: Option<&str>) -> bool {
        let Some(app) = app else {
            return !self.allow.is_empty();
        };
        let name = fold(app);
        if BUILTIN_EXCLUDED.iter().any(|b| fold(b) == name) {
            return true;
        }
        if self.block.iter().any(|p| glob(p, &name)) {
            return true;
        }
        !self.allow.is_empty() && !self.allow.iter().any(|p| glob(p, &name))
    }
}

/// 进程名是否命中一组通配符（`*` / `?`，忽略大小写），不含内置排除项。
/// 温柔改写的“只插入不替换”名单用它（`moodflow.rewrite_insert_apps`，ADR 0041）。
pub fn app_listed<S: AsRef<str>>(patterns: &[S], app: &str) -> bool {
    let name = fold(app);
    patterns
        .iter()
        .map(|p| p.as_ref().trim())
        .filter(|p| !p.is_empty())
        .any(|p| glob(&fold(p), &name))
}

fn fold(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// `*` 匹配任意串，`?` 匹配一个字符；回溯只记最近一个 `*`，线性时间。
fn glob(pat: &[char], s: &[char]) -> bool {
    let (mut p, mut i) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some((p, i));
            p += 1;
        } else if let Some((sp, si)) = star {
            p = sp + 1;
            i = si + 1;
            star = Some((sp, si + 1));
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_listed_uses_globs_without_builtin_exclusions() {
        assert!(app_listed(&["WINWORD.EXE", "*notes*"], "winword.exe"));
        assert!(app_listed(&["*notes*"], "OneNotesHelper.exe"));
        assert!(!app_listed(&["winword.exe"], "excel.exe"));
        assert!(!app_listed::<&str>(&[], "winword.exe"));
    }

    use super::*;

    fn gate(
        enabled: bool,
        collect: bool,
        paused: bool,
        scope: bool,
        app: bool,
        secure: bool,
    ) -> Gate {
        let g = Gate::default();
        g.set_enabled(enabled);
        g.set_consent(collect, false, false);
        g.set_paused(paused);
        g.set_focus(scope, app);
        g.set_secure_desktop(secure);
        g
    }

    /// 17 第 1.3 节真值表逐行；名单两列在这里合并为 `app_blocked`，匹配规则另测。
    #[test]
    fn truth_table() {
        // (enabled, collect, paused, scope 关闭, 名单关闭, 安全桌面) → (allow, link_open)
        let rows = [
            ((false, true, false, false, false, false), (false, false)),
            ((true, false, false, false, false, false), (false, false)),
            ((true, true, true, false, false, false), (false, false)),
            ((true, true, false, true, false, false), (false, true)),
            ((true, true, false, false, true, false), (false, true)),
            ((true, true, false, false, false, true), (false, false)),
            ((true, true, false, false, false, false), (true, true)),
        ];
        for ((e, c, p, s, a, d), want) in rows {
            let g = gate(e, c, p, s, a, d);
            assert_eq!((g.allow(), g.link_open()), want, "{e} {c} {p} {s} {a} {d}");
        }
    }

    #[test]
    fn rewrite_follows_its_own_consent() {
        let g = gate(true, true, false, false, false, false);
        assert!(!g.rewrite());
        g.set_consent(true, false, true);
        assert!(g.rewrite());
        g.set_focus(true, false);
        assert!(!g.rewrite(), "密码框里不记录也不改写");
        g.set_focus(false, false);
        g.set_paused(true);
        assert!(!g.rewrite());
        g.set_paused(false);
        g.clear_consent();
        assert!(!g.rewrite() && !g.allow());
    }

    #[test]
    fn factory_blocklist() {
        let f = AppFilter::new(DEFAULT_BLOCKLIST, &[]);
        for app in [
            "AliPay.exe",
            "ICBCBank.exe",
            "KeePassXC.exe",
            "1Password.exe",
            "Bitwarden.exe",
            "MSTSC.EXE",
        ] {
            assert!(f.blocked(Some(app)), "{app}");
        }
        for app in ["WeChat.exe", "notepad.exe", "Code.exe"] {
            assert!(!f.blocked(Some(app)), "{app}");
        }
        assert!(!f.blocked(None));
    }

    #[test]
    fn hub_is_always_excluded() {
        let f = AppFilter::new::<&str>(&[], &[]);
        assert!(f.blocked(Some("moodflow_hub.exe")));
        assert!(f.blocked(Some("Moodflow_Hub.EXE")));
        assert!(f.blocked(Some("com.moodflow.ime")));
        let f = AppFilter::new(&[], &["*"]);
        assert!(f.blocked(Some("moodflow_hub.exe")), "白名单放不开内置排除");
        assert!(
            f.blocked(Some("com.moodflow.ime")),
            "白名单放不开 Android Hub 内置排除"
        );
    }

    #[test]
    fn allowlist_closes_everything_else() {
        let f = AppFilter::new(&["wechat.exe"], &["WeChat.exe", "notepad*"]);
        assert!(f.blocked(Some("WeChat.exe")), "黑名单优先");
        assert!(!f.blocked(Some("Notepad.exe")));
        assert!(f.blocked(Some("Code.exe")));
        assert!(f.blocked(None), "白名单非空时未知进程关闭");
    }

    #[test]
    fn glob_rules() {
        let m = |p: &str, s: &str| glob(&fold(p), &fold(s));
        assert!(m("*", ""));
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "ac"));
        assert!(m("*pay*", "WePay"));
        assert!(m("k*e*.exe", "KeePassXC.exe"));
        assert!(!m("k*.exe", "KeePass.exe.bak"));
        assert!(m("**a**", "bab"));
        assert!(m("银行*", "银行助手.exe"));
    }
}
