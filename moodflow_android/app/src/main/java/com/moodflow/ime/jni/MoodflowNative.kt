package com.moodflow.ime.jni

/**
 * `libmoodflow_android.so` 的全部 JNI 入口，实现在 `moodflow_android/rust/moodflow-android/src/lib.rs`。
 *
 * 名字、参数个数与类型必须与 Rust 侧逐一对应：写错不会编译失败，只会在**调用时**抛
 * `UnsatisfiedLinkError`。`MoodflowNativeContractTest` 解析 Rust 源码与本类的反射信息做对账，
 * 改任何一边都要让它通过。
 *
 * 结构化结果以 `String[]` 返回，由 `com.moodflow.ime.logic.Wire` 解码。句柄 0 恒无效。
 *
 * 本类刻意不引用任何 `android.*`：JVM 单测要能加载它做反射对账。
 */
object MoodflowNative {
    private var loaded: Boolean? = null

    /** 加载失败的原因（供调用方写日志）；成功或尚未加载为 null。 */
    @Volatile
    var loadError: Throwable? = null
        private set

    /** 首次调用时加载 .so；之后返回缓存的结果。线程安全。 */
    @Synchronized
    fun ensureLoaded(): Boolean {
        loaded?.let { return it }
        val ok = try {
            System.loadLibrary("moodflow_android")
            nativeInit()
            true
        } catch (e: UnsatisfiedLinkError) {
            loadError = e
            false
        }
        loaded = ok
        return ok
    }

    // ── 核心会话 ──
    @JvmStatic external fun nativeInit()
    @JvmStatic external fun nativeCreate(dataDir: String, userRoot: String, fallbackSchema: String, sink: NativeEventSink): Long
    @JvmStatic external fun nativeDestroy(handle: Long)
    @JvmStatic external fun nativeKeyDown(handle: Long, vk: Int, modifiers: Int): Array<String>?
    @JvmStatic external fun nativeFocusGained(handle: Long, isPrivate: Boolean, hostApp: String)
    @JvmStatic external fun nativeFocusLost(handle: Long)
    @JvmStatic external fun nativeRunCommand(handle: Long, cmd: Int, intArg: Int, strArg: String)
    @JvmStatic external fun nativeUserStoreAvailable(handle: Long): Boolean
    @JvmStatic external fun nativeSchemas(handle: Long): Array<String>?
    @JvmStatic external fun nativeCandidates(handle: Long, offset: Int, limit: Int): Array<String>?
    @JvmStatic external fun nativeSelectCandidate(handle: Long, index: Int): Array<String>?
    @JvmStatic external fun nativeThemes(handle: Long): Array<String>?
    @JvmStatic external fun nativeSetTheme(handle: Long, id: String): Boolean
    @JvmStatic external fun nativeThemeStyle(handle: Long): String?
    @JvmStatic external fun nativeSetThemeStyle(handle: Long, style: String)
    @JvmStatic external fun nativePalette(handle: Long, systemDark: Boolean): Array<String>?
    @JvmStatic external fun nativeIsDark(handle: Long, systemDark: Boolean): Boolean
    @JvmStatic external fun nativeCharStrokes(handle: Long): IntArray?
    @JvmStatic external fun nativeSymbolKeys(handle: Long): Array<String>?
    @JvmStatic external fun nativeConfigSnapshot(handle: Long): String?
    @JvmStatic external fun nativeConfigDefaults(handle: Long): String?
    @JvmStatic external fun nativeConfigRegistry(handle: Long): Array<String>?
    @JvmStatic external fun nativeConfigSet(handle: Long, key: String, type: Int, value: String): String?
    @JvmStatic external fun nativeShuangpinLayouts(handle: Long): Array<String>?
    @JvmStatic external fun nativeFeedText(handle: Long, text: String): Array<String>?
    @JvmStatic external fun nativeReplaceComposition(handle: Long, text: String): Array<String>?
    @JvmStatic external fun nativeClearComposition(handle: Long): Array<String>?

