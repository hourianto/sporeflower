package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assumptions.assumeTrue
import java.util.zip.ZipFile
import java.util.zip.ZipInputStream
import java.io.ByteArrayInputStream
import java.io.InputStream
import org.junit.jupiter.api.io.TempDir
import org.junit.jupiter.api.io.CleanupMode
import org.jetbrains.java.decompiler.api.SemanticMappingData
import org.jetbrains.java.decompiler.api.J2meApi
import j2me.map.loadBuiltinSemanticMapSources
import org.objectweb.asm.ClassReader
import org.objectweb.asm.Opcodes
import org.objectweb.asm.tree.ClassNode
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import javax.tools.ToolProvider
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.io.path.writeText

class DistributionIntegrationTest {
    @TempDir(cleanup = CleanupMode.ON_SUCCESS) lateinit var temporary: Path

    @Test fun `relocated installation works without local vendor binaries`() = roundTrip(false)

    @Test fun `local legacy compiler works with relocated project paths`() {
        val installed = Path.of(requireNotNull(System.getProperty("j2me.test.installation")))
        assumeTrue(installed.resolve("vendor/compilers/legacy-javac/legacy-javac.jar").exists(),
            "Optional local compiler is not available")
        roundTrip(true)
    }

    @Test fun `release archive excludes local vendor binaries`() {
        val archive = Path.of(requireNotNull(System.getProperty("j2me.test.archive")))
        ZipFile(archive.toFile()).use { zip ->
            val entries = zip.entries().asSequence().toList()
            assertFalse(entries.any { "/vendor/" in it.name })
            assertFalse(entries.any { "/api/" in it.name })
        }
    }

    @Test fun `every shipped runtime jar including nested plugins targets Java 8`() {
        val archive = Path.of(requireNotNull(System.getProperty("j2me.test.archive")))
        Files.newInputStream(archive).use { assertJava8Classes(it, archive.fileName.toString()) }
    }

