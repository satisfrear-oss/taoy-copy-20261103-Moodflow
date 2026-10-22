package com.moodflow.ime

import android.content.ClipboardManager
import android.content.Intent
import android.content.SharedPreferences
import android.content.res.Configuration
import android.inputmethodservice.InputMethodService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.WindowInsets
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import com.moodflow.ime.core.ChordProfiles
import com.moodflow.ime.core.CoreHolder
import com.moodflow.ime.core.MoodflowCore
import com.moodflow.ime.core.NativeBuffer
import com.moodflow.ime.core.NativeChord
import com.moodflow.ime.core.T9Native
import com.moodflow.ime.jni.MoodflowNative
import com.moodflow.ime.jni.NativeEventSink
import com.moodflow.ime.keyboard.ChordGesture
import com.moodflow.ime.keyboard.ChordLayout
import com.moodflow.ime.keyboard.ChordProfile
import com.moodflow.ime.keyboard.KeyboardLayout
import com.moodflow.ime.keyboard.KeyboardTheme
import com.moodflow.ime.keyboard.PunctuationLayout
import com.moodflow.ime.logic.BufferSink
import com.moodflow.ime.logic.CandidateWindow
import com.moodflow.ime.logic.ChordKeys
import com.moodflow.ime.logic.ChordResult
import com.moodflow.ime.logic.ClipEntry
import com.moodflow.ime.logic.ClipboardGate
import com.moodflow.ime.logic.ConfigSnapshot
import com.moodflow.ime.logic.EditOp
import com.moodflow.ime.logic.FieldChange
import com.moodflow.ime.logic.FieldKey
import com.moodflow.ime.logic.FieldPolicy
import com.moodflow.ime.logic.FieldTracker
import com.moodflow.ime.logic.KeyOutcome
import com.moodflow.ime.logic.KeyboardChoice
import com.moodflow.ime.logic.Mods
import com.moodflow.ime.logic.OpRouter
import com.moodflow.ime.logic.PhysicalKeys
import com.moodflow.ime.logic.SchemaEntry
import com.moodflow.ime.logic.StrokeTable
import com.moodflow.ime.logic.T9Input
import com.moodflow.ime.logic.TextSink
import com.moodflow.ime.logic.Vk
import com.moodflow.ime.ui.BufferRail
import com.moodflow.ime.ui.CandidateMatrixView
import com.moodflow.ime.ui.ChordPreview
import com.moodflow.ime.ui.ChordSurface
import com.moodflow.ime.ui.DeleteRepeatTouch
import com.moodflow.ime.ui.KeyButton
import com.moodflow.ime.ui.KeyboardAppearancePanel
import com.moodflow.ime.ui.KeyboardIcon
import com.moodflow.ime.ui.KeyboardRoot
import com.moodflow.ime.ui.KeyboardSettings
import com.moodflow.ime.ui.KeyboardSurface
import com.moodflow.ime.ui.TextClipboardPanel
import com.moodflow.ime.ui.Themes
import java.util.Locale

/**
 * Moodflow 输入法服务。
 *
 * 界面移植自 XSIME Android（`RimesInputMethodService`，Apache-2.0）：候选行（每页 9 个、翻页、左侧设置、
 * 右侧 Buffer 开关、展开成矩阵）、XSIME 键位几何的 26 键 / 九宫格 / 数字 / 符号 / 表情页、长按删除连删、
 * 上滑出数字、Buffer 轨、外观面板（18 套配色）、剪贴板面板、双拇指滑动并击。
 *
 * **引擎是 Moodflow Rust 核心**，不是 librime：按键、候选、选词、组字都经 `moodflow-mobile` 门面
 * （[MoodflowCore]），Buffer / 并击 / 九宫格 / 剪贴板历史也都是门面里的共享逻辑。XSIME 那套
 * 「引擎工作线程 + 结果票据」没有搬：核心调用是同步的，全部在主线程。
 *
 * 隐私契约（AGENTS.md「私密字段」）：密码类与 `IME_FLAG_NO_PERSONALIZED_LEARNING` 字段完全绕过引擎、
 * 不启用 Buffer、不采集剪贴板；旧组字绝不带进新输入框。
 *
 * 日志：只记生命周期事件，**不记任何用户输入、候选或剪贴板内容**。
 */
class MoodflowImeService : InputMethodService(), NativeEventSink {
    private val main = Handler(Looper.getMainLooper())
    private var destroyed = false

    // ── 核心与共享逻辑 ──
    private var core: MoodflowCore? = null
    private var coreFailed = false
    private var strokes = StrokeTable.EMPTY
    private var storeAvailable = true
    private var chinese = true
    private var schemas: List<SchemaEntry> = emptyList()
    private var buffer: NativeBuffer? = null
    private var bufferMode = false
    private var chord: NativeChord? = null
    private var chordProfile: ChordProfile? = null
    private var lastChordSyllable = false
    private val t9 = T9Input()

    private val tracker = FieldTracker()
    private var policy = FieldPolicy.NONE

