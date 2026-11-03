package j2me.cli

import j2me.process.RealProcessRunner
import j2me.testing.compileJava
import j2me.testing.jarDirectory
import org.jetbrains.java.decompiler.api.NamingPlan
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Tag
import org.junit.jupiter.api.io.TempDir
import java.net.URLClassLoader
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.io.path.writeText

/** Uses the actual toolkit producer and the runnable decompiler built in this checkout. */
@Tag("integration")
class ToolchainIntegrationTest {
    @TempDir lateinit var root: Path

    @Test fun `authored mappings survive the bundled JVM launcher and preserve behavior`() = roundTrip(false, true)
    @Test fun `authored mappings survive embedded decompilation and preserve behavior`() = roundTrip(true, true)
    @Test fun `semantic opt-out retains ordinary renames and behavior`() = roundTrip(false, false)

    private fun roundTrip(inProcess: Boolean, semantics: Boolean) {
        val source = root.resolve("a.java")
        source.writeText(resource("a.java"))
        val original = compileJava(listOf(source), root.resolve("original"), "-g:none")
        val jar = jarDirectory(root.resolve("input.jar"), original)
        root.resolve("mappings").createDirectories().resolve("Engine.map").writeText(resource("Engine.map"))
        root.resolve("j2me.toml").writeText("jar = \"input.jar\"\n")
        val paths = testToolkitPaths(root).copy(
            bundledDecompiler = Path.of(requireNotNull(System.getProperty("sporeflower.test.jar"))),
        )
        val args = buildRemapPipelineArgs(root, paths, null, jar, raw = false, noComments = false, semanticMappingsEnabled = semantics)
        val process = RealProcessRunner()
        val runner = if (inProcess) SporeflowerRunner(paths, process) else object : DecompilerRunner {
            override fun run(invocation: DecompilerInvocation): Long = runBundledDecompilerJvm(paths, process, invocation)
            override fun prepareNames(invocation: DecompilerInvocation): NamingPlan {
                val output = invocation.output.resolve("prepared.tiny")
                run(invocation.copy(options = invocation.options + mapOf(
                    "prepare-names-only" to "true", "naming-output" to output.toString(),
                )))
                return NamingPlan.read(output)
            }
        }
        val result = runRemapPipeline(args, runner, quiet = true)
        assertNotNull(result.mappings?.remappedJar?.path)
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
        val compile = compileStubs(
            root, paths, ProcessCompilerRunner(process),
            CompileStubsArgs(
                compiler = CompileBackend.JAVAC,
                // The fixture uses only Java SE; API selection is covered by the MIDP release round trip.
                apiJarsDirArg = root.resolve("empty-api").createDirectories().toString(),
                javacBin = Path.of(System.getProperty("java.home"), "bin", "javac").toString(),
            ),
            quiet = true,
        )
        compile.requireSuccess()
        assertEquals(StageStatus.SKIPPED, compile.restorationStatus)
        assertFalse(root.resolve("decompiled/.sporeflower.json").exists())
        assertFalse(root.resolve("out/compile_check/restored").exists())
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