    @Test fun `packaged records retain JSON validation and immutability`() {
        val engine = Path.of(requireNotNull(System.getProperty("sporeflower.test.jar")))
        val source = temporary.resolve("RuntimeProbe.java")
        source.writeText(requireNotNull(javaClass.getResource("/toolchain/RuntimeProbe.java")).readText())
        val classes = temporary.resolve("probe-classes").createDirectories()
        assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null,
            "--release", "8", "-classpath", engine.toString(), "-d", classes.toString(), source.toString()))
        val javaHome = System.getProperty("j2me.test.java.home", System.getProperty("java.home"))
        val output = command(temporary, Path.of(javaHome, "bin/java").toString(), "-cp",
            engine.toString() + java.io.File.pathSeparator + classes, "RuntimeProbe", temporary.toString())
        assertTrue(output.contains("Runtime compatibility passed"), output)
    }

    @Test fun `Unicode survives decompilation and recompilation with a non UTF-8 default`() {
        val text = "Привет, 世界 — café"
        val source = temporary.resolve("Greeting.java")
        source.writeText("public class Greeting { public static final String TEXT = \"$text\"; }")
        val classes = temporary.resolve("original").createDirectories()
        assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null,
            "--release", "8", "-encoding", "UTF-8", "-d", classes.toString(), source.toString()))
        val input = temporary.resolve("input.jar")
        JarOutputStream(Files.newOutputStream(input)).use { jar ->
            jar.putNextEntry(JarEntry("Greeting.class"))
            jar.write(Files.readAllBytes(classes.resolve("Greeting.class")))
            jar.closeEntry()
        }

        val javaHome = System.getProperty("j2me.test.java.home", System.getProperty("java.home"))
        val engine = requireNotNull(System.getProperty("sporeflower.test.jar"))
        val standalone = temporary.resolve("standalone").createDirectories()
        command(temporary, Path.of(javaHome, "bin/java").toString(), "-jar", engine,
            input.toString(), standalone.toString(), fileEncoding = "windows-1252")
        assertTrue(standalone.resolve("Greeting.java").readText().contains(text))

        val launcher = Path.of(requireNotNull(System.getProperty("j2me.test.installation")), "bin/j2me")
        val project = temporary.resolve("unicode project")
        command(temporary, "sh", launcher.toString(), "init", "--project", project.toString(),
            "--jar", input.toString(), fileEncoding = "windows-1252")
        assertTrue(project.resolve("decompiled/defpackage/Greeting.java").readText().contains(text))
        val stubs = temporary.resolve("stubs").createDirectories()
        stubs.resolve("GreetingStub.java").writeText(
            "public class GreetingStub { public static final String TEXT = \"$text\"; }")
        command(temporary, "sh", launcher.toString(), "compile-stubs", "--project", project.toString(),
            "--compiler", "javac", "--stubs-src", stubs.toString(), "--no-stub-cache", fileEncoding = "windows-1252")

        // A compiler can silently accept mojibake, so check the class constants.
        for (relative in listOf("classes/defpackage/Greeting.class", "stubs-classes/GreetingStub.class")) {
            val node = ClassNode()
            ClassReader(Files.readAllBytes(project.resolve("out/compile_check/$relative"))).accept(node, 0)
            assertEquals(text, node.fields.single { it.name == "TEXT" }.value, relative)
        }
    }

    private fun assertJava8Classes(input: InputStream, path: String) {
        ZipInputStream(input).use { zip ->
            while (true) {
                val entry = zip.nextEntry ?: break
                val name = entry.name
                if (name.endsWith(".jar")) {
                    assertJava8Classes(ByteArrayInputStream(zip.readBytes()), "$path!/$name")
                } else if (name.endsWith(".class") && name != "module-info.class" && !name.startsWith("META-INF/versions/")) {
                    val header = zip.readNBytes(8)
                    assertEquals(8, header.size, "$path!/$name")
                    val version = ((header[6].toInt() and 255) shl 8) or (header[7].toInt() and 255)
                    assertTrue(version <= 52, "$path!/$name requires class version $version")
                }
            }
        }
    }

    @Test fun `release alone decompiles MIDP and M3G with semantic constants and recompiles`() {
        val home = temporary.resolve("extracted release").createDirectories()
        val archive = Path.of(requireNotNull(System.getProperty("j2me.test.archive")))
        ZipFile(archive.toFile()).use { zip ->
            for (entry in zip.entries()) {
                val relative = entry.name.substringAfter('/')
                if (relative.isEmpty()) continue
                val target = home.resolve(relative)
                if (entry.isDirectory) target.createDirectories() else {
                    target.parent.createDirectories()
                    zip.getInputStream(entry).use { Files.copy(it, target) }
                }
            }
        }
        assertFalse(home.resolve("vendor").exists())
        assertFalse(home.resolve("api").exists())
        val emptyInput = temporary.resolve("empty.jar")
        JarOutputStream(Files.newOutputStream(emptyInput)).use { }
        val api = J2meApi.resolve(emptyInput, emptyList(), true)
        assertEquals(36, loadBuiltinSemanticMapSources(api.classes().keys).size)
        assertStubBodies(home.resolve("decompiler/sporeflower.jar"))

        val source = temporary.resolve("Screen.java")
        source.writeText("""
            import javax.microedition.lcdui.Canvas;
            import javax.microedition.lcdui.Graphics;
            import javax.microedition.m3g.Light;
            public class Screen extends Canvas {
                protected void paint(Graphics graphics) {
                    graphics.drawString("Sample", 0, 0, Graphics.TOP | Graphics.LEFT);
                }
                public void configure(Light light) { light.setMode(Light.DIRECTIONAL); }
            }
        """.trimIndent())
        val classes = temporary.resolve("original").createDirectories()
        val compileClasspath = writeApiSnapshot(api, temporary.resolve("fixture-compile-api")).toString()
        assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null,
            "--release", "8", "-g:none", "-classpath", compileClasspath, "-d", classes.toString(), source.toString()))
        val input = temporary.resolve("input.jar")
        JarOutputStream(Files.newOutputStream(input)).use { jar ->
            jar.putNextEntry(JarEntry("Screen.class"))
            jar.write(Files.readAllBytes(classes.resolve("Screen.class")))
            jar.closeEntry()
        }
        val project = temporary.resolve("midp project")
        val launcher = home.resolve("bin/j2me")
        command(temporary, "sh", launcher.toString(), "init", "--project", project.toString(), "--jar", input.toString())
        val output = Files.walk(project.resolve("decompiled")).use { files ->
            files.filter { it.toString().endsWith(".java") }.map { it.readText() }.toList().joinToString("\n")
        }
        assertTrue(output.contains("Graphics.TOP") && output.contains("Graphics.LEFT"), output)
        assertTrue(output.contains("Light.DIRECTIONAL"), output)
        assertFalse(home.resolve(".cache/api").exists(), "Mapping/decompilation must not extract API jars")
        command(temporary, "sh", launcher.toString(), "compile-stubs", "--project", project.toString(), "--compiler", "javac")
        assertTrue(project.resolve("out/compile_check/summary.txt").readText().contains("compile_exit=0"))
    }

    private fun assertStubBodies(engine: Path) {
        var classCount = 0
        ZipFile(engine.toFile()).use { zip ->
            assertNotNull(zip.getEntry("META-INF/j2me-api/index.tsv"))
            assertNull(zip.getEntry("java/lang/Object.class"))
            for (entry in zip.entries()) {
                if (!entry.name.startsWith("META-INF/j2me-api/") || !entry.name.endsWith(".class")) continue
                val node = ClassNode()
                zip.getInputStream(entry).use { ClassReader(it).accept(node, 0) }
                assertNull(node.sourceFile)
                for (method in node.methods) {
                    assertNotEquals("<clinit>", method.name)
                    val instructions = method.instructions.toArray().map { it.opcode }
                    val expected = if (method.access and (Opcodes.ACC_ABSTRACT or Opcodes.ACC_NATIVE) != 0) emptyList()
                        else listOf(Opcodes.ACONST_NULL, Opcodes.ATHROW)
                    assertEquals(expected, instructions, "${entry.name}:${method.name}${method.desc}")
                }
                classCount++
            }
        }
        assertTrue(classCount > 3000)
    }

    private fun roundTrip(legacy: Boolean) {
        val installed = Path.of(requireNotNull(System.getProperty("j2me.test.installation")))
        val relocated = temporary.resolve("toolkit with spaces").createDirectories()
        Files.walk(installed).use { files ->
            files.forEach { source ->
                val relative = installed.relativize(source)
                if (!legacy && relative.firstOrNull()?.toString() == "vendor") return@forEach
                val target = relocated.resolve(relative)
                if (Files.isDirectory(source)) target.createDirectories() else Files.copy(source, target)
            }
        }
        // Invoke via sh so this test does not depend on the copy's executable bit.
        val launcher = relocated.resolve("bin/j2me")
        val cwd = temporary.resolve("unrelated working directory").createDirectories()
        val doctor = command(cwd, "sh", launcher.toString(), "doctor")
        assertTrue(doctor.contains(relocated.resolve("decompiler/sporeflower.jar").toString()), doctor)
        assertFalse(doctor.contains("(missing)"), doctor)
        assertTrue(doctor.contains("decompiler: Sporeflower (enabled)"), doctor)

        val source = temporary.resolve("a.java")
        source.writeText(requireNotNull(javaClass.getResource("/toolchain/a.java")).readText())
        val classes = temporary.resolve("original classes").createDirectories()
        assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null,
            "--release", "8", "-g:none", "-d", classes.toString(), source.toString()))
        val jar = temporary.resolve("input.jar")
        JarOutputStream(Files.newOutputStream(jar)).use { output ->
            output.putNextEntry(JarEntry("a.class"))
            output.write(Files.readAllBytes(classes.resolve("a.class")))
            output.closeEntry()
        }
        val project = temporary.resolve("sample project")
        command(cwd, "sh", launcher.toString(), "init", "--project", project.toString(), "--jar", jar.toString())
        assertTrue(Files.isSameFile(relocated.resolve("templates/mappings-doc.md"), project.resolve("AGENTS.md")))
        project.resolve("mappings/Engine.map").writeText(requireNotNull(javaClass.getResource("/toolchain/Engine.map")).readText())
        val remap = command(cwd, "sh", launcher.toString(), "remap", "--project", project.toString(), "--export-semantic-map")
        assertTrue(Regex("Timing: total=\\d+ms, decompiler=\\d+ms").containsMatchIn(remap), remap)
        assertTrue(project.resolve("out/decompiler.stdout.log").exists())
        assertTrue(project.resolve("out/decompiler.stderr.log").exists())
        val export = project.resolve("out/semantic-map.json")
        val semantics = SemanticMappingData.read(export)
        assertTrue(semantics.domains().any { it.id() == "named/Direction" })
        assertTrue(semantics.scalarBindings().any { it.target().name() == "<init>" })
        for (disabled in listOf("--raw", "--no-semantic-mappings")) {
            val error = command(cwd, "sh", launcher.toString(), "remap", "--project", project.toString(),
                "--export-semantic-map", disabled, expectedExit = 1)
            assertTrue(error.contains("--export-semantic-map requires semantic mappings"), error)
            assertEquals(semantics, SemanticMappingData.read(export))
        }
        assertTrue(project.resolve("decompiled/named/Engine.java").readText().contains("Direction.RIGHT"))
        val namedSource = project.resolve("decompiled/named/Engine.java").readText()
        assertTrue(namedSource.contains("Engine(int direction)"), namedSource)
        assertTrue(namedSource.contains("this(Direction.RIGHT)"), namedSource)
        assertTrue(namedSource.contains("== Position.SECOND"), namedSource)
        assertTrue(namedSource.contains("== 1"), namedSource)
        assertTrue(namedSource.contains("{Direction.RIGHT, Buttons.JUMP, Direction.CENTER, Buttons.FIRE}"), namedSource)

        val backend = if (legacy) emptyArray() else arrayOf("--compiler", "javac")
        command(cwd, "sh", launcher.toString(), "compile-stubs", "--project", project.toString(), *backend)
        assertTrue(project.resolve("out/compile_check/classes/named/Engine.class").exists())
        assertTrue(project.resolve("out/compile_check/summary.txt").readText().contains("compile_exit=0"))

        if (legacy) {
            val embedded = InProcessCompilerRunner { error("Bundled legacy compiler unexpectedly required a subprocess") }
            val result = compileStubs(project, toolkitPaths(relocated, null, null), embedded, CompileStubsArgs(), quiet = true)
            assertNull(result.failureMessage, project.resolve("out/compile_check/compiler.stderr.log").readText())
            assertEquals(0, result.diagnostics.errors.size)
        }

        if (!legacy) {
            val disabledProject = temporary.resolve("disabled project").createDirectories()
            disabledProject.resolve("j2me.toml").writeText("jar = \"missing.jar\"\n\n[fullrun]\nenabled = false\n")
            val logs = temporary.resolve("mapped fullrun")
            val report = temporary.resolve("mapped-report.md")
            command(cwd, "sh", launcher.toString(), "fullrun", "--mapped", "--root", temporary.toString(),
                "--logs", logs.toString(), "--report", report.toString(),
                "--history-mode", "off", "--keep-work", "all", "--no-compile")
            assertTrue(report.readText().contains("Decompiler: `Sporeflower`"))
            assertTrue(report.readText().contains("Mapping mode: `mapped`"))
            assertTrue(logs.resolve("sample_project.log").readText().contains("decompiler_ms="))
            assertTrue(report.readText().contains("Projects: 1"))
            assertFalse(report.readText().contains("disabled project"))
            Files.walk(logs.resolve("work")).use { files ->
                val engine = files.filter { it.fileName.toString() == "Engine.java" }.findFirst().orElseThrow()
                assertTrue(engine.readText().contains("Direction.RIGHT"))
            }
            val explicit = command(cwd, "sh", launcher.toString(), "fullrun", "--root", temporary.toString(),
                "--project", disabledProject.toString(), "--history-mode", "off", "--no-compile", expectedExit = 1)
            assertTrue(explicit.contains("[disabled project] remap=FAIL"), explicit)
        }

        val override = temporary.resolve("override.toml")
        override.writeText("[decompiler]\nenabled = false\n")
        val configured = command(cwd, "sh", launcher.toString(), "doctor", config = override)
        assertTrue(configured.contains("global config: $override"), configured)
        assertTrue(configured.contains("decompiler: Sporeflower (disabled)"), configured)
        command(cwd, "sh", launcher.toString(), "remap", "--project", project.toString(), "--export-semantic-map", config = override)
        assertEquals(semantics, SemanticMappingData.read(export))
        assertFalse(project.resolve("out/decompiler.stdout.log").exists())
    }

    private fun command(cwd: Path, vararg args: String, config: Path? = null, expectedExit: Int = 0,
                        fileEncoding: String? = null): String {
        val log = Files.createTempFile(temporary, "command-", ".log")
        val process = ProcessBuilder(*args).directory(cwd.toFile()).redirectErrorStream(true).redirectOutput(log.toFile()).apply {
            environment().remove("J2ME_BASE")
            environment().remove("J2ME_CONFIG")
            environment().remove("JAVA_OPTS")
            environment().remove("J2ME_OPTS")
            val javaHome = System.getProperty("j2me.test.java.home", System.getProperty("java.home"))
            environment()["JAVA_HOME"] = javaHome
            environment()["PATH"] = Path.of(javaHome, "bin").toString() + java.io.File.pathSeparator + environment()["PATH"]
            if (fileEncoding != null) environment()["JAVA_TOOL_OPTIONS"] = "-Dfile.encoding=$fileEncoding"
            if (config != null) environment()["J2ME_CONFIG"] = config.toString()
        }.start()
        if (!process.waitFor(60, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            fail<Unit>("Timed out: ${args.joinToString(" ")}\n${log.readText()}")
        }
        val output = log.readText()
        assertEquals(expectedExit, process.exitValue(), output)
        return output
    }
}
