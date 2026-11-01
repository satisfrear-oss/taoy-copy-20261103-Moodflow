//! 设置应用里的打字练习不计入日常统计（xsime-feature-port-plan §10.3 A2）。
//!
//! Android 焦点上报没有 PID，宿主以包名（`bundle_id`）给出；`com.moodflow.ime` 在
//! `moodflow_tap::BUILTIN_EXCLUDED` 里，焦点落在它上面时上屏不进每日统计。
//!
//! 单独成一个测试二进制：判定读的是进程级的焦点应用，与别的统计测试同进程跑会互相干扰。

use std::sync::Arc;

use moodflow_bridge::handler::{FocusData, MessageHandler};
use moodflow_config::Config;
use moodflow_coordinator::Coordinator;
use moodflow_ipc::protocol::caret_source;
use moodflow_store::Store;
use moodflow_store::stats::CommitSource;

fn focus_on(package: &str) -> FocusData {
    FocusData {
        x: 0,
        y: 0,
        height: 0,
        composition_start_x: 0,
        composition_start_y: 0,
        client_token: 1,
        input_scope_mask: 0,
        disabled: false,
        reason: 0,
        caret_source: caret_source::TSF_SELECTION,
        bundle_id: package.into(),
        window_class: String::new(),
    }
}

#[test]
fn practice_in_own_app_is_not_counted_but_other_apps_are() {
    let path = std::env::temp_dir().join(format!(
        "moodflow_practice_stats_{}.redb",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let store = Arc::new(Store::open(&path).unwrap());
    let coord = Coordinator::new_headless_with_store(Config::default(), None, store);

    // 练习页在本应用里：上屏不计数（stat_recorded 只在真正记了一笔时置位）。
    let _ = coord.handle_focus_gained(&focus_on("com.moodflow.ime"));
    coord.debug_record_commit("练习", 6, 0, CommitSource::Candidate);
    assert!(
        !coord.debug_stat_recorded(),
        "焦点在 Moodflow 自身应用时，练习输入不应进每日统计"
    );

    // 换到普通应用：照常计数。
    let _ = coord.handle_focus_gained(&focus_on("com.example.notes"));
    coord.debug_record_commit("你好", 4, 0, CommitSource::Candidate);
    assert!(
        coord.debug_stat_recorded(),
        "普通应用里的输入应照常进每日统计"
    );

    drop(coord);
    let _ = std::fs::remove_file(&path);
}
