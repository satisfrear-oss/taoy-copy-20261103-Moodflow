package com.moodflow.ime.core

import com.moodflow.ime.jni.MoodflowNative as N
import com.moodflow.ime.jni.NativeEventSink
import com.moodflow.ime.logic.CandidateWindow
import com.moodflow.ime.logic.ChordMapping
import com.moodflow.ime.logic.ChordResult
import com.moodflow.ime.logic.ClipEntry
import com.moodflow.ime.logic.PluginInfo
import com.moodflow.ime.logic.PracticeScore
import com.moodflow.ime.logic.KeyOutcome
import com.moodflow.ime.logic.NamedEntry
import com.moodflow.ime.logic.SchemaEntry
import com.moodflow.ime.logic.StrokeTable
import com.moodflow.ime.logic.Target
import com.moodflow.ime.logic.Wire
import java.io.File

/**
 * 输入法核心会话（`moodflow_mobile::MobileCore` 的 Kotlin 包装）。
 *
 * 构造耗时（加载词库、编译方案），必须在后台线程调 [create]；其余方法在主线程调用。
 * [close] 之后不得再用。
 */
class MoodflowCore private constructor(private var handle: Long) : AutoCloseable {

    fun keyDown(vk: Int, modifiers: Int): KeyOutcome = Wire.outcome(N.nativeKeyDown(handle, vk, modifiers))

    fun focusGained(isPrivate: Boolean, hostApp: String) = N.nativeFocusGained(handle, isPrivate, hostApp)

    fun focusLost() = N.nativeFocusLost(handle)

    fun runCommand(cmd: Command, intArg: Int = 0, strArg: String = "") =
        N.nativeRunCommand(handle, cmd.code, intArg, strArg)

    fun userStoreAvailable(): Boolean = N.nativeUserStoreAvailable(handle)

    fun schemas(): List<SchemaEntry> = Wire.schemas(N.nativeSchemas(handle))

    fun candidates(offset: Int, limit: Int): CandidateWindow =
        Wire.candidateWindow(N.nativeCandidates(handle, offset, limit))

    fun selectCandidate(index: Int): KeyOutcome = Wire.outcome(N.nativeSelectCandidate(handle, index))

    fun themes(): List<NamedEntry> = Wire.namedEntries(N.nativeThemes(handle))

    fun shuangpinLayouts(): List<NamedEntry> = Wire.namedEntries(N.nativeShuangpinLayouts(handle))

    fun charStrokes(): StrokeTable = StrokeTable.fromFlat(N.nativeCharStrokes(handle))

    fun configSnapshot(): String = N.nativeConfigSnapshot(handle).orEmpty()

    /** @return null 成功，否则为给用户看的失败原因 */
    fun configSet(key: String, type: ConfigType, value: String): String? =
        N.nativeConfigSet(handle, key, type.code, value)

    fun feedText(text: String): KeyOutcome = Wire.outcome(N.nativeFeedText(handle, text))

    fun replaceComposition(text: String): KeyOutcome = Wire.outcome(N.nativeReplaceComposition(handle, text))

    fun clearComposition(): KeyOutcome = Wire.outcome(N.nativeClearComposition(handle))

    // ── 剪贴板历史：核心那份（`clipboard_history.json`，与 Windows 同一存储与闸门）──

    /** 键盘可见期间读到的一次剪贴板变化；收不收由核心定（开关、隐私、敏感、空白、过长）。 */
    fun clipboardChanged(text: String, sensitive: Boolean) = N.nativeClipboardChanged(handle, text, sensitive)

    /** 收藏的在前，各自新到旧。 */
    fun clipboardEntries(): List<ClipEntry> = Wire.clipEntries(N.nativeClipboardEntries(handle))

    fun clipboardSetPinned(id: Long, pinned: Boolean): Boolean = N.nativeClipboardSetPinned(handle, id, pinned)

