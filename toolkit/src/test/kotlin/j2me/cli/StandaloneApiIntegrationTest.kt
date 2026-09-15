package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Attribute
import org.objectweb.asm.ByteVector
import org.objectweb.asm.Opcodes.*
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText

class StandaloneApiIntegrationTest {
    @TempDir lateinit var temporary: Path

    @Test fun `standalone jar resolves J2ME APIs without support files or a writable temp directory`() {
        val home = installation("standalone")
        input(home)
        val log = run(home)
        assertTrue(log.contains("Loaded bundled J2ME API declarations"), log)
        val source = home.resolve("out/Screen.java").readText()
        // The JVM permits a concrete class without this inherited abstract
        // method; Java source needs the engine's existing compatibility stub.
        assertTrue(source.contains("void paint(Graphics"), source)
        assertEquals(setOf("sporeflower.jar", "input.jar", "out"), Files.list(home).use { files ->
            files.map { it.fileName.toString() }.toList().toSet()
        })
        assertFalse(temporary.resolve("unavailable-temp").exists())
        assertFalse(temporary.resolve("unavailable-home").exists())
    }

    @Test fun `standalone accepts explicit SDK overrides and can disable embedded declarations`() {
        val home = installation("override")
        input(home)
        val canvas = ClassWriter(0)
        canvas.visit(V1_1, ACC_PUBLIC or ACC_ABSTRACT, "javax/microedition/lcdui/Canvas", null, "java/lang/Object", null)
        constructor(canvas, "java/lang/Object", ACC_PROTECTED)
        canvas.visitMethod(ACC_PROTECTED or ACC_ABSTRACT, "customPaint", "()V", null, null).visitEnd()
        canvas.visitEnd()
        jar(home.resolve("sdk.jar"), "javax/microedition/lcdui/Canvas", canvas.toByteArray())
        val log = run(home, "--add-external=sdk.jar", "--rename-members=true")
        assertTrue(log.contains("Loaded bundled J2ME API declarations"), log)
        val output = Files.walk(home.resolve("out")).use { files ->
            files.filter { it.toString().endsWith(".java") }.map { it.readText() }.toList().joinToString("\n")
        }
        assertTrue(output.contains("void customPaint()"), output)
        assertFalse(output.contains("void paint(Graphics"), output)

        val disabled = installation("disabled")
        input(disabled)
        val disabledLog = run(disabled, "--bundled-j2me-api=false")
        assertFalse(disabledLog.contains("Loaded bundled J2ME API declarations"), disabledLog)
        assertFalse(disabled.resolve("out/Screen.java").readText().contains("void paint(Graphics"))
    }

    @Test fun `plain Java classes do not receive a CLDC core`() {
        val home = installation("plain-java")
        val writer = ClassWriter(0)
        writer.visit(V1_8, ACC_PUBLIC, "Plain", null, "java/lang/Object", null)
        constructor(writer, "java/lang/Object", ACC_PUBLIC)
        writer.visitEnd()
        jar(home.resolve("input.jar"), "Plain", writer.toByteArray())
        val log = run(home)
        assertFalse(log.contains("Loaded bundled J2ME API declarations"), log)
        assertTrue(home.resolve("out/Plain.java").exists())
    }

    @Test fun `API discovery preserves malformed-attribute recovery for J2ME inputs`() {
        val home = installation("malformed-j2me")
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Screen", null, "javax/microedition/lcdui/Canvas", null)
        constructor(writer, "javax/microedition/lcdui/Canvas", ACC_PUBLIC)
        writer.visitMethod(ACC_PUBLIC or ACC_STATIC, "broken", "()V", null, null).apply {
            visitAttribute(object : Attribute("Exceptions") {
                override fun write(writer: ClassWriter, code: ByteArray?, length: Int, stack: Int, locals: Int): ByteVector =
                    ByteVector().putShort(1).putShort(65535)
            })
            visitCode(); visitInsn(RETURN); visitMaxs(0, 0); visitEnd()
        }
        writer.visitEnd()
        jar(home.resolve("input.jar"), "Screen", writer.toByteArray())
        val log = run(home)
        assertTrue(log.contains("Loaded bundled J2ME API declarations"), log)
        assertTrue(home.resolve("out/Screen.java").readText().contains("void paint(Graphics"))
    }

    @Test fun `descriptor-only API references activate the embedded library`() {
        val home = installation("descriptor-only")
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Screen", null, "java/lang/Object", null)
        constructor(writer, "java/lang/Object", ACC_PUBLIC)
        writer.visitField(ACC_PUBLIC, "graphics", "Ljavax/microedition/lcdui/Graphics;", null, null).visitEnd()
        writer.visitEnd()
        jar(home.resolve("input.jar"), "Screen", writer.toByteArray())
        val log = run(home)
        assertTrue(log.contains("Loaded bundled J2ME API declarations"), log)
    }

    private fun installation(name: String): Path = temporary.resolve(name).createDirectories().also {
        Files.copy(Path.of(requireNotNull(System.getProperty("sporeflower.test.jar"))), it.resolve("sporeflower.jar"))
    }

    private fun input(home: Path) {
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Screen", null, "javax/microedition/lcdui/Canvas", null)
        constructor(writer, "javax/microedition/lcdui/Canvas", ACC_PUBLIC)
        writer.visitEnd()
        jar(home.resolve("input.jar"), "Screen", writer.toByteArray())
    }

    private fun constructor(writer: ClassWriter, parent: String, access: Int) {
        writer.visitMethod(access, "<init>", "()V", null, null).apply {
            visitCode()
            visitVarInsn(ALOAD, 0)
            visitMethodInsn(INVOKESPECIAL, parent, "<init>", "()V", false)
            visitInsn(RETURN)
            visitMaxs(1, 1)
            visitEnd()
        }
    }

    private fun jar(path: Path, owner: String, bytes: ByteArray) {
        JarOutputStream(Files.newOutputStream(path)).use {
            it.putNextEntry(JarEntry("$owner.class")); it.write(bytes); it.closeEntry()
        }
    }

    private fun run(home: Path, vararg options: String): String {
        val java = Path.of(System.getProperty("java.home"), "bin/java").toString()
        val process = ProcessBuilder(listOf(java,
            "-Djava.io.tmpdir=${temporary.resolve("unavailable-temp")}",
            "-Duser.home=${temporary.resolve("unavailable-home")}",
            "-jar", "sporeflower.jar") + options + listOf("input.jar", "out"))
            .directory(home.toFile()).redirectErrorStream(true).start()
        val output = process.inputStream.bufferedReader().readText()
        assertTrue(process.waitFor(60, TimeUnit.SECONDS), "Standalone decompiler timed out")
        assertEquals(0, process.exitValue(), output)
        return output
    }
}
