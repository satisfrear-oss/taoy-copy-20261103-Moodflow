package com.moodflow.ime.logic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PhysicalKeysTest {
    @Test
    fun lettersDigitsAndPunctuationMapToVk() {
        assertEquals(0x41, PhysicalKeys.toVk(PhysicalKeys.KEYCODE_A))
        assertEquals(0x5A, PhysicalKeys.toVk(PhysicalKeys.KEYCODE_Z))
        assertEquals(0x30, PhysicalKeys.toVk(PhysicalKeys.KEYCODE_0))
        assertEquals(Vk.BACK, PhysicalKeys.toVk(PhysicalKeys.KEYCODE_DEL))
        assertEquals(Vk.QUOTE, PhysicalKeys.toVk(PhysicalKeys.KEYCODE_APOSTROPHE))
        assertNull("音量键不归输入法", PhysicalKeys.toVk(24))
    }

    @Test
    fun metaStateMapsToModifiers() {
        assertEquals(Mods.SHIFT, PhysicalKeys.toMods(PhysicalKeys.META_SHIFT_ON))
        assertEquals(
            Mods.CTRL or Mods.ALT,
            PhysicalKeys.toMods(PhysicalKeys.META_CTRL_ON or PhysicalKeys.META_ALT_ON),
        )
    }

    @Test
    fun chordKeyNamesMatchCoreKeyTable() {
        assertEquals("a", PhysicalKeys.chordKeyName(PhysicalKeys.KEYCODE_A))
        assertEquals("semicolon", PhysicalKeys.chordKeyName(PhysicalKeys.KEYCODE_SEMICOLON))
        assertEquals("quote", PhysicalKeys.chordKeyName(PhysicalKeys.KEYCODE_APOSTROPHE))
        assertNull(PhysicalKeys.chordKeyName(PhysicalKeys.KEYCODE_ENTER))
    }
}

class StrokeTableTest {
    @Test
    fun decodesTriplesFirstWins() {
        val t = StrokeTable.fromFlat(
            intArrayOf('a'.code, 0x41, 0, 'A'.code, 0x41, 1, 'a'.code, 0x99, 1, 0x1F600, 1, 0),
        )
        assertEquals(Stroke(0x41, false), t['a'])
        assertEquals(Stroke(0x41, true), t['A'])
        assertEquals(2, t.size)
        assertEquals(0, StrokeTable.fromFlat(null).size)
    }
}

class T9InputTest {
    @Test
    fun digitsAndSeparators() {
        val t = T9Input()
        assertFalse(t.pressSeparator())
        assertTrue(t.pressDigit('6'))
        assertTrue(t.pressDigit('4'))
        assertFalse(t.pressDigit('1'))
        assertTrue(t.pressSeparator())
        assertFalse("不出现连续分隔符", t.pressSeparator())
        t.pressDigit('4')
        assertEquals("64'4", t.text)
    }

    @Test
    fun backspaceUnpinsTheLastSyllable() {
        val t = T9Input()
        t.replaceWith("ni'4")
        t.backspace()
        assertEquals("ni'", t.text)
        assertEquals("钉住音节后的分隔符原样交给切分", "ni'", t.query)
        t.backspace()
        assertEquals("删掉钉住音节的分隔符，音节还原成数字", "64", t.text)
        t.backspace()
        t.backspace()
        assertTrue(t.isEmpty)
        assertFalse(t.backspace())
    }

    @Test
    fun activeSegmentIsTheFirstUnpinnedOne() {
        val t = T9Input()
        t.replaceWith("ni'426")
        assertEquals(1, t.activeSegmentIndex)
        t.pressSeparator()
        assertEquals("「1」键收尾的段还没钉住，仍是它", 1, t.activeSegmentIndex)
        t.replaceWith("ni'hao'")
        assertEquals("全部钉完：指向末尾的空段", 2, t.activeSegmentIndex)
    }

    @Test
    fun segmentClosedWithSeparatorStaysPickable() {
        val t = T9Input()
        t.pressDigit('6')
        t.pressDigit('4')
        t.pressSeparator()
        "426".forEach { t.pressDigit(it) }
        assertEquals("64'426", t.text)
        assertEquals("先给 64 选拼音，而不是只能选 426", 0, t.activeSegmentIndex)
        t.replaceWith("ni'426")
        assertEquals(1, t.activeSegmentIndex)
    }

    @Test
    fun pinnedSyllableKeepsItsBoundaryInQuery() {
        val t = T9Input()
        t.replaceWith("fan'")
        assertEquals("末尾分隔符 = 音节已打完，不能去掉让切分再展开", "fan'", t.query)
        t.pressDigit('6')
        t.pressDigit('4')
        t.pressSeparator()
        assertEquals("fan'64'", t.query)
    }

    @Test
    fun keypadLabels() {
        assertEquals("PQRS", T9Input.LETTERS['7'])
        assertEquals(8, T9Input.LETTERS.size)
    }
}

class OpRouterTest {
    private class RecordingSink : TextSink {
        val log = mutableListOf<String>()
        override fun setComposing(text: String, caret: Int) { log += "S:$text:$caret" }
        override fun commit(text: String) { log += "C:$text" }
        override fun deleteBefore(codePoints: Int) { log += "D:$codePoints" }
        override fun replaceBefore(codePoints: Int, text: String) { log += "R:$codePoints:$text" }
        override fun moveCursor(delta: Int) { log += "M:$delta" }
    }

    private class RecordingBuffer(private val accept: Boolean) : BufferSink {
        val blocks = mutableListOf<String>()
        override fun pushPhrase(text: String): Boolean {
            if (accept) blocks += text
            return accept
        }
    }

    private val ops = listOf(
        EditOp.SetComposition("ni", 2),
        EditOp.Commit("你"),
        EditOp.SetComposition("", 0),
        EditOp.ReplaceBackward(1, "."),
        EditOp.DeleteBackward(1),
        EditOp.MoveCursor(-1),
    )

    @Test
    fun withoutBufferEverythingGoesToHost() {
        val sink = RecordingSink()
        val rejected = OpRouter.apply(ops, sink, null)
        assertTrue(rejected.isEmpty())
        assertEquals(listOf("S:ni:2", "C:你", "S::0", "R:1:.", "D:1", "M:-1"), sink.log)
    }

    @Test
    fun withBufferCommitsGoToBufferAndHostTextIsUntouched() {
        val sink = RecordingSink()
        val buffer = RecordingBuffer(accept = true)
        OpRouter.apply(ops, sink, buffer)
        assertEquals(listOf("你"), buffer.blocks)
        assertEquals("只清残留组字，不写组字、不删改宿主原文", listOf("S::0"), sink.log)
    }

    @Test
    fun bufferRejectionIsReportedNotRedirected() {
        val sink = RecordingSink()
        val rejected = OpRouter.apply(listOf(EditOp.Commit("长文")), sink, RecordingBuffer(accept = false))
        assertEquals(listOf("长文"), rejected)
        assertTrue("拒收的文本不改投宿主", sink.log.isEmpty())
    }
}
