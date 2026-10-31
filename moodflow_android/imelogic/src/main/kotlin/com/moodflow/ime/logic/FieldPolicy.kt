package com.moodflow.ime.logic

/**
 * 一个输入框的隐私与输入策略（XSIME 吸收计划 §2.1 / §2.4 的 Android 落地）。
 *
 * 判据只看 `EditorInfo.inputType` 与 `EditorInfo.imeOptions` 两个整数，常量照抄自
 * `android.text.InputType` / `android.view.inputmethod.EditorInfo`（公开 API，数值不会变），
 * 这样本类不依赖 Android SDK，能在 JVM 单测里覆盖全部分支。
 *
 * 私密字段 = 密码类（文本密码、可见密码、网页密码、数字密码）或宿主声明
 * `IME_FLAG_NO_PERSONALIZED_LEARNING`（无痕标签页等）。私密字段：
 * - **完全绕过引擎**：按键直接上屏，不进组字、不学词、不出候选。核心目前没有「本字段不学习但
 *   允许中文」的开关（路线图 §6.3 第 3 项 / 移植计划 A3），绕过是唯一能保证不学习的做法；
 *   代价是无痕字段里暂时只能打英文。
 * - 不启用 Buffer；
 * - 不采集剪贴板。
 */
data class FieldPolicy(
    val password: Boolean,
    val noPersonalizedLearning: Boolean,
    /** 输入类别（文本 / 数字 / 电话 / 日期），决定默认布局 */
    val inputClass: Int,
) {
    val isPrivate: Boolean get() = password || noPersonalizedLearning

    /** 按键是否交给引擎。非文本类别（数字、电话、日期）也直接上屏。 */
    val useEngine: Boolean get() = !isPrivate && inputClass == TYPE_CLASS_TEXT

    val allowBuffer: Boolean get() = !isPrivate

    val allowClipboardCapture: Boolean get() = !isPrivate

    /** 数字 / 电话 / 日期类字段默认给数字键盘。 */
    val prefersNumberPad: Boolean
        get() = inputClass == TYPE_CLASS_NUMBER || inputClass == TYPE_CLASS_PHONE ||
            inputClass == TYPE_CLASS_DATETIME

    companion object {
        // android.text.InputType
        const val TYPE_MASK_CLASS = 0x0000000f
        const val TYPE_MASK_VARIATION = 0x00000ff0
        const val TYPE_NULL = 0
        const val TYPE_CLASS_TEXT = 0x00000001
        const val TYPE_CLASS_NUMBER = 0x00000002
        const val TYPE_CLASS_PHONE = 0x00000003
        const val TYPE_CLASS_DATETIME = 0x00000004
        const val TYPE_TEXT_VARIATION_PASSWORD = 0x00000080
        const val TYPE_TEXT_VARIATION_VISIBLE_PASSWORD = 0x00000090
        const val TYPE_TEXT_VARIATION_WEB_PASSWORD = 0x000000e0
        const val TYPE_NUMBER_VARIATION_PASSWORD = 0x00000010

        // android.view.inputmethod.EditorInfo
        const val IME_FLAG_NO_PERSONALIZED_LEARNING = 0x01000000

        /** 尚无输入框时的策略：当私密处理，什么都不采集。 */
        val NONE = FieldPolicy(password = false, noPersonalizedLearning = true, inputClass = TYPE_NULL)

        fun of(inputType: Int, imeOptions: Int): FieldPolicy {
            val cls = inputType and TYPE_MASK_CLASS
            val variation = inputType and TYPE_MASK_VARIATION
            val password = when (cls) {
                TYPE_CLASS_TEXT -> variation == TYPE_TEXT_VARIATION_PASSWORD ||
                    variation == TYPE_TEXT_VARIATION_VISIBLE_PASSWORD ||
                    variation == TYPE_TEXT_VARIATION_WEB_PASSWORD
                TYPE_CLASS_NUMBER -> variation == TYPE_NUMBER_VARIATION_PASSWORD
                else -> false
            }
            return FieldPolicy(
                password = password,
                noPersonalizedLearning = imeOptions and IME_FLAG_NO_PERSONALIZED_LEARNING != 0,
                // TYPE_NULL（终端类宿主）按文本处理：它们不声明类别，但仍要能输入
                inputClass = if (cls == TYPE_NULL) TYPE_CLASS_TEXT else cls,
            )
        }
    }
}

/** 剪贴板采集闸门：键盘在前台、字段允许、平台未标记敏感，三者同时成立才收录。 */
object ClipboardGate {
    fun shouldCapture(keyboardVisible: Boolean, policy: FieldPolicy): Boolean =
        keyboardVisible && policy.allowClipboardCapture

    /** 收录时随文本上报的「敏感」：平台标记或当前字段私密（双保险，Rust 侧据此拒收）。 */
    fun sensitive(platformSensitive: Boolean, policy: FieldPolicy): Boolean =
        platformSensitive || policy.isPrivate
}
