package j2me.testing

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.objectweb.asm.ClassReader
import org.objectweb.asm.tree.ClassNode
import org.objectweb.asm.tree.LineNumberNode
import java.nio.file.Path
import kotlin.io.path.readBytes
import kotlin.io.path.writeText

class FixturesTest {
    @TempDir lateinit var root: Path

    @Test fun `default compilation retains source and lines without local variable names`() {
        assertEquals(DebugAttributes(source = true, lines = true, variables = false), compileAttributes())
    }

    @Test fun `explicit no-debug compilation removes all debug attributes`() {
        assertEquals(DebugAttributes(source = false, lines = false, variables = false), compileAttributes("-g:none"))
    }

    @Test fun `explicit full-debug compilation retains local variable names`() {
        assertEquals(DebugAttributes(source = true, lines = true, variables = true), compileAttributes("-g"))
    }

    private fun compileAttributes(vararg options: String): DebugAttributes {
        val source = root.resolve("Subject.java")
        source.writeText("""
            public class Subject {
                public static int add(int value) {
                    int result = value + 1;
                    return result;
                }
            }
        """.trimIndent())
        val output = compileJava(listOf(source), root.resolve("classes"), *options)
        val node = ClassNode().also { ClassReader(output.resolve("Subject.class").readBytes()).accept(it, 0) }
        val method = node.methods.single { it.name == "add" }
        return DebugAttributes(node.sourceFile != null,
            method.instructions.any { it is LineNumberNode }, method.localVariables.orEmpty().isNotEmpty())
    }

    private data class DebugAttributes(val source: Boolean, val lines: Boolean, val variables: Boolean)
}