    // ── 宿主偏好 ──
    private lateinit var settings: KeyboardSettings
    private var layoutPref = KeyboardChoice.QWERTY
    private var theme: KeyboardTheme = KeyboardTheme.ALL[0]
    private var clipboardEnabled = false
    private val prefListener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> restoreSettings(); render() }

    // ── 界面状态 ──
    private var uppercase = false
    private var numeric = false
    private var symbols = false
    private var emoji = false
    private var numberPageFresh = false
    private var appearanceOpen = false
    private var clipboardOpen = false
    private var spellingOpen = false
    private var punctuationOpen = false
    private var spellingPage = 0
    private var spellingChoices: List<String> = emptyList()
    private var keyboardVisible = false
    private var clipListening = false

    // 候选：推送帧给编码串，列表按页从核心拉（每页 9 个，翻页在宿主）
    private var preedit = ""
    private var composing = false
    private var pageStart = 0
    private var page = CandidateWindow.EMPTY
    private var matrixExpanded = false
    private var matrixItems: List<CandidateMatrixView.Item> = emptyList()
    private var matrixTotal = 0
    private var chordPreviewState: ChordGesture.Preview? = null

    // ── 视图 ──
    private var keyboard: KeyboardRoot? = null
    private lateinit var status: TextView
    private lateinit var bufferRow: LinearLayout
    private lateinit var bufferRail: BufferRail
    private lateinit var bufferMetrics: TextView
    private lateinit var insertNext: KeyButton
    private lateinit var candidateRow: LinearLayout
    private lateinit var settingsButton: KeyButton
    private lateinit var previous: KeyButton
    private lateinit var next: KeyButton
    private lateinit var expand: KeyButton
    private lateinit var bufferButton: KeyButton
    private lateinit var candidateScroll: HorizontalScrollView
    private lateinit var candidateMatrix: CandidateMatrixView
    private lateinit var chordReadout: ChordPreview
    private lateinit var idleRow: LinearLayout
    private val candidateSlots = ArrayList<KeyButton>()
    private lateinit var spellingRow: LinearLayout
    private val spellingButtons = ArrayList<KeyButton>()
    private lateinit var surfaceContainer: FrameLayout
    private lateinit var keys: KeyboardSurface
    private var chords: ChordSurface? = null
    private lateinit var chordFooter: LinearLayout
    private val chordControls = ArrayList<KeyButton>()
    private lateinit var appearancePanel: KeyboardAppearancePanel
    private lateinit var clipboardPanel: TextClipboardPanel
    private val chromeButtons = ArrayList<KeyButton>()
    private var renderedCandidateKey = ""

    /** 物理键盘 keydown 被核心吃掉的键，对应的 keyup 也吞掉，不让应用收到半个按键。 */
    private val swallowedKeyUps = HashSet<Int>()

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener { onClipChanged(explicit = false) }
    private val chordTick = Runnable { chord?.tick(SystemClock.uptimeMillis())?.let { handleChord(it) } }

    private val bufferSink = object : BufferSink {
        override fun pushPhrase(text: String): Boolean = buffer?.pushPhrase(text) == 0
    }

    // ── 生命周期 ────────────────────────────────────────────────

    override fun onCreate() {
        super.onCreate()
        settings = KeyboardSettings(this)
        settings.preferences().registerOnSharedPreferenceChangeListener(prefListener)
        restoreSettings()
        buffer = NativeBuffer.create()
        chord = NativeChord.builtin()
        chordProfile = chord?.let { ChordProfiles.of(it) }
        MoodflowNative.loadError?.let { Log.e(TAG, "原生库加载失败，键盘退化为直接上屏", it) }
        CoreHolder.listener = this
        CoreHolder.acquire(this) { onCoreReady(it) }
    }

    private fun onCoreReady(c: MoodflowCore?) {
        if (destroyed) return
        if (c == null) {
            coreFailed = true
            render()
            return
        }
        core = c
        strokes = c.charStrokes()
        storeAvailable = c.userStoreAvailable()
        refreshSchemas()
        readCoreConfig()
        if (tracker.current != null) c.focusGained(policy.isPrivate, currentInputEditorInfo?.packageName.orEmpty())
        if (KeyboardChoice.isChord(layoutPref) || layoutPref == KeyboardChoice.NINE_KEY) ensurePinyinSchema()
        render()
    }

    override fun onDestroy() {
        destroyed = true
        stopClipListening()
        main.removeCallbacksAndMessages(null)
        settings.preferences().unregisterOnSharedPreferenceChangeListener(prefListener)
        if (CoreHolder.listener === this) CoreHolder.listener = null
        core = null
        CoreHolder.release()
        buffer?.close()
        buffer = null
        chord?.close()
        chord = null
        super.onDestroy()
    }

    private fun restoreSettings() {
        layoutPref = settings.layout
        theme = KeyboardTheme.named(settings.theme)
    }

    private fun refreshSchemas() {
        schemas = core?.schemas().orEmpty()
    }

    private fun activeSchema(): String = schemas.firstOrNull { it.active }?.id ?: CoreHolder.FALLBACK_SCHEMA

    private fun readCoreConfig() {
        val snap = core?.configSnapshot() ?: return
        clipboardEnabled = ConfigSnapshot(snap).bool("clipboard.enabled") ?: false
    }

    /** 横屏也不进全屏抽取界面：候选与 Buffer 都画在键盘里。 */
    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
        super.onStartInput(attribute, restarting)
        policy = FieldPolicy.of(attribute.inputType, attribute.imeOptions)
        val change = tracker.onStartInput(
            FieldKey(attribute.packageName.orEmpty(), attribute.fieldId, attribute.inputType),
            restarting,
        )
        // 旧组字绝不带进新上下文（XSIME 吸收计划 §3.7）：核心与本地状态一起清
        core?.clearComposition()
        resetLocalComposition()
        retireTouches()
        chord?.reset()
        swallowedKeyUps.clear()

        // Buffer：换字段丢弃，同字段重启只暂停（buffer.md §3 Android 一列）；私密字段一律丢弃
        val target = tracker.current
        buffer?.let { b ->
            if (!policy.allowBuffer || change == FieldChange.NEW_FIELD || target == null) {
                b.discard()
            } else {
                b.focusChanged(target)
            }
        }
        if (!policy.allowBuffer || change == FieldChange.NEW_FIELD) bufferMode = false
        if (change == FieldChange.NEW_FIELD) closePanels()

        core?.focusGained(policy.isPrivate, attribute.packageName.orEmpty())
        restoreSettings()
        uppercase = false
        numeric = policy.prefersNumberPad
        symbols = false
        emoji = false
        numberPageFresh = false
        punctuationOpen = false
        spellingOpen = false
        render()
    }

    override fun onStartInputView(info: EditorInfo, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        keyboardVisible = true
        readCoreConfig()
        startClipListening()
        render()
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        keyboardHidden()
        super.onFinishInputView(finishingInput)
    }

    override fun onWindowHidden() {
        keyboardHidden()
        super.onWindowHidden()
    }

    /** 隐藏键盘：停止剪贴板采集、收起面板，Buffer 清空（Android 上隐藏即撤销投递授权并丢弃内容）。 */
    private fun keyboardHidden() {
        keyboardVisible = false
        stopClipListening()
        closePanels()
        retireTouches()
        buffer?.discard()
        bufferMode = false
        if (composing || !t9.isEmpty) {
            core?.clearComposition()
            clearHostComposition()
            resetLocalComposition()
        }
    }

    override fun onFinishInput() {
        // 旧输入框里的组字删掉而不是提交：拼音串不是用户要的内容，也不能留给下一个输入框
        clearHostComposition()
        tracker.onFinishInput()
        core?.focusLost()
        buffer?.discard()
        bufferMode = false
        chord?.reset()
        retireTouches()
        resetLocalComposition()
        closePanels()
        policy = FieldPolicy.NONE
        super.onFinishInput()
    }

    /** 换输入框 / 换模式时作废进行中的触摸：迟到的抬起与排队的点击不落到新目标上（XSIME `KeyboardRoot`）。 */
    private fun retireTouches() {
        chords?.cancel()
        chordPreviewState = null
        keyboard?.cancelPendingInputEvents()
    }

    private fun closePanels() {
        appearanceOpen = false
        clipboardOpen = false
        if (::clipboardPanel.isInitialized) clipboardPanel.redact()
    }

    // ── 核心推送（来自泵线程，切回主线程）──────────────────────

    override fun onCandidates(
        preedit: String,
        caret: Int,
        modeLabel: String,
        items: Array<String>,
        selected: Int,
        page: Int,
        totalPages: Int,
    ) {
        main.post { if (!destroyed) onCandidateFrame(preedit) }
    }

    override fun onHideCandidates() {
        main.post { if (!destroyed) onCandidatesHidden() }
    }

    override fun onStatus(
        chineseMode: Boolean,
        iconLabel: String,
        fullWidth: Boolean,
        chinesePunct: Boolean,
        s2tEnabled: Boolean,
        s2tShown: Boolean,
    ) {
        main.post {
            if (destroyed) return@post
            chinese = chineseMode
            refreshSchemas()
            render()
        }
    }

    override fun onToast(text: String) {
        main.post { if (!destroyed) Toast.makeText(this, text, Toast.LENGTH_SHORT).show() }
    }

    private fun onCandidateFrame(preeditText: String) {
        if (preeditText != preedit) pageStart = 0
        preedit = preeditText
        composing = true
        matrixExpanded = false
        pullPage()
        render()
    }

    private fun onCandidatesHidden() {
        // 九宫格状态不在这里清：replaceComposition 内部的 Esc 也会推一次隐藏帧
        preedit = ""
        composing = false
        pageStart = 0
        page = CandidateWindow.EMPTY
        matrixExpanded = false
        render()
    }

    private fun pullPage() {
        page = core?.candidates(pageStart, PAGE) ?: CandidateWindow.EMPTY
        if (page.items.isEmpty() && pageStart > 0) {
            pageStart = 0
            page = core?.candidates(0, PAGE) ?: CandidateWindow.EMPTY
        }
    }

    // ── 输入路由 ────────────────────────────────────────────────

    /** 本字段该不该走引擎：私密 / 非文本字段、核心未就绪时为 null，按键直接上屏。 */
    private fun engine(): MoodflowCore? = if (policy.useEngine) core else null

    private val directOnly: Boolean get() = !policy.useEngine

    /** 字母区此刻的形态（九宫格 / 并击只配中文拼音，见 [KeyboardChoice.visibleLetters]）。 */
    private fun letters(): String {
        if (directOnly) return KeyboardChoice.QWERTY
        val visible = KeyboardChoice.visibleLetters(layoutPref, activeSchema(), chinese)
        return if (KeyboardChoice.isChord(visible) && (chordProfile == null || chords == null)) KeyboardChoice.QWERTY else visible
    }

    private fun nineKeyVisible() = !numeric && !emoji && letters() == KeyboardChoice.NINE_KEY
    private fun chordVisible() = !numeric && !emoji && KeyboardChoice.isChord(letters())

    /** 数字 / 符号页在中文态显示中文标点（XSIME `PunctuationLayout`）。 */
    private fun chinesePages() = chinese && !directOnly

    private fun visibleMode(): KeyboardLayout.Mode = when {
        emoji -> KeyboardLayout.Mode.EMOJI
        numeric -> if (symbols) KeyboardLayout.Mode.SYMBOLS else KeyboardLayout.Mode.NUMERIC
        nineKeyVisible() -> KeyboardLayout.Mode.NINE_KEY
        else -> KeyboardLayout.Mode.QWERTY
    }

    /** 键面上的一段文字：逐字按核心给的按法送进引擎；按法表里没有的字直接上屏。 */
    private fun type(text: String) {
        if (text.isEmpty()) return
        val c = engine()
        if (c == null || emoji || text.codePointCount(0, text.length) != 1 || text.length != 1) {
            literal(text)
            return
        }
        val ch = text[0]
        val stroke = strokes[ch]
        if (stroke == null) {
            literal(text)
            return
        }
        val o = c.keyDown(stroke.vk, if (stroke.shift) Mods.SHIFT else Mods.NONE)
        if (o.consumed) applyOutcome(o) else commitRaw(text)
    }

    /**
     * 不经引擎的一段文字（表情、上滑数字、键面上引擎不认的符号）。有组字时先把当前页首选上屏，
     * 再上屏这段文字——与 XSIME `punctuate` 同一规则：符号不能吞掉用户已经打的拼音。
     */
    private fun literal(text: String) {
        settleComposition()
        commitRaw(text)
    }

    /** 有组字时选当前页首选上屏（没有候选时按回车交原码）。 */
    private fun settleComposition() {
        val c = engine() ?: return
        if (!composing) return
        if (page.items.isNotEmpty()) {
            applyOutcome(c.selectCandidate(pageStart))
        } else {
            applyOutcome(c.keyDown(Vk.RETURN, Mods.NONE))
        }
        if (!t9.isEmpty) {
            t9.clear()
            spellingChoices = emptyList()
        }
    }

    /** 中文标点：引擎认得的（`，。` 等有源键）走引擎（智能标点、中英标点切换都在核心）；其余先定组字再直接上屏。 */
    private fun punctuate(mark: String) {
        val stroke = if (mark.length == 1) strokes[mark[0]] else null
        if (engine() != null && stroke != null && t9.isEmpty) type(mark) else literal(mark)
    }

    private fun delete() {
        val c = engine()
        if (c != null && nineKeyVisible() && !t9.isEmpty) {
            t9.backspace()
            refreshT9()
            return
        }
        if (c != null && composing) {
            val o = c.keyDown(Vk.BACK, Mods.NONE)
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        val b = buffer
        if (bufferMode && b != null) {
            // Buffer 里删除整块（XSIME：Delete removes a whole last block）
            if (b.blocks().isNotEmpty()) b.deleteLast()
            render()
            return
        }
        sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL)
    }

    private fun enter() {
        val b = buffer
        if (bufferMode && b != null) {
            // 有组字时只确认组字，同一次按键不顺带发送（buffer.md 契约 #7）
            when (b.sendKeyAction(composing)) {
                NativeBuffer.SendKey.CONFIRM_COMPOSITION -> {
                    engine()?.keyDown(Vk.RETURN, Mods.NONE)?.let { applyOutcome(it) }
                    t9.clear()
                    return
                }
                NativeBuffer.SendKey.SEND_NEXT -> {
                    deliverBuffer(all = false)
                    return
                }
                NativeBuffer.SendKey.PASS_THROUGH -> Unit
            }
        }
        val c = engine()
        if (c != null && composing) {
            val o = c.keyDown(Vk.RETURN, Mods.NONE)
            t9.clear()
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        if (!sendDefaultEditorAction(true)) sendDownUpKeyEvents(KeyEvent.KEYCODE_ENTER)
    }

    private fun space() {
        val c = engine()
        if (c != null && composing && pageStart > 0 && page.items.isNotEmpty()) {
            // 翻到后面的页时，空格上屏当前页首选（XSIME：Space selects the first candidate on the current page）
            applyOutcome(c.selectCandidate(pageStart))
            return
        }
        if (c != null) {
            val o = c.keyDown(Vk.SPACE, Mods.NONE)
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        commitRaw(" ")
    }

    private fun toggleLanguage() {
        val c = core ?: return
        if (directOnly) return
        t9.clear()
        if (chinese) {
            c.runCommand(MoodflowCore.Command.ENGLISH)
        } else {
            val idx = schemas.indexOfFirst { it.active }.coerceAtLeast(0)
            c.runCommand(MoodflowCore.Command.SELECT_SCHEMA, idx)
        }
        uppercase = false
        spellingOpen = false
        punctuationOpen = false
    }

    private fun chooseSchema(id: String) {
        val c = core ?: return
        val idx = schemas.indexOfFirst { it.id == id }
        if (idx < 0) return
        settleComposition()
        c.runCommand(MoodflowCore.Command.SELECT_SCHEMA, idx)
        // 非拼音方案不配九宫格 / 并击：偏好回到 26 键（XSIME KeyboardSettings.setSchema 同一规则）
        if (id != KeyboardChoice.PINYIN_SCHEMA && layoutPref != KeyboardChoice.QWERTY) settings.setLayout(KeyboardChoice.QWERTY)
        refreshSchemas()
        render()
    }

    private fun chooseLayout(id: String) {
        settleComposition()
        retireTouches()
        settings.setLayout(id)
        restoreSettings()
        numeric = false
        symbols = false
        emoji = false
        uppercase = false
        spellingOpen = false
        // 九宫格与并击都接拼音（XSIME：Nine-key selects Pinyin）
        if (id != KeyboardChoice.QWERTY) ensurePinyinSchema()
        render()
    }

    private fun ensurePinyinSchema() {
        val c = core ?: return
        val idx = schemas.indexOfFirst { it.id == KeyboardChoice.PINYIN_SCHEMA }
        if (idx >= 0 && (!schemas[idx].active || !chinese)) {
            c.runCommand(MoodflowCore.Command.SELECT_SCHEMA, idx)
            refreshSchemas()
        }
    }

    // ── 键区回调 ────────────────────────────────────────────────

    private val keyHandler = object : KeyboardSurface.Handler {
        override fun label(key: KeyboardLayout.Key): String = when (key.action) {
            KeyboardLayout.Action.TEXT -> when {
                nineKeyVisible() -> T9Input.LETTERS[key.text[0]].orEmpty()
                !numeric && !emoji -> if (uppercase) key.text.uppercase(Locale.ROOT) else key.text
                else -> key.text
            }
            KeyboardLayout.Action.SHIFT -> if (uppercase) "⇪" else "⇧"
            KeyboardLayout.Action.DELETE -> "⌫"
            KeyboardLayout.Action.RETURN -> returnLabel()
            KeyboardLayout.Action.NUMBERS -> if (numeric || emoji) {
                if (layoutPref == KeyboardChoice.NINE_KEY && chinese && !directOnly) "拼音" else "ABC"
            } else {
                "123"
            }
            KeyboardLayout.Action.SYMBOLS -> if (numeric && symbols) "123" else "#+="
            KeyboardLayout.Action.LANGUAGE -> if (chinese && !directOnly) "中" else "英"
            KeyboardLayout.Action.EMOJI -> if (emoji) "ABC" else "☺"
            KeyboardLayout.Action.SPACE -> if (chinese && !directOnly) "空格" else "space"
            KeyboardLayout.Action.SPELLING -> "选拼音"
            KeyboardLayout.Action.SEPARATOR -> "分隔"
            KeyboardLayout.Action.PUNCTUATION -> when {
                key.text.isEmpty() -> "，。"
                chinesePages() -> if (key.text == ",") "，" else "。"
                else -> key.text
            }
            else -> ""
        }

        override fun description(key: KeyboardLayout.Key): String = when (key.action) {
            KeyboardLayout.Action.TEXT -> if (nineKeyVisible()) "九键 ${key.text} ${label(key)}" else label(key)
            KeyboardLayout.Action.SHIFT -> "Shift"
            KeyboardLayout.Action.DELETE -> getString(R.string.backspace)
            KeyboardLayout.Action.RETURN -> getString(R.string.enter)
            KeyboardLayout.Action.NUMBERS -> "数字与字母"
            KeyboardLayout.Action.SYMBOLS -> "符号页"
            KeyboardLayout.Action.LANGUAGE -> "中英切换"
            KeyboardLayout.Action.EMOJI -> "表情"
            KeyboardLayout.Action.SPACE -> getString(R.string.space)
            KeyboardLayout.Action.SPELLING -> "选拼音"
            KeyboardLayout.Action.SEPARATOR -> "分隔音节"
            KeyboardLayout.Action.PUNCTUATION -> if (key.text.isEmpty()) "中文标点" else label(key)
            else -> ""
        }

        override fun enabled(key: KeyboardLayout.Key): Boolean = when (key.action) {
            KeyboardLayout.Action.LANGUAGE -> !directOnly && core != null
            KeyboardLayout.Action.SPELLING -> spellingChoices.isNotEmpty()
            KeyboardLayout.Action.SEPARATOR -> !t9.isEmpty && !t9.text.endsWith("'")
            else -> true
        }

        override fun selected(key: KeyboardLayout.Key): Boolean =
            key.action == KeyboardLayout.Action.SHIFT && uppercase ||
                key.action == KeyboardLayout.Action.SPELLING && spellingOpen ||
                key.action == KeyboardLayout.Action.PUNCTUATION && key.text.isEmpty() && punctuationOpen ||
                key.action == KeyboardLayout.Action.RETURN && returnSelected()

        override fun onAlternate(text: String) {
            // 上滑出的数字 / 标点是字面量：不能被引擎当成选词数字键
            if (text.length == 1 && text[0].isDigit()) literal(text) else punctuate(text)
            render()
        }

        override fun canRepeatDelete(): Boolean = tracker.current != null

        override fun press(key: KeyboardLayout.Key) {
            if (punctuationOpen && key.action != KeyboardLayout.Action.PUNCTUATION) punctuationOpen = false
            when (key.action) {
                KeyboardLayout.Action.TEXT -> pressText(key)
                KeyboardLayout.Action.SHIFT -> uppercase = !uppercase
                KeyboardLayout.Action.DELETE -> delete()
                KeyboardLayout.Action.RETURN -> enter()
                KeyboardLayout.Action.NUMBERS -> {
                    if (emoji) {
                        emoji = false
                        numeric = false
                    } else {
                        numeric = !numeric
                        numberPageFresh = numeric
                        symbols = false
                    }
                    spellingOpen = false
                }
                KeyboardLayout.Action.SYMBOLS -> {
                    symbols = !symbols || !numeric
                    numberPageFresh = numberPageFresh || !numeric
                    numeric = true
                    emoji = false
                }
                KeyboardLayout.Action.LANGUAGE -> toggleLanguage()
                KeyboardLayout.Action.EMOJI -> {
                    emoji = !emoji
                    numeric = false
                }
                KeyboardLayout.Action.SPACE -> space()
                KeyboardLayout.Action.SPELLING -> {
                    spellingOpen = !spellingOpen
                    punctuationOpen = false
                    spellingPage = 0
                }
                KeyboardLayout.Action.SEPARATOR -> if (t9.pressSeparator()) refreshT9()
                KeyboardLayout.Action.PUNCTUATION ->
                    if (key.text.isNotEmpty()) {
                        punctuate(label(key))
                    } else {
                        punctuationOpen = !punctuationOpen
                        spellingOpen = false
                        if (punctuationOpen) candidateScroll.scrollTo(0, 0)
                    }
                else -> Unit
            }
            render()
        }
    }

    private fun pressText(key: KeyboardLayout.Key) {
        if (nineKeyVisible()) {
            if (engine() == null) commitRaw(key.text) else if (t9.pressDigit(key.text[0])) refreshT9()
            return
        }
        val text = if (uppercase && !numeric && !emoji) key.text.uppercase(Locale.ROOT) else key.text
        // 刚从字母页打开 123 时先打的一个句读标点：打完回字母页（XSIME 一次性数字页）
        val oneShot = numeric && numberPageFresh && chinesePages() && PunctuationLayout.returnsToLetters(text)
        numberPageFresh = false
        when {
            emoji -> literal(text)
            chinesePages() && PunctuationLayout.STRIP.contains(text) -> punctuate(text)
            else -> type(text)
        }
        if (oneShot) {
            numeric = false
            symbols = false
        }
    }

    private fun returnSelected(): Boolean {
        if (composing) return false
        if (bufferMode) return buffer?.blocks()?.isNotEmpty() == true
        val info = currentInputEditorInfo ?: return false
        if (info.imeOptions and EditorInfo.IME_FLAG_NO_ENTER_ACTION != 0) return false
        return when (info.imeOptions and EditorInfo.IME_MASK_ACTION) {
            EditorInfo.IME_ACTION_SEARCH, EditorInfo.IME_ACTION_GO, EditorInfo.IME_ACTION_SEND,
            EditorInfo.IME_ACTION_NEXT, EditorInfo.IME_ACTION_DONE -> true
            else -> false
        }
    }

    private fun returnLabel(): String {
        if (composing) return "原码"
        if (bufferMode) return "插入"
        val info = currentInputEditorInfo ?: return "换行"
        if (info.imeOptions and EditorInfo.IME_FLAG_NO_ENTER_ACTION != 0) return "换行"
        return when (info.imeOptions and EditorInfo.IME_MASK_ACTION) {
            EditorInfo.IME_ACTION_SEARCH -> "搜索"
            EditorInfo.IME_ACTION_GO -> "前往"
            EditorInfo.IME_ACTION_SEND -> "发送"
            EditorInfo.IME_ACTION_NEXT -> "下一项"
            EditorInfo.IME_ACTION_DONE -> "完成"
            else -> "换行"
        }
    }

    // ── 九宫格 ──────────────────────────────────────────────────

    /**
     * 按当前九宫格串重算：取第一种切分整体替换核心里的组字（`moodflow_mobile::t9`）。
     *
     * TODO(P3): 多种切分之间的取舍目前只看段数与字典序，应改为按引擎候选质量排序；部分上屏后剩余
     * 数字串与核心组字的同步也未处理（上屏即清空九宫格串）。
     */
    private fun refreshT9() {
        val c = engine() ?: return
        if (t9.isEmpty) {
            spellingChoices = emptyList()
            spellingOpen = false
            applyOutcome(c.clearComposition())
            render()
            return
        }
        spellingChoices = T9Native.leading(t9.text)
        spellingPage = 0
        val query = T9Native.segmentations(t9.query, T9_SEGMENTATIONS).firstOrNull()
        if (query == null) {
            // 没有合法切分：清掉组字，编码栏直接显示数字串
            applyOutcome(c.clearComposition())
            preedit = t9.text
        } else {
            applyOutcome(c.replaceComposition(query))
        }
        render()
    }

    private fun chooseSpelling(index: Int) {
        val syllable = spellingChoices.getOrNull(index) ?: return
        T9Native.pin(t9.text, syllable)?.let { t9.replaceWith(it) }
        spellingOpen = false
        refreshT9()
    }

    // ── 候选 ────────────────────────────────────────────────────

    private fun candidateTapped(index: Int) {
        if (punctuationOpen) {
            PunctuationLayout.STRIP.getOrNull(index)?.let {
                punctuationOpen = false
                punctuate(it)
            }
            render()
            return
        }
        val c = core ?: return
        if (!composing || index >= page.items.size) return
        applyOutcome(c.selectCandidate(pageStart + index))
    }

    private fun candidatePage(forward: Boolean) {
        if (spellingOpen) {
            val last = ((spellingChoices.size - 1) / PAGE) * PAGE
            spellingPage = (spellingPage + if (forward) PAGE else -PAGE).coerceIn(0, last.coerceAtLeast(0))
        } else if (composing) {
            val nextStart = pageStart + if (forward) PAGE else -PAGE
            if (nextStart < 0 || (forward && nextStart >= page.total)) return
            pageStart = nextStart
            pullPage()
            candidateScroll.scrollTo(0, 0)
        }
        render()
    }

    private fun toggleMatrix() {
        if (matrixExpanded) {
            matrixExpanded = false
            render()
            return
        }
        if (!composing || page.items.isEmpty()) return
        matrixExpanded = true
        matrixItems = emptyList()
        loadMatrix()
        render()
    }

    /** 矩阵展开后按需续取（XSIME：每次多拉 27 个）。 */
    private fun loadMatrix() {
        val c = core ?: return
        if (!matrixExpanded) return
        if (matrixItems.isNotEmpty() && matrixItems.size >= matrixTotal) return
        val w = c.candidates(matrixItems.size, if (matrixItems.isEmpty()) PAGE * 4 else MATRIX_MORE)
        matrixTotal = w.total
        matrixItems = matrixItems + w.items.mapIndexed { i, text -> CandidateMatrixView.Item(w.offset + i, text, "") }
    }

    // ── Buffer ──────────────────────────────────────────────────

    private fun toggleBuffer() {
        val b = buffer
        if (b == null) {
            Toast.makeText(this, "缓冲不可用（原生库未加载）", Toast.LENGTH_SHORT).show()
            return
        }
        // 组字中不切换（XSIME：Buffer cannot toggle during composition）
        if (!policy.allowBuffer || composing) return
        retireTouches()
        bufferMode = !bufferMode
        if (bufferMode) {
            // 打开 Buffer 即绑定当前字段（buffer.md §3）
            tracker.current?.let { b.bind(it, policy.isPrivate) }
        } else {
            // 正文是临时数据（buffer.md 契约 #12），关掉即丢弃，不发送
            b.discard()
            bufferRail.clearProjection()
        }
        render()
    }

    private fun clearBuffer() {
        val b = buffer ?: return
        if (!bufferMode) return
        b.clear()
        bufferRail.clearProjection()
        render()
    }

    /** 两段式投递：取出 → 写入前复核授权 → 按 `commitText` 结果报告（buffer.md §2）。被拒时内容保留，可再点重试。 */
    private fun deliverBuffer(all: Boolean) {
        val b = buffer ?: return
        if (composing) return
        val t = tracker.current ?: return
        if (b.state() != NativeBuffer.State.READY) {
            if (b.state() == NativeBuffer.State.HELD) b.releaseHold()
            tracker.current?.let { b.bind(it, policy.isPrivate) }
        }
        val text = b.begin(t, all)
        if (text == null) {
            Toast.makeText(
                this,
                when (b.state()) {
                    NativeBuffer.State.UNBOUND, NativeBuffer.State.SUSPENDED -> getString(R.string.buffer_unbound)
                    NativeBuffer.State.HELD -> getString(R.string.delivery_pending)
                    NativeBuffer.State.READY -> getString(R.string.buffer_empty)
                },
                Toast.LENGTH_SHORT,
            ).show()
            render()
            return
        }
        val ic = currentInputConnection
        if (ic == null) {
            // 连接已断开、无法确认是否写入：保留并暂停，不自动重发
            b.finish(NativeBuffer.WriteResult.UNKNOWN)
            render()
            return
        }
        if (!b.isAuthorized(t)) {
            render()
            return
        }
        val ok = ic.commitText(text, 1)
        b.finish(if (ok) NativeBuffer.WriteResult.ACCEPTED else NativeBuffer.WriteResult.REJECTED)
        if (!ok) Toast.makeText(this, R.string.delivery_rejected, Toast.LENGTH_SHORT).show()
        render()
    }

    // ── 剪贴板（核心那份历史）───────────────────────────────────

    private fun openClipboard() {
        if (directOnly || core == null) return
        retireTouches()
        readCoreConfig()
        appearanceOpen = false
        clipboardOpen = true
        render()
    }

    private fun startClipListening() {
        if (clipListening) return
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        cm.addPrimaryClipChangedListener(clipListener)
        clipListening = true
    }

    private fun stopClipListening() {
        if (!clipListening) return
        getSystemService(ClipboardManager::class.java)?.removePrimaryClipChangedListener(clipListener)
        clipListening = false
    }

    /**
     * 剪贴板变化（或面板上的「收录当前文字」）。Android 10+ 只有前台的默认输入法读得到剪贴板，
     * 所以只在键盘可见时读（XSIME 吸收计划 §2.4）；私密字段不读，平台标记敏感的连同字段私密一起报给核心。
     * 收不收由核心定：`clipboard.enabled`、隐私环境、空白、过长。
     */
    private fun onClipChanged(explicit: Boolean) {
        if (!ClipboardGate.shouldCapture(keyboardVisible, policy)) return
        val c = core ?: return
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        val clip = try {
            cm.primaryClip
        } catch (e: SecurityException) {
            null
        }
        if (clip == null || clip.itemCount == 0) {
            if (explicit) Toast.makeText(this, R.string.clipboard_empty, Toast.LENGTH_SHORT).show()
            return
        }
        val platformSensitive = clip.description?.extras?.getBoolean(EXTRA_IS_SENSITIVE, false) == true
        // 只取文字：不把 URI / 图片强转成文本，不打开文件
        val text = clip.getItemAt(0).text?.toString()
        if (text.isNullOrEmpty()) {
            if (explicit) Toast.makeText(this, R.string.clipboard_empty, Toast.LENGTH_SHORT).show()
            return
        }
        c.clipboardChanged(text, ClipboardGate.sensitive(platformSensitive, policy))
        if (clipboardOpen) render()
    }

    private val clipboardListener = object : TextClipboardPanel.Listener {
        override fun collect() = onClipChanged(explicit = true)

        override fun select(entry: ClipEntry) {
            clipboardOpen = false
            clipboardPanel.redact()
            val b = buffer
            if (bufferMode && b != null) {
                if (b.pushPhrase(entry.text) != 0) Toast.makeText(this@MoodflowImeService, R.string.buffer_limit, Toast.LENGTH_SHORT).show()
            } else {
                settleComposition()
                currentInputConnection?.commitText(entry.text, 1)
            }
            render()
        }

        override fun pin(entry: ClipEntry) {
            core?.clipboardSetPinned(entry.id, !entry.pinned)
            render()
        }

        override fun remove(entry: ClipEntry) {
            core?.clipboardRemove(entry.id)
            render()
        }

        override fun clear() {
            core?.clipboardClearUnpinned()
            render()
        }

        override fun enable() {
            val c = core ?: return
            val err = c.configSet("clipboard.enabled", MoodflowCore.ConfigType.BOOL, "true")
            if (err != null) {
                Toast.makeText(this@MoodflowImeService, err, Toast.LENGTH_LONG).show()
                return
            }
            c.runCommand(MoodflowCore.Command.RELOAD_CONFIG)
            readCoreConfig()
            render()
        }

        override fun close() {
            clipboardOpen = false
            clipboardPanel.redact()
            render()
        }
    }

    private fun openSettings() {
        startActivity(Intent(this, SettingsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    // ── 并击 ────────────────────────────────────────────────────

    private val chordHandler = object : ChordSurface.Handler {
        override fun onChord(result: ChordProfile.Resolution) {
            chordPreviewState = null
            if (result.single()) {
                // 单键按普通键打（`,` `.` 走标点）
                val k = result.keys
                if (k == "," || k == ".") punctuate(if (chinesePages()) (if (k == ",") "，" else "。") else k) else type(k)
                lastChordSyllable = false
            } else {
                val names = result.keys.map { ChordKeys.name(it) }
                handleChord(chord?.settleBatch(names, SystemClock.uptimeMillis()))
            }
            render()
        }

        override fun onKey(text: String) {
            type(text)
            render()
        }

        override fun onPreview(preview: ChordGesture.Preview?) {
            chordPreviewState = preview
            render()
        }

        override fun onControl(action: ChordLayout.Action) {
            if (action == ChordLayout.Action.DELETE) {
                delete()
            } else {
                emoji = true
                numeric = false
            }
            render()
        }

        override fun label(action: ChordLayout.Action): String = if (action == ChordLayout.Action.DELETE) "⌫" else "☺"

        override fun description(action: ChordLayout.Action): String =
            if (action == ChordLayout.Action.DELETE) getString(R.string.backspace) else "表情"
    }

    /**
     * 结算结果送进拼音引擎。完整音节之间补隔音符：`xi` 后面接 `an` 不能被引擎读成 `xian`
     * （XSIME「每个多键批次（兼容飞耀）」的边界策略）。
     */
    private fun handleChord(r: ChordResult?) {
        val c = engine() ?: return
        when (r) {
            is ChordResult.Emit -> {
                val boundary = !r.fragment && lastChordSyllable && composing
                applyOutcome(c.feedText(if (boundary) "'" + r.pinyin else r.pinyin))
                lastChordSyllable = !r.fragment
            }
            ChordResult.NoMatch, null -> Unit
        }
    }

    private fun chordControl(action: KeyboardLayout.Action) {
        retireTouches()
        when (action) {
            KeyboardLayout.Action.NUMBERS -> {
                numeric = !numeric
                numberPageFresh = numeric
                symbols = false
            }
            KeyboardLayout.Action.SHIFT -> uppercase = !uppercase
            KeyboardLayout.Action.LANGUAGE -> toggleLanguage()
            KeyboardLayout.Action.SPACE -> space()
            KeyboardLayout.Action.RETURN -> enter()
            else -> Unit
        }
        render()
    }

    // ── 物理键盘 ────────────────────────────────────────────────

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val c = engine()
        if (c == null || tracker.current == null) return super.onKeyDown(keyCode, event)
        val mods = PhysicalKeys.toMods(event.metaState)
        val ch = chord
        val name = PhysicalKeys.chordKeyName(keyCode)
        if (chordVisible() && chinese && ch != null && name != null &&
            mods and (Mods.CTRL or Mods.ALT) == 0 && ch.isChordKey(name)
        ) {
            // 并击：按下只记账，全部松开时结算（moodflow-chord）
            if (event.repeatCount == 0) handleChord(ch.keyDown(name, event.eventTime))
            return true
        }
        val vk = PhysicalKeys.toVk(keyCode) ?: return super.onKeyDown(keyCode, event)
        val o = c.keyDown(vk, mods)
        if (!o.consumed) return super.onKeyDown(keyCode, event)
        swallowedKeyUps += keyCode
        applyOutcome(o)
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        val ch = chord
        val name = PhysicalKeys.chordKeyName(keyCode)
        if (engine() != null && chordVisible() && chinese && ch != null && name != null && ch.isChordKey(name)) {
            handleChord(ch.keyUp(name, event.eventTime))
            // 先敲一只手时要等另一只手：超时后单独结算
            main.removeCallbacks(chordTick)
            main.postDelayed(chordTick, CHORD_TICK_SHORT_MS)
            main.postDelayed(chordTick, CHORD_TICK_LONG_MS)
            return true
        }
        if (swallowedKeyUps.remove(keyCode)) return true
        return super.onKeyUp(keyCode, event)
    }

    // ── 输出 ────────────────────────────────────────────────────

    /** 把核心的编辑指令写进宿主或 Buffer。 */
    private fun applyOutcome(o: KeyOutcome) {
        val ic = currentInputConnection
        val sink: TextSink = if (ic != null) IcSink(ic) else NullSink
        val rejected = OpRouter.apply(o.ops, sink, if (bufferMode) bufferSink else null)
        if (rejected.isNotEmpty()) Toast.makeText(this, R.string.buffer_limit, Toast.LENGTH_SHORT).show()
        if (o.ops.any { it is EditOp.Commit }) {
            lastChordSyllable = false
            if (!t9.isEmpty) {
                t9.clear()
                spellingChoices = emptyList()
                spellingOpen = false
            }
        }
        if (o.modeChanged) refreshSchemas()
        render()
    }

    /** 不经引擎直接上屏（私密字段、引擎未就绪、核心不收的键）。Buffer 开着时进 Buffer。 */
    private fun commitRaw(text: String) {
        val b = buffer
        if (bufferMode && b != null) {
            val ok = if (text.all { it.code in 0x21..0x7E }) {
                text.all { b.pushLatin(it.code) == 0 }
            } else {
                b.pushPhrase(text) == 0
            }
            if (!ok) Toast.makeText(this, R.string.buffer_limit, Toast.LENGTH_SHORT).show()
            return
        }
        currentInputConnection?.commitText(text, 1)
    }

    private fun clearHostComposition() {
        currentInputConnection?.let {
            it.setComposingText("", 1)
            it.finishComposingText()
        }
    }

    private fun resetLocalComposition() {
        preedit = ""
        composing = false
        pageStart = 0
        page = CandidateWindow.EMPTY
        matrixExpanded = false
        t9.clear()
        spellingChoices = emptyList()
        spellingOpen = false
        lastChordSyllable = false
    }

    private fun notice(): String = when {
        tracker.current != null && policy.isPrivate -> getString(R.string.private_field)
        coreFailed -> getString(R.string.engine_failed)
        core == null -> getString(R.string.engine_loading)
        !storeAvailable -> getString(R.string.store_unavailable)
        buffer?.state() == NativeBuffer.State.HELD && bufferMode -> getString(R.string.delivery_pending)
        else -> ""
    }

    // ── 视图 ────────────────────────────────────────────────────

    override fun onCreateInputView(): View {
        chromeButtons.clear()
        chordControls.clear()
        candidateSlots.clear()
        spellingButtons.clear()
        val root = KeyboardRoot(this)
        keyboard = root
        root.orientation = LinearLayout.VERTICAL
        if (Build.VERSION.SDK_INT >= 29) root.isForceDarkAllowed = false
        root.layoutDirection = View.LAYOUT_DIRECTION_LTR
        root.setPadding(dp(5), dp(5), dp(5), dp(5))
        root.addOnLayoutChangeListener { _, l, _, r, _, ol, _, or, _ -> if (r - l != or - ol) render() }
        root.setOnApplyWindowInsetsListener { view, insets ->
            val left: Int
            val right: Int
            val bottom: Int
            if (Build.VERSION.SDK_INT >= 30) {
                val safe = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                left = safe.left
                right = safe.right
                bottom = safe.bottom
            } else {
                @Suppress("DEPRECATION")
                left = insets.systemWindowInsetLeft
                @Suppress("DEPRECATION")
                right = insets.systemWindowInsetRight
                @Suppress("DEPRECATION")
                bottom = insets.systemWindowInsetBottom
            }
            view.setPadding(dp(5) + left, dp(5), dp(5) + right, dp(5) + bottom)
            insets
        }

        // 状态行（28dp）：组字串、引擎准备中、私密字段等提示；没有内容时收起
        status = TextView(this)
        status.isSingleLine = true
        status.textSize = 14f
        status.gravity = Gravity.CENTER_VERTICAL
        status.setPadding(dp(8), 0, dp(8), 0)
        root.addView(status, LinearLayout.LayoutParams(-1, dp(28)))

        // Buffer：上行 轨 + 插入；下行 统计
        bufferRow = LinearLayout(this)
        bufferRow.orientation = LinearLayout.VERTICAL
        val bufferParams = LinearLayout.LayoutParams(-1, dp(if (landscape()) 60 else 76))
        bufferParams.bottomMargin = dp(4)
        root.addView(bufferRow, bufferParams)
        val railHeight = if (landscape()) 28 else 36
        val top = row(bufferRow, railHeight)
        bufferRail = BufferRail(this)
        top.addView(bufferRail, LinearLayout.LayoutParams(0, -1, 1f))
        insertNext = button(top, "↑", 0f) { deliverBuffer(all = false) }
        fixedWidth(insertNext, 40)
        gapLeft(insertNext, 4)
        insertNext.icon(KeyboardIcon.PAPER_PLANE)
        insertNext.appearance(false, false, true)
        insertNext.contentDescription = getString(R.string.insert_next)
        insertNext.setOnLongClickListener {
            deliverBuffer(all = true)
            true
        }
        val bottom = LinearLayout(this)
        bufferRow.addView(bottom, LinearLayout.LayoutParams(-1, dp(railHeight)))
        bufferMetrics = TextView(this)
        bufferMetrics.gravity = Gravity.CENTER
        bufferMetrics.textSize = if (landscape()) 14f else 16f
        bottom.addView(bufferMetrics, LinearLayout.LayoutParams(0, -1, 1f))
        val clear = button(bottom, getString(R.string.clear), 0f) { clearBuffer() }
        fixedWidth(clear, 40)
        gapLeft(clear, 4)
        clear.icon(KeyboardIcon.CLEAR)
        clear.contentDescription = getString(R.string.clear)

        // 候选行（32dp）：设置 ‹ 候选 › 展开 Buffer
        candidateRow = row(root, 32)
        settingsButton = button(candidateRow, "设置", 0f) { toggleAppearance() }
        fixedWidth(settingsButton, 32)
        gapRight(settingsButton, 4)
        settingsButton.icon(KeyboardIcon.SLIDERS)
        settingsButton.contentDescription = "键位、方案与配色"
        settingsButton.setOnLongClickListener {
            openSettings()
            true
        }
        previous = button(candidateRow, "‹", 0f) { candidatePage(false) }
        fixedWidth(previous, 32)
        previous.plain(true)
        previous.icon(KeyboardIcon.CHEVRON_LEFT, 16f)
        previous.contentDescription = "上一页候选"
        val center = FrameLayout(this)
        candidateRow.addView(center, LinearLayout.LayoutParams(0, -1, 1f))
        candidateScroll = HorizontalScrollView(this)
        candidateScroll.isFillViewport = false
        candidateScroll.isHorizontalScrollBarEnabled = false
        val strip = LinearLayout(this)
        candidateScroll.addView(strip, FrameLayout.LayoutParams(-2, -1))
        center.addView(candidateScroll, FrameLayout.LayoutParams(-1, -1))
        for (i in 0 until maxOf(PAGE, PunctuationLayout.STRIP.size)) {
            val slot = button(strip, "", 0f) { candidateTapped(i) }
            slot.layoutParams = LinearLayout.LayoutParams(-2, -1)
            slot.fontStyle(false, 20, false)
            slot.plain(true)
            slot.minWidth = dp(32)
            slot.minimumWidth = dp(32)
            slot.setPadding(dp(6), 0, dp(6), 0)
            gapRight(slot, 4)
            slot.setAutoSizeTextTypeWithDefaults(TextView.AUTO_SIZE_TEXT_TYPE_NONE)
            slot.textSize = 20f
            candidateSlots += slot
        }
        candidateMatrix = CandidateMatrixView(
            this,
            object : CandidateMatrixView.Listener {
                override fun select(item: CandidateMatrixView.Item) {
                    val c = core ?: return
                    if (!matrixExpanded || !composing) return
                    matrixExpanded = false
                    applyOutcome(c.selectCandidate(item.index))
                }

                override fun more() {
                    val before = matrixItems.size
                    loadMatrix()
                    if (matrixItems.size != before) render()
                }
            },
        )
        center.addView(candidateMatrix, FrameLayout.LayoutParams(-1, -1))
        chordReadout = ChordPreview(this)
        center.addView(chordReadout, FrameLayout.LayoutParams(-1, -1))
        // 空闲时（没有组字）的快捷入口
        idleRow = LinearLayout(this)
        idleRow.gravity = Gravity.CENTER_VERTICAL
        center.addView(idleRow, FrameLayout.LayoutParams(-1, -1))
        val clipShortcut = button(idleRow, getString(R.string.clipboard_history), 0f) { openClipboard() }
        clipShortcut.layoutParams = LinearLayout.LayoutParams(-2, -1)
        clipShortcut.shortcut(true)
        clipShortcut.icon(KeyboardIcon.STACK_LAYERS, 16f, true)
        clipShortcut.setPadding(dp(10), 0, dp(10), 0)
        gapRight(clipShortcut, 6)
        val settingsShortcut = button(idleRow, getString(R.string.settings_title), 0f) { openSettings() }
        settingsShortcut.layoutParams = LinearLayout.LayoutParams(-2, -1)
        settingsShortcut.shortcut(true)
        settingsShortcut.icon(KeyboardIcon.SETTINGS, 16f, true)
        settingsShortcut.setPadding(dp(10), 0, dp(10), 0)
        next = button(candidateRow, "›", 0f) { candidatePage(true) }
        fixedWidth(next, 32)
        next.plain(true)
        next.icon(KeyboardIcon.CHEVRON_RIGHT, 16f)
        next.contentDescription = "下一页候选"
        expand = button(candidateRow, "⌃", 0f) { toggleMatrix() }
        fixedWidth(expand, 32)
        expand.plain(true)
        bufferButton = button(candidateRow, "▤", 0f) { toggleBuffer() }
        fixedWidth(bufferButton, 32)
        gapLeft(bufferButton, 4)
        bufferButton.appearance(false, false, true)
        bufferButton.icon(KeyboardIcon.STACK_LAYERS)

        // 九宫格「选拼音」行
        spellingRow = row(root, 34)
        for (i in 0 until PAGE) {
            spellingButtons += button(spellingRow, "", 1f) { chooseSpelling(spellingPage + i) }
        }

        // 键区：标准键盘 / 并击 / 外观面板 / 剪贴板面板叠在同一块
        surfaceContainer = FrameLayout(this)
        keys = KeyboardSurface(this, keyHandler)
        surfaceContainer.addView(keys, FrameLayout.LayoutParams(-1, -1))
        chords = chordProfile?.let { profile ->
            val surface = ChordSurface(this, chordHandler, profile)
            surfaceContainer.addView(surface, FrameLayout.LayoutParams(-1, -1))
            surface
        }
        appearancePanel = KeyboardAppearancePanel(this, { chooseLayout(it) }, { settings.setTheme(it) }) { toggleAppearance() }
        appearancePanel.action("全部插入", getString(R.string.insert_all), KeyboardIcon.SEND_ALL) { deliverBuffer(all = true) }
        appearancePanel.action("清空 Buffer", getString(R.string.clear), KeyboardIcon.CLEAR) { clearBuffer() }
        appearancePanel.action(getString(R.string.clipboard_history), getString(R.string.clipboard_history), KeyboardIcon.STACK_LAYERS) { openClipboard() }
        appearancePanel.action(getString(R.string.settings_title), getString(R.string.settings_title), KeyboardIcon.SETTINGS) { openSettings() }
        appearancePanel.action("系统键盘", getString(R.string.switch_keyboard), KeyboardIcon.GLOBE) {
            getSystemService(InputMethodManager::class.java)?.showInputMethodPicker()
        }
        surfaceContainer.addView(appearancePanel, FrameLayout.LayoutParams(-1, -1))
        clipboardPanel = TextClipboardPanel(this, clipboardListener)
        surfaceContainer.addView(clipboardPanel, FrameLayout.LayoutParams(-1, -1))
        root.addView(surfaceContainer, LinearLayout.LayoutParams(-1, dp(KeyboardLayout.height(landscape()))))
        root.setInputSurface(surfaceContainer)

        // 并击布局的底栏：123 ⇧ 空格 中/英 回车
        chordFooter = row(root, if (landscape()) 34 else 40)
        (chordFooter.layoutParams as LinearLayout.LayoutParams).bottomMargin = 0
        for (action in listOf(
            KeyboardLayout.Action.NUMBERS, KeyboardLayout.Action.SHIFT, KeyboardLayout.Action.SPACE,
            KeyboardLayout.Action.LANGUAGE, KeyboardLayout.Action.RETURN,
        )) {
            val b = button(chordFooter, "", if (action == KeyboardLayout.Action.SPACE) 3.5f else 1f) { chordControl(action) }
            b.classic(true)
            b.fontStyle(false, if (action == KeyboardLayout.Action.SHIFT) 17 else 14, true)
            b.appearance(action != KeyboardLayout.Action.SPACE, true, action == KeyboardLayout.Action.RETURN)
            if (chordControls.isNotEmpty()) gapLeft(b, 2)
            chordControls += b
        }
        chords?.let { surface ->
            val repeat = DeleteRepeatTouch(surface.utilityButton(ChordLayout.Action.DELETE)) { tracker.current != null }
            surface.setUtilityRetirementListener { repeat.cancel() }
        }
        render()
        return root
    }

    private fun toggleAppearance() {
        retireTouches()
        clipboardOpen = false
        appearanceOpen = !appearanceOpen
        if (appearanceOpen) appearancePanel.showPets()
        render()
    }

    private fun render() {
        val root = keyboard ?: return
        val palette = Themes.palette(this, theme)
        root.setBackgroundColor(palette.background)
        chromeButtons.forEach { it.theme(theme) }

        // 状态行：组字串（Buffer 关时）或提示
        val note = notice()
        val statusText = when {
            composing && !bufferMode && preedit.isNotEmpty() -> preedit
            !t9.isEmpty && !bufferMode && preedit.isEmpty() -> t9.text
            else -> note
        }
        if (status.text.toString() != statusText) status.text = statusText
        status.setTextColor(if (statusText == note) palette.accentText else palette.ink)
        status.visibility = if (statusText.isEmpty()) View.GONE else View.VISIBLE

        // 候选行
        val chordHeld = chordPreviewState != null
        val marks = punctuationOpen
        val idle = !composing && !marks && !chordHeld && t9.isEmpty
        if (matrixExpanded && (!composing || marks || chordHeld)) matrixExpanded = false
        bufferButton.contentDescription = getString(if (bufferMode) R.string.buffer_on else R.string.buffer_off)
        bufferButton.isSelected = bufferMode
        bufferButton.isEnabled = policy.allowBuffer && buffer != null && !composing
        bufferButton.visibility = if (policy.allowBuffer) View.VISIBLE else View.GONE
        idleRow.visibility = if (idle && !directOnly && !matrixExpanded) View.VISIBLE else View.GONE
        candidateScroll.visibility = if (!idle && !matrixExpanded && !chordHeld) View.VISIBLE else View.GONE
        chordReadout.visibility = if (chordHeld) View.VISIBLE else View.GONE
        chordReadout.render(chordPreviewState, theme, landscape())
        candidateMatrix.visibility = if (matrixExpanded) View.VISIBLE else View.GONE
        candidateMatrix.update(matrixItems, theme, true)
        val candidateHeight = dp(if (matrixExpanded) (if (landscape()) 68 else 104) else 32)
        if (candidateRow.layoutParams.height != candidateHeight) {
            candidateRow.layoutParams.height = candidateHeight
            candidateRow.requestLayout()
        }
        expand.visibility = if (composing && page.items.isNotEmpty() && !marks && !chordHeld) View.VISIBLE else View.GONE
        expand.text = if (matrixExpanded) "⌄" else "⌃"
        expand.contentDescription = if (matrixExpanded) "收起候选" else "展开候选"
        val items = when {
            marks -> PunctuationLayout.STRIP
            composing -> page.items
            else -> emptyList()
        }
        for ((i, slot) in candidateSlots.withIndex()) {
            val exists = i < items.size
            slot.visibility = if (exists) View.VISIBLE else View.GONE
            if (exists) {
                slot.candidate(items[i], "")
                slot.contentDescription = if (marks) items[i] else "候选${pageStart + i + 1} ${items[i]}"
            }
            slot.isEnabled = exists
        }
        val candidateKey = "$pageStart|$preedit|$marks"
        if (candidateKey != renderedCandidateKey) {
            candidateScroll.scrollTo(0, 0)
            renderedCandidateKey = candidateKey
        }
        val paging = !matrixExpanded && !idle && !marks && !chordHeld
        previous.visibility = if (paging && (spellingOpen || pageStart > 0)) View.VISIBLE else View.GONE
        next.visibility = if (paging) View.VISIBLE else View.GONE
        previous.isEnabled = if (spellingOpen) spellingPage > 0 else pageStart > 0
        next.isEnabled = if (spellingOpen) spellingPage + PAGE < spellingChoices.size else pageStart + PAGE < page.total

        // Buffer 轨
        bufferRow.visibility = if (bufferMode) View.VISIBLE else View.GONE
        if (bufferMode) {
            val blocks = buffer?.blocks().orEmpty()
            bufferRail.render(blocks, if (composing) preedit else t9.text, theme, landscape())
            val text = blocks.joinToString("")
            bufferMetrics.text = "${text.codePointCount(0, text.length)} 字 · ${blocks.size} 块"
            bufferMetrics.setTextColor(palette.ink)
            insertNext.isEnabled = !composing && blocks.isNotEmpty()
            insertNext.isSelected = insertNext.isEnabled
        }

        // 选拼音行
        if (spellingChoices.isEmpty()) spellingOpen = false
        if (spellingPage >= spellingChoices.size) spellingPage = 0
        spellingRow.visibility = if (spellingOpen) View.VISIBLE else View.GONE
        for ((i, b) in spellingButtons.withIndex()) {
            val value = spellingChoices.getOrNull(spellingPage + i)
            b.visibility = if (value != null) View.VISIBLE else View.GONE
            if (value != null && b.text.toString() != value) {
                b.text = value
                b.contentDescription = "拼音 $value"
            }
        }

        // 键区
        val chord = chordVisible() && chords != null
        val panelOpen = appearanceOpen || clipboardOpen
        val density = resources.displayMetrics.density
        val width = (if (root.width > 0) root.width else resources.displayMetrics.widthPixels) - root.paddingLeft - root.paddingRight
        var surfaceHeight = if (chord) {
            ChordLayout.height(maxOf(1f, width / density), layoutPref == KeyboardChoice.SPLIT_ORTHOGONAL)
        } else {
            KeyboardLayout.height(landscape()).toFloat()
        }
        if (chord && panelOpen) surfaceHeight += if (landscape()) 35 else 41
        val desired = Math.round(surfaceHeight * density)
        if (surfaceContainer.layoutParams.height != desired) {
            surfaceContainer.layoutParams.height = desired
            surfaceContainer.requestLayout()
        }
        keys.render(visibleMode(), chinesePages(), theme)
        keys.visibility = if (panelOpen || chord) View.GONE else View.VISIBLE
        chords?.let {
            it.render(layoutPref == KeyboardChoice.SPLIT_ORTHOGONAL, chinese && !uppercase && engine() != null, uppercase, theme)
            it.visibility = if (panelOpen || !chord) View.GONE else View.VISIBLE
        }
        chordFooter.visibility = if (!panelOpen && chord) View.VISIBLE else View.GONE
        val footerLabels = listOf("123", if (uppercase) "⇪" else "⇧", "空格", if (chinese && !directOnly) "中" else "EN", returnLabel())
        val footerDescriptions = listOf("数字与字母", "Shift", getString(R.string.space), "中英切换", getString(R.string.enter))
        val keyWidth = Math.round((width / density - 10) / 7.5f * density)
        for ((i, b) in chordControls.withIndex()) {
            if (b.text.toString() != footerLabels[i]) b.text = footerLabels[i]
            b.contentDescription = footerDescriptions[i]
            val params = b.layoutParams as LinearLayout.LayoutParams
            val w = if (i == 2) 0 else keyWidth
            val weight = if (i == 2) 1f else 0f
            if (params.width != w || params.weight != weight) {
                params.width = w
                params.weight = weight
                b.requestLayout()
            }
            b.icon(if (i == 1) (if (uppercase) KeyboardIcon.SHIFT_FILL else KeyboardIcon.SHIFT) else null)
            b.isSelected = i == 1 && uppercase || i == 3 && !chinese || i == 4 && returnSelected()
        }

        appearancePanel.visibility = if (appearanceOpen) View.VISIBLE else View.GONE
        if (appearanceOpen) {
            appearancePanel.schemes(schemas.map { it.id }, schemas.map { it.name }, activeSchema()) { chooseSchema(it) }
            appearancePanel.chordAvailable(chords != null)
            appearancePanel.render(layoutPref, theme)
        }
        clipboardPanel.visibility = if (clipboardOpen) View.VISIBLE else View.GONE
        if (clipboardOpen) {
            clipboardPanel.render(core?.clipboardEntries().orEmpty(), theme, clipboardEnabled)
            clipboardPanel.bringToFront()
        }
    }

    private fun row(parent: LinearLayout, height: Int): LinearLayout {
        val row = LinearLayout(this)
        row.orientation = LinearLayout.HORIZONTAL
        val params = LinearLayout.LayoutParams(-1, dp(height))
        params.bottomMargin = dp(4)
        parent.addView(row, params)
        return row
    }

    private fun button(row: LinearLayout, text: String, weight: Float, action: () -> Unit): KeyButton {
        val b = KeyButton(this)
        b.text = text
        b.font(15)
        b.appearance(false, false, false)
        b.setOnClickListener { action() }
        row.addView(b, LinearLayout.LayoutParams(0, -1, weight))
        chromeButtons += b
        return b
    }

    private fun fixedWidth(view: View, width: Int) {
        view.layoutParams = LinearLayout.LayoutParams(dp(width), -1)
    }

    private fun gapLeft(view: View, gap: Int) {
        (view.layoutParams as LinearLayout.LayoutParams).leftMargin = dp(gap)
    }

    private fun gapRight(view: View, gap: Int) {
        (view.layoutParams as LinearLayout.LayoutParams).rightMargin = dp(gap)
    }

    private fun landscape() = resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE

    private fun dp(v: Int): Int = Math.round(v * resources.displayMetrics.density)

    /** `InputConnection` → [TextSink]。 */
    private class IcSink(private val ic: InputConnection) : TextSink {
        override fun setComposing(text: String, caret: Int) {
            if (text.isEmpty()) {
                ic.setComposingText("", 1)
                ic.finishComposingText()
            } else {
                ic.setComposingText(text, 1)
            }
        }

        override fun commit(text: String) {
            ic.commitText(text, 1)
        }

        override fun deleteBefore(codePoints: Int) {
            ic.deleteSurroundingTextInCodePoints(codePoints, 0)
        }

        override fun replaceBefore(codePoints: Int, text: String) {
            // 同一个批量编辑：宿主撤销栈记成一步
            ic.beginBatchEdit()
            ic.deleteSurroundingTextInCodePoints(codePoints, 0)
            ic.commitText(text, 1)
            ic.endBatchEdit()
        }

        override fun moveCursor(delta: Int) {
            val et = ic.getExtractedText(ExtractedTextRequest(), 0) ?: return
            val pos = (et.startOffset + et.selectionEnd + delta).coerceAtLeast(0)
            ic.setSelection(pos, pos)
        }
    }

    private object NullSink : TextSink {
        override fun setComposing(text: String, caret: Int) = Unit
        override fun commit(text: String) = Unit
        override fun deleteBefore(codePoints: Int) = Unit
        override fun replaceBefore(codePoints: Int, text: String) = Unit
        override fun moveCursor(delta: Int) = Unit
    }

    private companion object {
        const val TAG = "Moodflow"
        /** 每页候选数（XSIME：nine candidates per page） */
        const val PAGE = 9
        const val MATRIX_MORE = 27
        const val T9_SEGMENTATIONS = 8
        const val CHORD_TICK_SHORT_MS = 400L
        const val CHORD_TICK_LONG_MS = 2100L

        /** `ClipDescription.EXTRA_IS_SENSITIVE`（API 33 起的常量，按字面量用以兼容低版本）。 */
        const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
    }
}
