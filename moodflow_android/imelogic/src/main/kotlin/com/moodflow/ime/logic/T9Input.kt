package com.moodflow.ime.logic

/**
 * 九宫格的输入串状态（XSIME 吸收计划 §2.6）。
 *
 * 串由数字 `2`–`9`、用户在「选拼音」行钉住的字母、显式分隔符 `'` 组成，如 `ni'426`。
 * 切分与钉住的规则在 Rust（`moodflow_mobile::t9`）；这里只管串怎么增删，保证形状合法：
 * - 不以 `'` 开头，不出现连续 `'`；
 * - 退格按 XSIME 的规则（`NineKeyPinyin.backspace`）：删掉末尾一个字符；删的是钉住音节后的分隔符时，
 *   那个音节还原成数字（`ni'` → `64`），用户可以改选别的拼法。
 */
class T9Input {
    var text: String = ""
        private set

    val isEmpty: Boolean get() = text.isEmpty()

    /**
     * 「选拼音」行针对的音节段（按 `'` 分段，从 0 起）：第一段还含数字、即还没钉住的段。
     *
     * 不能直接取最后一段：用「1」键收尾的段（`64'`）也还没钉住，接着打 `426` 后仍要能回头给
     * `64` 选 ni / mi。全部钉完时指向最后一段（空段或已钉的音节），选拼音行为空。
     */
    val activeSegmentIndex: Int
        get() {
            val parts = text.split('\'')
            val unpinned = parts.indexOfFirst { part -> part.any { it in '2'..'9' } }
            return if (unpinned >= 0) unpinned else parts.lastIndex
        }

    /** 按数字键。非 `2`–`9` 忽略并返回 `false`。 */
    fun pressDigit(d: Char): Boolean {
        if (d !in '2'..'9') return false
        text += d
        return true
    }

    /** 按分隔键（九宫格的「1」键位）：空串或末尾已是分隔符时忽略。 */
    fun pressSeparator(): Boolean {
        if (text.isEmpty() || text.endsWith('\'')) return false
        text += '\''
        return true
    }

    fun backspace(): Boolean {
        if (text.isEmpty()) return false
        text = com.moodflow.ime.keyboard.NineKeyPinyin.backspace(text)
        return true
    }

    /**
     * 交给切分的串：就是 [text]，**保留**末尾的分隔符。
     *
     * 末尾的 `'` 表示「这一段已打完」（钉住音节之后、或按了「1」键）。去掉它，切分会把末段当成
     * 还在打的音节按前缀展开（`64'` 只该切 mi / ni，去掉后还会冒出 nian、ming…），按候选质量
     * 排序时就可能挑中用户没打的拼法。
     */
    val query: String get() = text

    /** 采用 Rust 侧钉住音节后的新串（`nativeT9Pin` 的返回值）。 */
    fun replaceWith(pinned: String) {
        text = pinned.trimStart('\'')
    }

    fun clear() {
        text = ""
    }

    companion object {
        /** 键面标注：数字键 → 字母。 */
        val LETTERS: Map<Char, String> = mapOf(
            '2' to "ABC", '3' to "DEF", '4' to "GHI", '5' to "JKL",
            '6' to "MNO", '7' to "PQRS", '8' to "TUV", '9' to "WXYZ",
        )
    }
}