    fun clipboardRemove(id: Long): Boolean = N.nativeClipboardRemove(handle, id)

    fun clipboardClearUnpinned() = N.nativeClipboardClearUnpinned(handle)

    // ── 插件 ──

    fun plugins(): List<PluginInfo> = Wire.plugins(N.nativePlugins(handle))

    fun pluginReenable(id: String): Boolean = N.nativePluginReenable(handle, id)

    @Synchronized
    override fun close() {
        if (handle != 0L) {
            N.nativeDestroy(handle)
            handle = 0L
        }
    }

    /** 与 Rust 侧 `nativeRunCommand` 的 `cmd` 编码一致。 */
    enum class Command(val code: Int) {
        ENGLISH(0), SELECT_SCHEMA(1), TOGGLE_PUNCT(2), TOGGLE_WIDTH(3), TOGGLE_S2T(4),
        RELOAD_CONFIG(5), SET_SHUANGPIN_LAYOUT(6),
    }

    /** 与 Rust 侧 `parse_config_value` 的类型编码一致。 */
    enum class ConfigType(val code: Int) { BOOL(0), INT(1), FLOAT(2), STR(3), STR_LIST(4) }

    companion object {
        /** 后台线程调用。.so 加载失败或构造失败返回 null（键盘退化为直接上屏）。 */
        fun create(dataDir: File, userRoot: File, fallbackSchema: String, sink: NativeEventSink): MoodflowCore? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeCreate(dataDir.absolutePath, userRoot.absolutePath, fallbackSchema, sink)
            return if (h == 0L) null else MoodflowCore(h)
        }
    }
}

/** 九宫格切分（无状态）。 */
object T9Native {
    fun segmentations(input: String, limit: Int): List<String> =
        if (input.isEmpty() || !N.ensureLoaded()) emptyList()
        else N.nativeT9Segmentations(input, limit)?.toList().orEmpty()

    fun leading(input: String): List<String> =
        if (input.isEmpty() || !N.ensureLoaded()) emptyList()
        else N.nativeT9Leading(input)?.toList().orEmpty()

    fun pin(input: String, syllable: String): String? =
        if (!N.ensureLoaded()) null else N.nativeT9Pin(input, syllable)
}

/** Buffer（`moodflow_mobile::buffer::MobileBuffer`）。凭据留在 Rust 侧，这里只报目标与结果。 */
class NativeBuffer private constructor(private var h: Long) : AutoCloseable {
    enum class State { UNBOUND, READY, SUSPENDED, HELD }
    enum class SendKey { CONFIRM_COMPOSITION, SEND_NEXT, PASS_THROUGH }
    enum class WriteResult(val code: Int) { ACCEPTED(0), REJECTED(1), UNKNOWN(2) }

    fun bind(t: Target, isPrivate: Boolean): Boolean = N.nativeBufferBind(h, t.id, t.revision, isPrivate)
    fun focusChanged(t: Target) = N.nativeBufferFocusChanged(h, t.id, t.revision)
    fun suspend() = N.nativeBufferSuspend(h)
    fun discard() = N.nativeBufferDiscard(h)
    fun releaseHold() = N.nativeBufferReleaseHold(h)

    /** 返回 0 成功 / 1 空 / 2 超容量 / 3 块在投递中。 */
    fun pushPhrase(text: String): Int = N.nativeBufferPushPhrase(h, text)
    fun pushLatin(codePoint: Int): Int = N.nativeBufferPushLatin(h, codePoint)
    fun backspace(): Int = N.nativeBufferBackspace(h)
    fun deleteLast(): Int = N.nativeBufferDeleteLast(h)
    fun clear() = N.nativeBufferClear(h)
    fun blocks(): List<String> = N.nativeBufferBlocks(h)?.toList().orEmpty()

    fun state(): State = when (N.nativeBufferState(h)) {
        1 -> State.READY
        2 -> State.SUSPENDED
        3 -> State.HELD
        else -> State.UNBOUND
    }

