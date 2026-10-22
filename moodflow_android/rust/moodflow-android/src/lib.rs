//! moodflow-android：Moodflow Android 输入法的 JNI 绑定层（产物 `libmoodflow_android.so`）。
//!
//! 只做三件事：类型投影（Rust ↔ JNI，结构化结果编成字符串数组，见 [`wire`]）、句柄生命周期、
//! 日志接到 logcat。**一行输入逻辑都不在这里**——那些在 `moodflow-mobile` 门面及其背后的核心里，
//! 这里只转发。
//!
//! # 句柄
//!
//! 每类对象（核心会话、Buffer、剪贴板历史、并击结算器）以 `Box` 指针转成 `jlong` 交给 Kotlin，
//! 对应的 `*Free` / `nativeDestroy` 释放。Kotlin 侧包装类保证释放后不再使用、且只释放一次。
//! `0` 恒表示无效句柄，所有入口对 `0` 返回默认值而不是解引用。
//!
//! # panic
//!
//! 每个入口包 [`guard`]：panic 越过 FFI 边界是未定义行为，这里把它截下来记一条日志、返回默认值。
//!
//! # Kotlin 对应
//!
//! 全部入口是 `com.moodflow.ime.jni.MoodflowNative` 上的 `@JvmStatic external fun`，
//! 签名与 `MoodflowNative.kt` 逐一对应；改这里必须同步改那边。

pub mod logcat;
pub mod sink;
pub mod wire;

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Arc, Mutex};

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jint, jintArray, jlong, jobjectArray, jstring};
use moodflow_mobile::buffer::{DEFAULT_CAPACITY_UTF16, MobileBuffer, WriteResult};
use moodflow_mobile::chord::{ChordOutput, MobileChord};
use moodflow_mobile::config_api::ConfigValue;
use moodflow_mobile::practice::Practice;
use moodflow_mobile::{MobileCommand, MobileCore};

// 目标类型来自 moodflow-buffer，经门面透出的只有 Target 的构造方式：(id, revision)。
use moodflow_mobile::buffer::Target;

// ── 通用辅助 ──────────────────────────────────────────────────

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        tracing::error!("JNI 入口 panic，已截获");
        default
    })
}

