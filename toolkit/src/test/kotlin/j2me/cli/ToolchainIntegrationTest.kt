package j2me.cli

import j2me.process.RealProcessRunner
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import javax.tools.ToolProvider
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.io.path.writeText

/** Uses the actual toolkit producer and the runnable decompiler built in this checkout. */
class ToolchainIntegrationTest {
    @TempDir lateinit var root: Path

    @Test fun `authored mappings survive process decompilation and preserve behavior`() = roundTrip(false, true)
    @Test fun `authored mappings survive in-process decompilation and preserve behavior`() = roundTrip(true, true)
    @Test fun `semantic opt-out retains ordinary renames and behavior`() = roundTrip(false, false)

    private fun roundTrip(inProcess: Boolean, semantics: Boolean) {
        val source = root.resolve("a.java")
        source.writeText(resource("a.java"))
        val original = root.resolve("original").createDirectories()
        val compiler = requireNotNull(ToolProvider.getSystemJavaCompiler())
        assertEquals(0, compiler.run(null, null, null, "--release", "8", "-g:none", "-d", original.toString(), source.toString()))
        val jar = root.resolve("input.jar")
        JarOutputStream(Files.newOutputStream(jar)).use { output ->
            output.putNextEntry(JarEntry("a.class"))
            output.write(Files.readAllBytes(original.resolve("a.class")))
            output.closeEntry()
        }
        root.resolve("mappings").createDirectories().resolve("Engine.map").writeText(resource("Engine.map"))
        root.resolve("j2me.toml").writeText("jar = \"input.jar\"\n")
        val engine = Path.of(requireNotNull(System.getProperty("sporeflower.test.jar")))
        val paths = ToolkitPaths(root, root.resolve("config/global.toml"), root.resolve("guide.md"), engine)
        val args = buildRemapPipelineArgs(root, paths, null, jar, raw = false, noComments = false, semanticMappingsEnabled = semantics)
        val process = RealProcessRunner()
        val runner = if (inProcess) InProcessVineflowerRunner() else ProcessVineflowerRunner(process)
        val result = runRemapPipeline(args, process, runner, quiet = true)
        assertNotNull(result.remappedJar)
        assertTrue(root.resolve("out/mapping.tiny").readText().contains("named/Engine"))
        val decompiled = root.resolve("decompiled/named/Engine.java").readText()
        assertTrue(decompiled.contains("score("), decompiled)
        assertTrue(decompiled.contains("absolute("), decompiled)
        if (semantics) {
            assertTrue(decompiled.contains("Direction.RIGHT"), decompiled)
            assertTrue(decompiled.contains("Position.SECOND"), decompiled)
            assertTrue(decompiled.contains("Buttons.FIRE") && decompiled.contains("Buttons.JUMP"), decompiled)
            assertTrue(root.resolve("decompiled/named/Direction.java").exists())
        } else {
            assertFalse(root.resolve("decompiled/named/Direction.java").exists())
        }
        compileStubs(
            root, paths, process,
            CompileStubsArgs(
                compiler = CompileBackend.JAVAC,
                javacBin = Path.of(System.getProperty("java.home"), "bin", "javac").toString(),
            ),
            quiet = true,
        )
        val rebuilt = root.resolve("out/compile_check/classes")
        URLClassLoader(arrayOf(original.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { before ->
            URLClassLoader(arrayOf(rebuilt.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { after ->
                val originalClass = before.loadClass("a")
                val rebuiltClass = after.loadClass("named.Engine")
                for (value in listOf(Int.MIN_VALUE, -7, -1, 0, 1, 2, 7, Int.MAX_VALUE)) {
                    for ((old, named) in listOf("b" to "absolute", "c" to "score", "d" to "filter")) {
                        val expected = originalClass.getMethod(old, Int::class.javaPrimitiveType).invoke(null, value)
                        val actual = rebuiltClass.getMethod(named, Int::class.javaPrimitiveType).invoke(null, value)
                        assertEquals(expected, actual, "$named($value)")
                    }
                }
                assertEquals(originalClass.getMethod("e").invoke(null), rebuiltClass.getMethod("demo").invoke(null))
            }
        }
    }

    private fun resource(name: String): String = requireNotNull(javaClass.getResource("/toolchain/$name")).readText()
}
