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

    /** 当前待选音节的位置；每个显式分隔符开始一个新位置。 */
    val activeSegmentIndex: Int get() = text.count { it == '\'' }

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

    /** 交给切分的串：末尾的分隔符只是「本音节已结束」，不参与切分。 */
    val query: String get() = text.trimEnd('\'')

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