fn get_string(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn new_string(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn opt_string(env: &mut JNIEnv, s: Option<&str>) -> jstring {
    match s {
        Some(s) => new_string(env, s),
        None => std::ptr::null_mut(),
    }
}

fn string_array(env: &mut JNIEnv, items: &[String]) -> jobjectArray {
    let build = |env: &mut JNIEnv| -> jni::errors::Result<jobjectArray> {
        let arr = env.new_object_array(items.len() as i32, "java/lang/String", JObject::null())?;
        for (i, item) in items.iter().enumerate() {
            let s = env.new_string(item)?;
            env.set_object_array_element(&arr, i as i32, &s)?;
            env.delete_local_ref(s)?;
        }
        Ok(arr.into_raw())
    };
    build(env).unwrap_or(std::ptr::null_mut())
}

fn int_array(env: &mut JNIEnv, items: &[i32]) -> jintArray {
    let build = |env: &mut JNIEnv| -> jni::errors::Result<jintArray> {
        let arr = env.new_int_array(items.len() as i32)?;
        env.set_int_array_region(&arr, 0, items)?;
        Ok(arr.into_raw())
    };
    build(env).unwrap_or(std::ptr::null_mut())
}

fn jbool(b: bool) -> jboolean {
    if b { JNI_TRUE } else { JNI_FALSE }
}

fn into_handle<T>(v: T) -> jlong {
    Box::into_raw(Box::new(v)) as jlong
}

/// # Safety
/// `h` 必须是 [`into_handle::<T>`] 的返回值且尚未释放，或为 0。
unsafe fn handle_ref<'a, T>(h: jlong) -> Option<&'a T> {
    if h == 0 {
        None
    } else {
        // SAFETY: 由调用方保证 h 来自 into_handle::<T> 且仍存活。
        Some(unsafe { &*(h as *const T) })
    }
}

/// # Safety
/// 同 [`handle_ref`]，且释放后调用方不得再使用 `h`。
unsafe fn free_handle<T>(h: jlong) {
    if h != 0 {
        // SAFETY: 由调用方保证 h 来自 into_handle::<T> 且只释放一次。
        drop(unsafe { Box::from_raw(h as *mut T) });
    }
}

/// 核心会话：`Arc` 是因为门面构造就返回 `Arc<MobileCore>`。
struct Session {
    core: Arc<MobileCore>,
}

fn session<'a>(h: jlong) -> Option<&'a MobileCore> {
    // SAFETY: Kotlin 侧 MoodflowCore 只传 nativeCreate 返回且未 destroy 的句柄。
    unsafe { handle_ref::<Session>(h) }.map(|s| s.core.as_ref())
}

fn with_locked<T, R>(h: jlong, default: R, f: impl FnOnce(&mut T) -> R) -> R {
    // SAFETY: Kotlin 侧包装类只传对应 *New 返回且未释放的句柄。
    let Some(m) = (unsafe { handle_ref::<Mutex<T>>(h) }) else {
        return default;
    };
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

// ── 核心会话 ──────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeInit(
    _env: JNIEnv,
    _class: JClass,
) {
    guard((), logcat::init);
}

/// 构造核心。**耗时**（加载词库、编译方案），Kotlin 侧必须在后台线程调用。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeCreate(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    user_root: JString,
    fallback_schema: JString,
    sink: JObject,
) -> jlong {
    guard(0, || {
        logcat::init();
        let data_dir = get_string(&mut env, &data_dir);
        let user_root = get_string(&mut env, &user_root);
        let fallback = get_string(&mut env, &fallback_schema);
        let (Ok(vm), Ok(target)) = (env.get_java_vm(), env.new_global_ref(&sink)) else {
            tracing::error!("nativeCreate: 无法取得 JavaVM 或回调引用");
            return 0;
        };
        let sink = Arc::new(sink::JniSink::new(vm, target));
        let core = MobileCore::new(Path::new(&data_dir), Path::new(&user_root), &fallback, sink);
        tracing::info!("核心已创建");
        into_handle(Session { core })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    // SAFETY: Kotlin 侧 MoodflowCore.close() 只调一次，之后句柄置 0。
    guard((), || unsafe { free_handle::<Session>(handle) });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeKeyDown(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    vk: jint,
    modifiers: jint,
) -> jobjectArray {
    let enc = guard(None, || {
        session(handle).map(|c| wire::outcome(&c.key_down(vk as u32, modifiers as u32)))
    });
    string_array(&mut env, &enc.unwrap_or_else(|| vec![String::new()]))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeFocusGained(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    is_private: jboolean,
    host_app: JString,
) {
    let host_app = get_string(&mut env, &host_app);
    guard((), || {
        if let Some(c) = session(handle) {
            c.focus_gained_with(is_private != JNI_FALSE, &host_app);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeFocusLost(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    guard((), || {
        if let Some(c) = session(handle) {
            c.focus_lost();
        }
    });
}

/// `cmd`：0 英文 / 1 选方案(int_arg) / 2 标点 / 3 全半角 / 4 简繁 / 5 重载配置 / 6 双拼布局(str_arg)。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeRunCommand(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    cmd: jint,
    int_arg: jint,
    str_arg: JString,
) {
    let s = get_string(&mut env, &str_arg);
    guard((), || {
        let Some(c) = session(handle) else { return };
        let command = match cmd {
            0 => MobileCommand::English,
            1 => MobileCommand::SelectSchema(int_arg.max(0) as usize),
            2 => MobileCommand::TogglePunct,
            3 => MobileCommand::ToggleWidth,
            4 => MobileCommand::ToggleS2t,
            5 => MobileCommand::ReloadConfig,
            6 => MobileCommand::SetShuangpinLayout(s),
            _ => {
                tracing::warn!("未知命令 {cmd}");
                return;
            }
        };
        c.run_command(command);
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeUserStoreAvailable(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    // 句柄无效时报「可用」：不该因为宿主自己的时序问题弹出「词库坏了」的提示
    jbool(guard(true, || {
        session(handle).is_none_or(|c| c.user_store_available())
    }))
}

/// 每项 `id US 名称 US 短称 US 是否激活(1/0)`；下标即选方案命令的参数。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeSchemas(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.schemas()
                    .into_iter()
                    .map(|s| wire::join(&[&s.id, &s.name, &s.icon_label, wire::flag(s.active)]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

/// 第 0 项 `offset US total US selected`，其后为候选文本。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeCandidates(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    offset: jint,
    limit: jint,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        let Some(c) = session(handle) else {
            return vec![wire::join(&["0", "0", "0"])];
        };
        let w = c.candidates(offset.max(0) as usize, limit.max(0) as usize);
        let mut out = Vec::with_capacity(w.items.len() + 1);
        out.push(wire::join(&[
            &w.offset.to_string(),
            &w.total.to_string(),
            &w.selected.to_string(),
        ]));
        out.extend(w.items);
        out
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeSelectCandidate(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    index: jint,
) -> jobjectArray {
    let enc = guard(None, || {
        session(handle).map(|c| wire::outcome(&c.select_candidate(index.max(0) as usize)))
    });
    string_array(&mut env, &enc.unwrap_or_else(|| vec![String::new()]))
}

/// 每项 `id US 名称 US 是否激活(1/0)`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeThemes(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.themes()
                    .into_iter()
                    .map(|t| wire::join(&[&t.id, &t.name, wire::flag(t.active)]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeSetTheme(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    id: JString,
) -> jboolean {
    let id = get_string(&mut env, &id);
    jbool(guard(false, || {
        session(handle).is_some_and(|c| c.set_theme(&id))
    }))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeThemeStyle(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let s = guard(String::from("system"), || {
        session(handle)
            .map(|c| c.theme_style())
            .unwrap_or_else(|| "system".into())
    });
    new_string(&mut env, &s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeSetThemeStyle(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    style: JString,
) {
    let style = get_string(&mut env, &style);
    guard((), || {
        if let Some(c) = session(handle) {
            c.set_theme_style(&style);
        }
    });
}

/// 每项 `语义名 US ARGB(十进制无符号)`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePalette(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    system_dark: jboolean,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.palette(system_dark != JNI_FALSE)
                    .into_iter()
                    .map(|p| wire::join(&[&p.name, &p.argb.to_string()]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeIsDark(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    system_dark: jboolean,
) -> jboolean {
    let dark = system_dark != JNI_FALSE;
    jbool(guard(dark, || {
        session(handle).map_or(dark, |c| c.is_dark(dark))
    }))
}

/// 三元组平铺：`[码点, VK, 是否 Shift(1/0), …]`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeCharStrokes(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jintArray {
    let flat = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.char_strokes()
                    .into_iter()
                    .flat_map(|(ch, vk, shift)| [ch as i32, vk as i32, i32::from(shift)])
                    .collect()
            })
            .unwrap_or_default()
    });
    int_array(&mut env, &flat)
}

/// 每项 `键名 US 字符`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeSymbolKeys(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.symbol_keys()
                    .into_iter()
                    .map(|(name, ch)| wire::join(&[&name, &ch.to_string()]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeConfigSnapshot(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let s = guard(String::new(), || {
        session(handle)
            .map(|c| c.config_snapshot())
            .unwrap_or_default()
    });
    new_string(&mut env, &s)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeConfigDefaults(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let s = guard(String::new(), || {
        session(handle)
            .map(|c| c.config_defaults())
            .unwrap_or_default()
    });
    new_string(&mut env, &s)
}

/// 每项 `键 US 类型 US 枚举值(以 U+001E 分隔)`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeConfigRegistry(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.config_registry()
                    .into_iter()
                    .map(|f| wire::join(&[&f.key, &f.ty, &f.options.join(&wire::RS.to_string())]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

/// `ty`：0 bool（"true"/"false"）/ 1 int / 2 float / 3 str / 4 字符串列表（U+001F 分隔）。
/// 返回 `null` 表示成功，否则为失败原因（给用户看）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeConfigSet(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    key: JString,
    ty: jint,
    value: JString,
) -> jstring {
    let key = get_string(&mut env, &key);
    let value = get_string(&mut env, &value);
    let err = guard(Some("内部错误".to_string()), || {
        let Some(c) = session(handle) else {
            return Some("输入法核心尚未就绪".to_string());
        };
        let v = match parse_config_value(ty, &value) {
            Ok(v) => v,
            Err(e) => return Some(e),
        };
        c.config_set(&key, v).err()
    });
    opt_string(&mut env, err.as_deref())
}

fn parse_config_value(ty: jint, value: &str) -> Result<ConfigValue, String> {
    Ok(match ty {
        0 => ConfigValue::Bool(value == "true"),
        1 => ConfigValue::Int(value.parse().map_err(|_| "不是整数".to_string())?),
        2 => ConfigValue::Float(value.parse().map_err(|_| "不是数字".to_string())?),
        3 => ConfigValue::Str(value.to_string()),
        4 => ConfigValue::StrList(if value.is_empty() {
            Vec::new()
        } else {
            value.split(wire::US).map(str::to_string).collect()
        }),
        _ => return Err(format!("未知值类型 {ty}")),
    })
}

/// 每项 `id US 名称 US 是否激活(1/0)`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeShuangpinLayouts(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.shuangpin_layouts()
                    .into_iter()
                    .map(|l| wire::join(&[&l.id, &l.name, wire::flag(l.active)]))
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeFeedText(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    text: JString,
) -> jobjectArray {
    let text = get_string(&mut env, &text);
    let enc = guard(None, || {
        session(handle).map(|c| wire::outcome(&c.feed_text(&text)))
    });
    string_array(&mut env, &enc.unwrap_or_else(|| vec![String::new()]))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeReplaceComposition(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    text: JString,
) -> jobjectArray {
    let text = get_string(&mut env, &text);
    let enc = guard(None, || {
        session(handle).map(|c| wire::outcome(&c.replace_composition(&text)))
    });
    string_array(&mut env, &enc.unwrap_or_else(|| vec![String::new()]))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClearComposition(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let enc = guard(None, || {
        session(handle).map(|c| wire::outcome(&c.clear_composition()))
    });
    string_array(&mut env, &enc.unwrap_or_else(|| vec![String::new()]))
}

// ── 九宫格（无状态）─────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeT9Segmentations(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
    limit: jint,
) -> jobjectArray {
    let input = get_string(&mut env, &input);
    let items = guard(Vec::new(), || {
        moodflow_mobile::t9::segmentations(&input, limit.max(1) as usize)
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeT9Leading(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
) -> jobjectArray {
    let input = get_string(&mut env, &input);
    let items = guard(Vec::new(), || {
        moodflow_mobile::t9::leading_syllables(&input)
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeT9Pin(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
    syllable: JString,
) -> jstring {
    let input = get_string(&mut env, &input);
    let syllable = get_string(&mut env, &syllable);
    let pinned = guard(None, || moodflow_mobile::t9::pin_leading(&input, &syllable));
    opt_string(&mut env, pinned.as_deref())
}

// ── Buffer ──────────────────────────────────────────────────

fn edit_code(r: Result<(), moodflow_mobile::buffer::EditError>) -> jint {
    use moodflow_mobile::buffer::EditError;
    match r {
        Ok(()) => 0,
        Err(EditError::Empty) => 1,
        Err(EditError::CapacityExceeded) => 2,
        Err(EditError::Busy) => 3,
    }
}

fn target(id: jlong, revision: jlong) -> Target {
    Target::new(id as u64, revision as u64)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferNew(
    _env: JNIEnv,
    _class: JClass,
    capacity: jint,
) -> jlong {
    guard(0, || {
        let cap = if capacity > 0 {
            capacity as usize
        } else {
            DEFAULT_CAPACITY_UTF16
        };
        into_handle(Mutex::new(MobileBuffer::new(cap)))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferFree(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    // SAFETY: Kotlin 侧 NativeBuffer.close() 只调一次。
    guard((), || unsafe { free_handle::<Mutex<MobileBuffer>>(h) });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferBind(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    id: jlong,
    revision: jlong,
    is_private: jboolean,
) -> jboolean {
    jbool(guard(false, || {
        with_locked(h, false, |b: &mut MobileBuffer| {
            b.bind(target(id, revision), is_private != JNI_FALSE)
                .is_ok()
        })
    }))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferFocusChanged(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    id: jlong,
    revision: jlong,
) {
    guard((), || {
        with_locked(h, (), |b: &mut MobileBuffer| {
            b.focus_changed(target(id, revision))
        })
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferSuspend(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    guard((), || {
        with_locked(h, (), |b: &mut MobileBuffer| b.suspend())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferDiscard(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    guard((), || {
        with_locked(h, (), |b: &mut MobileBuffer| b.discard())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferReleaseHold(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    guard((), || {
        with_locked(h, (), |b: &mut MobileBuffer| b.release_hold())
    });
}

/// 返回 0 成功 / 1 空 / 2 超容量 / 3 块在投递中。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferPushPhrase(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    text: JString,
) -> jint {
    let text = get_string(&mut env, &text);
    guard(-1, || {
        with_locked(h, -1, |b: &mut MobileBuffer| {
            edit_code(b.push_phrase(&text))
        })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferPushLatin(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    code_point: jint,
) -> jint {
    guard(-1, || {
        let Some(ch) = char::from_u32(code_point as u32) else {
            return 1;
        };
        with_locked(h, -1, |b: &mut MobileBuffer| edit_code(b.push_latin(ch)))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferBackspace(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_locked(h, -1, |b: &mut MobileBuffer| edit_code(b.backspace()))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferDeleteLast(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_locked(h, -1, |b: &mut MobileBuffer| {
            edit_code(b.delete_last_block())
        })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferClear(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    guard((), || with_locked(h, (), |b: &mut MobileBuffer| b.clear()));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferBlocks(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jobjectArray {
    let blocks = guard(Vec::new(), || {
        with_locked(h, Vec::new(), |b: &mut MobileBuffer| b.blocks())
    });
    string_array(&mut env, &blocks)
}

/// 0 未绑定 / 1 就绪 / 2 暂停 / 3 结果不明待确认。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferState(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jint {
    use moodflow_mobile::buffer::State;
    guard(0, || {
        with_locked(h, 0, |b: &mut MobileBuffer| match b.state() {
            State::Unbound => 0,
            State::Ready => 1,
            State::Suspended => 2,
            State::Held => 3,
        })
    })
}

/// 0 只确认组字 / 1 投递下一块 / 2 交给宿主。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferSendKeyAction(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    composing: jboolean,
) -> jint {
    use moodflow_mobile::buffer::SendKeyAction;
    guard(2, || {
        with_locked(h, 2, |b: &mut MobileBuffer| {
            match b.send_key_action(composing != JNI_FALSE) {
                SendKeyAction::ConfirmComposition => 0,
                SendKeyAction::SendNext => 1,
                SendKeyAction::PassThrough => 2,
            }
        })
    })
}

/// 取出要投递的文本；不能投递（未绑定、暂停、目标不符、待确认、空）时返回 `null`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferBegin(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    id: jlong,
    revision: jlong,
    all: jboolean,
) -> jstring {
    let text = guard(None, || {
        with_locked(h, None, |b: &mut MobileBuffer| {
            b.begin(target(id, revision), all != JNI_FALSE).ok()
        })
    });
    opt_string(&mut env, text.as_deref())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferAuthorized(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    id: jlong,
    revision: jlong,
) -> jboolean {
    jbool(guard(false, || {
        with_locked(h, false, |b: &mut MobileBuffer| {
            b.is_authorized(target(id, revision))
        })
    }))
}

/// `result`：0 已接收 / 1 被拒（保留重试）/ 2 不明（保留并暂停）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeBufferFinish(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
    result: jint,
) -> jboolean {
    let r = match result {
        0 => WriteResult::Accepted,
        1 => WriteResult::Rejected,
        _ => WriteResult::Unknown,
    };
    jbool(guard(false, || {
        with_locked(h, false, |b: &mut MobileBuffer| b.finish(r))
    }))
}

// ── 剪贴板历史（走核心，与 Windows 同一份存储）────────────────

/// 宿主读到的一次剪贴板变化；是否收录由核心决定（开关、隐私、敏感、空白、过长）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClipboardChanged(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    text: JString,
    sensitive: jboolean,
) {
    let text = get_string(&mut env, &text);
    guard((), || {
        if let Some(c) = session(handle) {
            c.clipboard_changed(&text, sensitive != JNI_FALSE);
        }
    });
}

/// 每项 `id US 类型(t/l) US 收藏(1/0) US 正文`，收藏的在前、各自新到旧。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClipboardEntries(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.clipboard_entries()
                    .into_iter()
                    .map(|e| {
                        let kind = if e.link { "l" } else { "t" };
                        wire::join(&[&e.id.to_string(), kind, wire::flag(e.pinned), &e.text])
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClipboardSetPinned(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    id: jlong,
    pinned: jboolean,
) -> jboolean {
    jbool(guard(false, || {
        session(handle).is_some_and(|c| c.clipboard_set_pinned(id as u64, pinned != JNI_FALSE))
    }))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClipboardRemove(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    id: jlong,
) -> jboolean {
    jbool(guard(false, || {
        session(handle).is_some_and(|c| c.clipboard_remove(id as u64))
    }))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeClipboardClearUnpinned(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    guard((), || {
        if let Some(c) = session(handle) {
            c.clipboard_clear_unpinned();
        }
    });
}

// ── 插件 ────────────────────────────────────────────────────

/// 每项 `id US 版本 US 已启用 US 已加载 US 自动停用 US 能力(RS 分隔) US 名称`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePlugins(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        session(handle)
            .map(|c| {
                c.plugins()
                    .into_iter()
                    .map(|p| {
                        let caps = p.capabilities.join(&wire::RS.to_string());
                        wire::join(&[
                            &p.id,
                            &p.version,
                            wire::flag(p.enabled),
                            wire::flag(p.loaded),
                            wire::flag(p.auto_disabled),
                            &caps,
                            &p.name,
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    string_array(&mut env, &items)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePluginReenable(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    id: JString,
) -> jboolean {
    let id = get_string(&mut env, &id);
    jbool(guard(false, || {
        session(handle).is_some_and(|c| c.plugin_reenable(&id))
    }))
}

// ── 打字测速 ────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePracticeNew(
    mut env: JNIEnv,
    _class: JClass,
    target: JString,
) -> jlong {
    let target = get_string(&mut env, &target);
    guard(0, || into_handle(Mutex::new(Practice::new(&target, 0))))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePracticeFree(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    // SAFETY: Kotlin 侧 NativePractice.close() 只调一次。
    guard((), || unsafe { free_handle::<Mutex<Practice>>(h) });
}

/// 输入框内容变了。`t_ms` 从练习开始算起。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePracticeText(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    t_ms: jlong,
    text: JString,
) {
    let text = get_string(&mut env, &text);
    guard((), || {
        with_locked(h, (), |p: &mut Practice| {
            p.on_text(t_ms.max(0) as u64, &text)
        })
    });
}

/// 成绩 `用时ms US 范文字数 US 最终字数 US 编辑距离 US 最终字准 US 过程正确率 US 有效速度`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativePracticeFinish(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    finished_ms: jlong,
) -> jstring {
    let out = guard(None, || {
        with_locked(h, None, |p: &mut Practice| {
            let s = p.finish(finished_ms.max(0) as u64);
            Some(wire::join(&[
                &s.elapsed_ms.to_string(),
                &s.target_chars.to_string(),
                &s.typed_chars.to_string(),
                &s.distance.to_string(),
                &s.final_accuracy.to_string(),
                &s.process_accuracy.to_string(),
                &s.effective_cpm.to_string(),
            ]))
        })
    });
    opt_string(&mut env, out.as_deref())
}

// ── 并击 ────────────────────────────────────────────────────

/// 应用级校验不过时返回 0（错误原因用 `nativeChordValidate` 取）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordNew(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jlong {
    let json = get_string(&mut env, &json);
    guard(0, || match MobileChord::from_json(&json) {
        Ok(c) => into_handle(Mutex::new(c)),
        Err(e) => {
            tracing::warn!("并击方案无法应用: {e}");
            0
        }
    })
}

/// 内置默认方案（XSIME 触屏并击的 427 条映射，见 `moodflow_mobile::chord`）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordBuiltin(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    guard(0, || into_handle(Mutex::new(MobileChord::builtin())))
}

/// 应用级校验：`null` = 可以应用，否则为原因（不含方案内容）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordValidate(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let json = get_string(&mut env, &json);
    let err = guard(Some("内部错误".to_string()), || {
        MobileChord::from_json(&json).err()
    });
    opt_string(&mut env, err.as_deref())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordFree(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    // SAFETY: Kotlin 侧 NativeChord.close() 只调一次。
    guard((), || unsafe { free_handle::<Mutex<MobileChord>>(h) });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordIsChordKey(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    key: JString,
) -> jboolean {
    let key = get_string(&mut env, &key);
    jbool(guard(false, || {
        with_locked(h, false, |c: &mut MobileChord| c.is_chord_key(&key))
    }))
}

/// 结算结果编码：`null` = 未结算；`N` = 无映射；`Es<拼音>` 完整音节 / `Ef<拼音>` 片段。
fn chord_wire(o: Option<ChordOutput>) -> Option<String> {
    o.map(|o| match o {
        ChordOutput::Emit { pinyin, fragment } => {
            format!("E{}{pinyin}", if fragment { 'f' } else { 's' })
        }
        ChordOutput::NoMatch => "N".to_string(),
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordKeyDown(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    key: JString,
    now_ms: jlong,
) -> jstring {
    let key = get_string(&mut env, &key);
    let out = guard(None, || {
        with_locked(h, None, |c: &mut MobileChord| {
            chord_wire(c.key_down(&key, now_ms.max(0) as u64))
        })
    });
    opt_string(&mut env, out.as_deref())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordKeyUp(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    key: JString,
    now_ms: jlong,
) -> jstring {
    let key = get_string(&mut env, &key);
    let out = guard(None, || {
        with_locked(h, None, |c: &mut MobileChord| {
            chord_wire(c.key_up(&key, now_ms.max(0) as u64))
        })
    });
    opt_string(&mut env, out.as_deref())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordTick(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    now_ms: jlong,
) -> jstring {
    let out = guard(None, || {
        with_locked(h, None, |c: &mut MobileChord| {
            chord_wire(c.tick(now_ms.max(0) as u64))
        })
    });
    opt_string(&mut env, out.as_deref())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordReset(
    _env: JNIEnv,
    _class: JClass,
    h: jlong,
) {
    guard((), || with_locked(h, (), |c: &mut MobileChord| c.reset()));
}

/// 每项 `片段(1/0) US 键名(空格分隔，按方案键区顺序) US 输出`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordMappings(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        with_locked(h, Vec::new(), |c: &mut MobileChord| {
            c.mappings()
                .iter()
                .map(|m| wire::join(&[wire::flag(m.fragment), &m.keys.join(" "), &m.output]))
                .collect()
        })
    });
    string_array(&mut env, &items)
}

/// `[左手键名(空格分隔), 右手键名(空格分隔)]`。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordHands(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
) -> jobjectArray {
    let items = guard(Vec::new(), || {
        with_locked(h, Vec::new(), |c: &mut MobileChord| {
            vec![c.left_keys().join(" "), c.right_keys().join(" ")]
        })
    });
    string_array(&mut env, &items)
}

/// 触屏手势：一组同时落下又全部抬起的键（空格分隔的键名）一次结算，编码同 [`chord_wire`]；
/// 单个键返回 `null`（宿主按普通键处理）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_nativeChordSettleBatch(
    mut env: JNIEnv,
    _class: JClass,
    h: jlong,
    keys: JString,
    now_ms: jlong,
) -> jstring {
    let keys = get_string(&mut env, &keys);
    let out = guard(None, || {
        let keys: Vec<String> = keys.split_whitespace().map(str::to_owned).collect();
        with_locked(h, None, |c: &mut MobileChord| {
            chord_wire(c.settle_batch(&keys, now_ms.max(0) as u64))
        })
    });
    opt_string(&mut env, out.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_value_parsing() {
        assert!(matches!(
            parse_config_value(0, "true"),
            Ok(ConfigValue::Bool(true))
        ));
        assert!(matches!(
            parse_config_value(1, "42"),
            Ok(ConfigValue::Int(42))
        ));
        assert!(parse_config_value(1, "x").is_err());
        match parse_config_value(4, "a\u{1F}b") {
            Ok(ConfigValue::StrList(v)) => assert_eq!(v, vec!["a", "b"]),
            other => panic!("{other:?}"),
        }
        match parse_config_value(4, "") {
            Ok(ConfigValue::StrList(v)) => assert!(v.is_empty()),
            other => panic!("{other:?}"),
        }
        assert!(parse_config_value(9, "").is_err());
    }

    #[test]
    fn chord_wire_encoding() {
        assert_eq!(chord_wire(None), None);
        assert_eq!(chord_wire(Some(ChordOutput::NoMatch)).as_deref(), Some("N"));
        assert_eq!(
            chord_wire(Some(ChordOutput::Emit {
                pinyin: "zh".into(),
                fragment: true
            }))
            .as_deref(),
            Some("Efzh")
        );
    }

    #[test]
    fn zero_handles_are_inert() {
        assert!(session(0).is_none());
        assert!(!with_locked(0, false, |_: &mut MobileBuffer| true));
    }

    #[test]
    fn buffer_handle_round_trip() {
        let h = into_handle(Mutex::new(MobileBuffer::default()));
        let ok = with_locked(h, false, |b: &mut MobileBuffer| {
            b.bind(target(1, 1), false).is_ok() && b.push_phrase("你好").is_ok()
        });
        assert!(ok);
        let blocks = with_locked(h, Vec::new(), |b: &mut MobileBuffer| b.blocks());
        assert_eq!(blocks, vec!["你好".to_string()]);
        // SAFETY: h 来自上面的 into_handle，只释放一次。
        unsafe { free_handle::<Mutex<MobileBuffer>>(h) };
    }
}
