package j2me.validation

import j2me.model.CanonicalMap
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.nio.file.Path
import javax.tools.ToolProvider
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

class RealizedNamesTest {
    @TempDir lateinit var root: Path

    @Test fun `package-only split cannot break an actual package access`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { static int value; }",
            "p/Friend" to "package p; public class Friend { public static int read() { return Base.value; } }",
        ))
        val names = CanonicalMap(classes = mapOf("p/Friend" to "q/Friend"))
        val error = assertThrows(MappingValidationException::class.java) { validateRealizedNames(classes, names, emptyMap()) }
        assertTrue(error.message!!.contains("loses package access"), error.message)
    }

    @Test fun `safe public split remains allowed`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { public static int value; }",
            "p/Friend" to "package p; public class Friend { public static int read() { return Base.value; } }",
        ))
        validateRealizedNames(classes, CanonicalMap(classes = mapOf("p/Friend" to "q/Friend")), emptyMap())
    }

    @Test fun `package merge needs a conflict repair to preserve independent methods`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { int value() { return 1; } public int baseCall() { return value(); } }",
            "q/Child" to "package q; public class Child extends p.Base { int value() { return 2; } }",
        ))
        val names = CanonicalMap(classes = mapOf("q/Child" to "p/Child"))
        assertThrows(MappingValidationException::class.java) { validateRealizedNames(classes, names, emptyMap()) }
        validateRealizedNames(classes, names.copy(methods = mapOf(MethodSig("q/Child", "value", "()I") to "childValue")), emptyMap())
    }

    @Test fun `renamed field must still resolve to the same inherited declaration`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { public static int value; }",
            "p/Child" to "package p; public class Child extends Base { public static int another; }",
            "p/Use" to "package p; public class Use { public static int read() { return Child.value; } }",
        ))
        val names = CanonicalMap(fields = mapOf(FieldSig("p/Child", "another", "I") to "value"))
        val error = assertThrows(MappingValidationException::class.java) { validateRealizedNames(classes, names, emptyMap()) }
        assertTrue(error.message!!.contains("would bind"), error.message)
    }

    private fun compile(sources: Map<String, String>): Path {
        val files = sources.map { (name, text) -> root.resolve("src/$name.java").also { it.parent.createDirectories(); it.writeText(text) } }
        val output = root.resolve("classes").createDirectories()
        assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null,
            *(listOf("--release", "8", "-d", output.toString()) + files.map(Path::toString)).toTypedArray()))
        return output
    }

    @Test fun `a public intermediate override preserves dispatch across a package split`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { int value() { return 1; } }",
            "p/Middle" to "package p; public class Middle extends Base { public int value() { return 2; } }",
            "p/Child" to "package p; public class Child extends Middle { public int value() { return 3; } }",
        ))
        validateRealizedNames(classes, CanonicalMap(classes = mapOf("p/Child" to "q/Child")), emptyMap())
    }

    @Test fun `an inherited implementation keeps the interface name introduced by a child`() {
        val classes = compile(mapOf(
            "p/Base" to "package p; public class Base { public int value() { return 1; } }",
            "p/Contract" to "package p; public interface Contract { int value(); }",
            "p/Child" to "package p; public class Child extends Base implements Contract {}",
        ))
        val names = CanonicalMap(methods = mapOf(MethodSig("p/Base", "value", "()I") to "renamed"))
        assertThrows(MappingValidationException::class.java) { validateRealizedNames(classes, names, emptyMap()) }
    }
}