    // ── 九宫格（无状态）──
    @JvmStatic external fun nativeT9Segmentations(input: String, limit: Int): Array<String>?
    @JvmStatic external fun nativeT9Leading(input: String): Array<String>?
    @JvmStatic external fun nativeT9Pin(input: String, syllable: String): String?

    // ── Buffer ──
    @JvmStatic external fun nativeBufferNew(capacity: Int): Long
    @JvmStatic external fun nativeBufferFree(h: Long)
    @JvmStatic external fun nativeBufferBind(h: Long, id: Long, revision: Long, isPrivate: Boolean): Boolean
    @JvmStatic external fun nativeBufferFocusChanged(h: Long, id: Long, revision: Long)
    @JvmStatic external fun nativeBufferSuspend(h: Long)
    @JvmStatic external fun nativeBufferDiscard(h: Long)
    @JvmStatic external fun nativeBufferReleaseHold(h: Long)
    @JvmStatic external fun nativeBufferPushPhrase(h: Long, text: String): Int
    @JvmStatic external fun nativeBufferPushLatin(h: Long, codePoint: Int): Int
    @JvmStatic external fun nativeBufferBackspace(h: Long): Int
    @JvmStatic external fun nativeBufferDeleteLast(h: Long): Int
    @JvmStatic external fun nativeBufferClear(h: Long)
    @JvmStatic external fun nativeBufferBlocks(h: Long): Array<String>?
    @JvmStatic external fun nativeBufferState(h: Long): Int
    @JvmStatic external fun nativeBufferSendKeyAction(h: Long, composing: Boolean): Int
    @JvmStatic external fun nativeBufferBegin(h: Long, id: Long, revision: Long, all: Boolean): String?
    @JvmStatic external fun nativeBufferAuthorized(h: Long, id: Long, revision: Long): Boolean
    @JvmStatic external fun nativeBufferFinish(h: Long, result: Int): Boolean

    // ── 剪贴板历史（核心那份，与 Windows 同一存储）──
    @JvmStatic external fun nativeClipboardChanged(handle: Long, text: String, sensitive: Boolean)
    @JvmStatic external fun nativeClipboardEntries(handle: Long): Array<String>?
    @JvmStatic external fun nativeClipboardSetPinned(handle: Long, id: Long, pinned: Boolean): Boolean
    @JvmStatic external fun nativeClipboardRemove(handle: Long, id: Long): Boolean
    @JvmStatic external fun nativeClipboardClearUnpinned(handle: Long)

    // ── 插件 ──
    @JvmStatic external fun nativePlugins(handle: Long): Array<String>?
    @JvmStatic external fun nativePluginReenable(handle: Long, id: String): Boolean

    // ── 打字测速 ──
    @JvmStatic external fun nativePracticeNew(target: String): Long
    @JvmStatic external fun nativePracticeFree(h: Long)
    @JvmStatic external fun nativePracticeText(h: Long, tMs: Long, text: String)
    @JvmStatic external fun nativePracticeFinish(h: Long, finishedMs: Long): String?

    // ── 并击 ──
    @JvmStatic external fun nativeChordNew(json: String): Long
    @JvmStatic external fun nativeChordBuiltin(): Long
    @JvmStatic external fun nativeChordMappings(h: Long): Array<String>?
    @JvmStatic external fun nativeChordHands(h: Long): Array<String>?
    @JvmStatic external fun nativeChordSettleBatch(h: Long, keys: String, nowMs: Long): String?
    @JvmStatic external fun nativeChordValidate(json: String): String?
    @JvmStatic external fun nativeChordFree(h: Long)
    @JvmStatic external fun nativeChordIsChordKey(h: Long, key: String): Boolean
    @JvmStatic external fun nativeChordKeyDown(h: Long, key: String, nowMs: Long): String?
    @JvmStatic external fun nativeChordKeyUp(h: Long, key: String, nowMs: Long): String?
    @JvmStatic external fun nativeChordTick(h: Long, nowMs: Long): String?
    @JvmStatic external fun nativeChordReset(h: Long)
}
