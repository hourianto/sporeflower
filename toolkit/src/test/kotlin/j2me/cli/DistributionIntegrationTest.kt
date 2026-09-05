package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assumptions.assumeTrue
import java.util.zip.ZipFile
import org.junit.jupiter.api.io.TempDir
import org.junit.jupiter.api.io.CleanupMode
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
        }
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
        command(cwd, "sh", launcher.toString(), "remap", "--project", project.toString())
        assertTrue(project.resolve("decompiled/named/Engine.java").readText().contains("Direction.RIGHT"))

        val backend = if (legacy) emptyArray() else arrayOf("--compiler", "javac")
        command(cwd, "sh", launcher.toString(), "compile-stubs", "--project", project.toString(), *backend)
        assertTrue(project.resolve("out/compile_check/classes/named/Engine.class").exists())
        assertTrue(project.resolve("out/compile_check/summary.txt").readText().contains("compile_exit=0"))

        val override = temporary.resolve("override.toml")
        override.writeText("[vineflower]\nenabled = false\n")
        val configured = command(cwd, "sh", launcher.toString(), "doctor", config = override)
        assertTrue(configured.contains("global config: $override"), configured)
    }

    private fun command(cwd: Path, vararg args: String, config: Path? = null): String {
        val log = Files.createTempFile(temporary, "command-", ".log")
        val process = ProcessBuilder(*args).directory(cwd.toFile()).redirectErrorStream(true).redirectOutput(log.toFile()).apply {
            environment().remove("J2ME_BASE")
            environment().remove("J2ME_CONFIG")
            environment().remove("SPOREFLOWER_JAR")
            environment().remove("JAVA_OPTS")
            environment().remove("J2ME_OPTS")
            environment()["JAVA_HOME"] = System.getProperty("java.home")
            if (config != null) environment()["J2ME_CONFIG"] = config.toString()
        }.start()
        if (!process.waitFor(60, TimeUnit.SECONDS)) {
            process.destroyForcibly()
            fail<Unit>("Timed out: ${args.joinToString(" ")}\n${log.readText()}")
        }
        val output = log.readText()
        assertEquals(0, process.exitValue(), output)
        return output
    }
}