    fun sendKeyAction(composing: Boolean): SendKey = when (N.nativeBufferSendKeyAction(h, composing)) {
        0 -> SendKey.CONFIRM_COMPOSITION
        1 -> SendKey.SEND_NEXT
        else -> SendKey.PASS_THROUGH
    }

    /** 取出待投递文本；不能投递时返回 null。 */
    fun begin(t: Target, all: Boolean): String? = N.nativeBufferBegin(h, t.id, t.revision, all)
    fun isAuthorized(t: Target): Boolean = N.nativeBufferAuthorized(h, t.id, t.revision)
    fun finish(r: WriteResult): Boolean = N.nativeBufferFinish(h, r.code)

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativeBufferFree(h)
            h = 0L
        }
    }

    companion object {
        fun create(): NativeBuffer? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeBufferNew(0)
            return if (h == 0L) null else NativeBuffer(h)
        }
    }
}

/** 打字测速一次练习（`moodflow_mobile::practice::Practice`，与 Hub 同一套计分）。 */
class NativePractice private constructor(private var h: Long) : AutoCloseable {
    /** 输入框内容变了；[tMs] 从练习开始算起。 */
    fun onText(tMs: Long, text: String) = N.nativePracticeText(h, tMs, text)

    fun finish(finishedMs: Long): PracticeScore? = Wire.practiceScore(N.nativePracticeFinish(h, finishedMs))

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativePracticeFree(h)
            h = 0L
        }
    }

    companion object {
        fun start(target: String): NativePractice? {
            if (!N.ensureLoaded()) return null
            val h = N.nativePracticeNew(target)
            return if (h == 0L) null else NativePractice(h)
        }
    }
}

/** 并击结算器（`moodflow_mobile::chord::MobileChord`）：物理键盘逐键、触屏整组。 */
class NativeChord private constructor(private var h: Long) : AutoCloseable {
    fun mappings(): List<ChordMapping> = Wire.chordMappings(N.nativeChordMappings(h))

    /** 左右手键名（核心键名，`comma` / `period`）。 */
    fun hands(): Pair<List<String>, List<String>> {
        val a = N.nativeChordHands(h).orEmpty()
        fun keys(i: Int) = a.getOrNull(i).orEmpty().split(' ').filter(String::isNotEmpty)
        return keys(0) to keys(1)
    }

    /** 触屏手势一整组键一次结算；单键返回 null。 */
    fun settleBatch(keys: List<String>, nowMs: Long): ChordResult? =
        Wire.chord(N.nativeChordSettleBatch(h, keys.joinToString(" "), nowMs))

    fun isChordKey(key: String): Boolean = N.nativeChordIsChordKey(h, key)
    fun keyDown(key: String, nowMs: Long): ChordResult? = Wire.chord(N.nativeChordKeyDown(h, key, nowMs))
    fun keyUp(key: String, nowMs: Long): ChordResult? = Wire.chord(N.nativeChordKeyUp(h, key, nowMs))
    fun tick(nowMs: Long): ChordResult? = Wire.chord(N.nativeChordTick(h, nowMs))
    fun reset() = N.nativeChordReset(h)

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativeChordFree(h)
            h = 0L
        }
    }

    companion object {
        /** 内置默认方案（XSIME 触屏并击的映射）。原生库不在时返回 null。 */
        fun builtin(): NativeChord? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeChordBuiltin()
            return if (h == 0L) null else NativeChord(h)
        }

        /** 校验并构造；不可应用时返回 null（原因用 [validate] 取）。 */
        fun fromJson(json: String): NativeChord? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeChordNew(json)
            return if (h == 0L) null else NativeChord(h)
        }

        /** @return null = 可以应用，否则为原因 */
        fun validate(json: String): String? =
            if (!N.ensureLoaded()) "原生库未加载" else N.nativeChordValidate(json)
    }
}
