package j2me.cli

import j2me.bytecode.restoreCompiledClasses
import j2me.testing.compileJava
import j2me.testing.writeJar
import org.jetbrains.java.decompiler.api.NamingPlan
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.io.TempDir
import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.Arguments
import org.junit.jupiter.params.provider.MethodSource
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.Type
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.JarFile
import java.util.stream.Stream
import kotlin.io.path.*

class PortableClassRestorationTest {
    @TempDir lateinit var root: Path

    @ParameterizedTest
    @MethodSource("classNames")
    fun `restoration preserves compiled classes and original resource paths`(first: String, second: String, archive: Boolean) {
        val owners = listOf(first, second)
        val values = listOf(17, 29)
        val resources = linkedMapOf<String, ByteArray>()
        owners.forEachIndexed { index, owner ->
            resources.putIfAbsent(owner.substringBeforeLast('/') + "/item", byteArrayOf((37 + index * 16).toByte()))
        }
        val input = writeJar(root.resolve("input.jar"), owners.mapIndexed { index, owner -> "$owner.class" to classBytes(owner, values[index]) }.toMap() + resources)
        URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            owners.forEachIndexed { index, owner -> assertEquals(values[index], loader.loadClass(owner.replace('/', '.')).getMethod("value").invoke(null)) }
        }

        val sources = decompileForRestoration(root, input)
        val names = NamingPlan.read(sources.resolve(".sporeflower-names.tiny"))
        owners.forEachIndexed { index, owner ->
            val source = sources.resolve(names.classes().getValue(owner) + ".java")
            val text = source.readText()
            assertTrue(text.contains("return ${values[index]};"), text)
            assertTrue(text.contains("getResourceAsStream(\"item\")"), text)
            // Both restored results must come from the edited, compiled source;
            // retaining or loading either original method cannot pass this test.
            source.writeText(text.replace("return ${values[index]};", "return ${values[index] + 100};"))
        }
        val compiled = compileJava(sources, root.resolve("compiled"))
        val requested = root.resolve("restored/classes")
        val restored = restoreCompiledClasses(input, compiled, sources, requested)
        assertEquals(2, restored.classCount)
        assertEquals(if (archive) requested.resolveSibling("classes.jar") else requested, restored.artifact)
        if (archive) {
            assertTrue(restored.artifact.isRegularFile())
            assertFalse(requested.exists())
            JarFile(restored.artifact.toFile()).use { jar ->
                assertEquals(owners.map { "$it.class" }.toSet(), jar.entries().asSequence().map { it.name }.toSet())
            }
            val saved = restored.artifact.readBytes()
            assertThrows(IllegalArgumentException::class.java) { restoreCompiledClasses(input, compiled, sources, requested) }
            assertArrayEquals(saved, restored.artifact.readBytes())
        } else {
            assertTrue(restored.artifact.isDirectory())
            assertFalse(requested.resolveSibling("classes.jar").exists())
        }

        // Assets contain no original class bytes and keep their original JAR
        // names, including case-distinct package prefixes. The runtime loader
        // has no path from which it could fall back to the input program.
        val assets = writeJar(root.resolve("assets.jar"), resources)
        URLClassLoader(arrayOf(restored.artifact.toUri().toURL(), assets.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            owners.forEachIndexed { index, owner ->
                val type = loader.loadClass(owner.replace('/', '.'))
                assertEquals(values[index] + 100, type.getMethod("value").invoke(null))
                assertEquals(resources.getValue(owner.substringBeforeLast('/') + "/item")[0].toInt(), type.getMethod("resource").invoke(null))
            }
        }
        assertFalse(Files.list(requested.parent).use { entries -> entries.anyMatch { it.name.startsWith(".restored-") } })
    }

    private fun classBytes(name: String, value: Int): ByteArray = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
        visit(V1_8, ACC_PUBLIC or ACC_SUPER, name, null, "java/lang/Object", null)
        visitMethod(ACC_PUBLIC or ACC_STATIC, "value", "()I", null, null).apply {
            visitCode()
            visitLdcInsn(value)
            visitInsn(IRETURN)
            visitMaxs(0, 0)
            visitEnd()
        }
        visitMethod(ACC_PUBLIC or ACC_STATIC, "resource", "()I", null, arrayOf("java/io/IOException")).apply {
            visitCode()
            visitLdcInsn(Type.getObjectType(name))
            visitLdcInsn("item")
            visitMethodInsn(INVOKEVIRTUAL, "java/lang/Class", "getResourceAsStream", "(Ljava/lang/String;)Ljava/io/InputStream;", false)
            visitMethodInsn(INVOKEVIRTUAL, "java/io/InputStream", "read", "()I", false)
            visitInsn(IRETURN)
            visitMaxs(0, 0)
            visitEnd()
        }
        visitEnd()
    }.toByteArray()

    companion object {
        @JvmStatic
        fun classNames(): Stream<Arguments> = Stream.of(
            Arguments.of("p/Readable", "p/READABLE", true),
            Arguments.of("p/Games", "p/Game\u017f", true),
            Arguments.of("p/Sigma\u03c3", "p/Sigma\u03c2", true),
            Arguments.of("p/Sharp\u1e9e", "p/Sharp\u00df", true),
            Arguments.of("p/Caf\u00e9", "p/Cafe\u0301", true),
            Arguments.of("p/First", "P/Second", true),
            Arguments.of("p/First", "p/Second", false),
        )
    }
}
