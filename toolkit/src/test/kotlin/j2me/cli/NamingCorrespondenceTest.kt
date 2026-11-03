package j2me.cli

import j2me.bytecode.restoreCompiledClasses
import j2me.process.RealProcessRunner
import j2me.testing.classJar
import j2me.testing.compileJava
import j2me.testing.copyDirectory
import org.jetbrains.java.decompiler.api.SourceMetadata
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.BeforeAll
import org.junit.jupiter.api.Tag
import org.junit.jupiter.api.TestInstance
import org.junit.jupiter.api.io.TempDir
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.commons.ClassRemapper
import org.objectweb.asm.commons.Remapper
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.*

@TestInstance(TestInstance.Lifecycle.PER_CLASS)
class NamingCorrespondenceTest {
    @TempDir lateinit var root: Path
    private lateinit var fixtureRoot: Path
    private val nested by lazy {
        val root = fixtureRoot.resolve("nested").createDirectories()
        val jar = nestedInput(root)
        val sources = decompile(jar, root = root)
        val compiled = compile(sources, root)
        val restored = restoreCompiledClasses(jar, compiled, sources, root.resolve("restored")).artifact
        NestedFixture(jar, sources, compiled, restored)
    }

    @BeforeAll fun prepareFixtureDirectory(@TempDir directory: Path) {
        fixtureRoot = directory
    }

    private data class NestedFixture(val jar: Path, val sources: Path, val compiled: Path, val restored: Path)

    @Test fun `nested members follow a renumbered anonymous parent and locals keep lexical scope`() {
        for (input in listOf(nested.jar, nested.restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val subject = loader.loadClass("p.Subject")
                assertEquals("11", subject.getMethod("pick", Boolean::class.javaPrimitiveType).invoke(null, true).toString())
                assertEquals("second", subject.getMethod("pick", Boolean::class.javaPrimitiveType).invoke(null, false).toString())
                assertEquals("left", subject.getMethod("left").invoke(null).toString())
                assertEquals("right", subject.getMethod("right").invoke(null).toString())
                assertNotNull(loader.loadClass("p.Subject\$9\$Member"))
            }
        }
    }

    @Test fun `anonymous matching reports missing line tables rather than guessing`() {
        val compiled = copyDirectory(nested.compiled, root.resolve("compiled"))
        Files.walk(compiled).use { files -> files.filter { it.extension == "class" }.forEach { file ->
            val writer = ClassWriter(0)
            ClassReader(file.readBytes()).accept(writer, ClassReader.SKIP_DEBUG)
            file.writeBytes(writer.toByteArray())
        } }
        val error = assertThrows(IllegalArgumentException::class.java) {
            restoreCompiledClasses(nested.jar, compiled, nested.sources, root.resolve("without-debug"))
        }
        assertTrue(error.message.orEmpty().contains("Missing line-number data"), error.message)
        assertTrue(error.message.orEmpty().contains("-g:lines,source"), error.message)
    }

    @Test fun `keyword class retains package access in source inspection and restoration`() {
        val stable = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_8, ACC_PUBLIC, "Stable", null, "java/lang/Object", null)
            visitField(ACC_STATIC, "value", "I", null, null).visitEnd()
            visitMethod(ACC_STATIC, "<clinit>", "()V", null, null).apply {
                visitCode(); visitIntInsn(BIPUSH, 37); visitFieldInsn(PUTSTATIC, "Stable", "value", "I")
                visitInsn(RETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val keyword = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_8, ACC_PUBLIC, "do", null, "java/lang/Object", null)
            visitMethod(ACC_PUBLIC or ACC_STATIC, "read", "()I", null, null).apply {
                visitCode(); visitFieldInsn(GETSTATIC, "Stable", "value", "I")
                visitInsn(IRETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val consumer = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_8, ACC_PUBLIC, "p/Consumer", null, "java/lang/Object", null)
            visitMethod(ACC_PUBLIC or ACC_STATIC, "run", "()I", null, null).apply {
                visitCode(); visitMethodInsn(INVOKESTATIC, "do", "read", "()I", false)
                visitInsn(IRETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val jar = writeJar(mapOf("Stable" to stable.toByteArray(), "do" to keyword.toByteArray(), "p/Consumer" to consumer.toByteArray()))
        val sources = decompile(jar)
        val names = org.jetbrains.java.decompiler.api.NamingPlan.read(sources.resolve(".sporeflower-names.tiny"))
        assertTrue(names.classes().getValue("do").startsWith("defpackage/"))
        assertEquals("defpackage/Stable", names.classes().getValue("Stable"))
        val restored = restoreCompiledClasses(jar, compile(sources), sources, root.resolve("restored")).artifact
        val inspection = root.resolve("out/input_remapped.jar")
        for (input in listOf(jar, inspection, restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                assertEquals(37, loader.loadClass("p.Consumer").getMethod("run").invoke(null))
            }
        }
    }

    @Test fun `empty and identity mappings use the same names for reflection and class files`() {
        val source = root.resolve("source").createDirectories().resolve("Subject.java")
        source.writeText("""
            public class Subject {
                public static Class lookup() throws Exception { return Class.forName("Subject"); }
            }
        """.trimIndent())
        val original = compile(source.parent)
        val jar = writeJar(mapOf("Subject" to original.resolve("Subject.class").readBytes()))
        val symbols = mapOf("Subject" to j2me.symbols.parseClassSymbols(jar, "Subject"))
        for (names in listOf(emptyMap(), mapOf("Subject" to "Subject"), mapOf("Subject" to "Readable"))) {
            val output = root.resolve("remapped-${names.size}-${names["Subject"]}.jar")
            j2me.bytecode.remapJarBytecode(jar, output, j2me.model.CanonicalMap(classes = names), symbols)
            URLClassLoader(arrayOf(output.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val expected = names["Subject"] ?: "Subject"
                assertEquals(expected, (loader.loadClass(expected).getMethod("lookup").invoke(null) as Class<*>).name)
            }
        }
    }

    @Test fun `illegal package repair preserves package access and restored relative resources`() {
        val source = root.resolve("source/clean").createDirectories().resolve("Subject.java")
        source.writeText("""
            package clean;
            public class Subject {
                public static int run() { return Helper.value; }
                public int resource(String path) throws Exception {
                    java.io.InputStream stream = getClass().getResourceAsStream(path);
                    if (stream == null) return -1;
                    try { return stream.read(); } finally { stream.close(); }
                }
            }
            class Helper { static int value = 37; }
        """.trimIndent())
        val original = compile(root.resolve("source"))
        val remapper = object : Remapper(ASM9) {
            override fun map(name: String): String = if (name.startsWith("clean/")) "for/" + name.removePrefix("clean/") else name
        }
        val jar = writeJar(listOf("Subject", "Helper").associate { simple ->
            val writer = ClassWriter(0)
            ClassReader(original.resolve("clean/$simple.class").readBytes()).accept(ClassRemapper(writer, remapper), 0)
            "for/$simple" to writer.toByteArray()
        })
        val sources = decompile(jar)
        val names = org.jetbrains.java.decompiler.api.NamingPlan.read(sources.resolve(".sporeflower-names.tiny"))
        val subject = names.classes().getValue("for/Subject")
        assertFalse(subject.startsWith("for/"))
        assertEquals(subject.substringBeforeLast('/'), names.classes().getValue("for/Helper").substringBeforeLast('/'))
        val compiled = compile(sources)
        URLClassLoader(arrayOf(compiled.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            assertEquals(37, loader.loadClass(subject.replace('/', '.')).getMethod("run").invoke(null))
        }
        val restored = restoreCompiledClasses(jar, compiled, sources, root.resolve("restored")).artifact
        val resources = root.resolve("resources").createDirectories()
        resources.resolve("for").createDirectories().resolve("item").writeBytes(byteArrayOf(29))
        // Keep resources separate so a missing restored class cannot load from the original JAR.
        for (input in listOf(jar, restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL(), resources.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val type = loader.loadClass("for.Subject")
                assertEquals(37, type.getMethod("run").invoke(null))
                assertEquals(29, type.getMethod("resource", String::class.java).invoke(type.getConstructor().newInstance(), "item"))
            }
        }
    }

    @Test fun `conflicting explicit class requests fail before replacing output`() {
        val source = root.resolve("source").createDirectories().resolve("First.java")
        source.writeText("public class First {} class Second {}")
        val original = compile(source.parent)
        val jar = writeJar(listOf("First", "Second").associateWith { original.resolve("$it.class").readBytes() })
        root.resolve("mappings").createDirectories().resolve("First.map")
            .writeText("package readable; class Shared /* was First */ {}")
        root.resolve("mappings/Second.map").writeText("package readable; class Shared /* was Second */ {}")
        val sentinel = root.resolve("out").createDirectories().resolve("sentinel")
        sentinel.writeText("keep")
        val error = assertThrows(IllegalArgumentException::class.java) { decompile(jar) }
        assertTrue(error.message.orEmpty().contains("duplicate readable class name"), error.message)
        assertEquals("keep", sentinel.readText())
    }

    @Test fun `explicit method requests cannot introduce an override`() {
        val source = root.resolve("source/p").createDirectories().resolve("Subject.java")
        source.writeText("package p; public class Subject extends Parent { public int b() { return 2; } } class Parent { public int a() { return 1; } }")
        val original = compile(root.resolve("source"))
        val jar = writeJar(listOf("p/Subject", "p/Parent").associateWith { original.resolve("$it.class").readBytes() })
        root.resolve("mappings").createDirectories().resolve("Names.map").writeText("""
            package p;
            class Subject /* was p/Subject */ { int same() /* was b */; }
            class Parent /* was p/Parent */ { int same() /* was a */; }
        """.trimIndent())
        val sentinel = root.resolve("out").createDirectories().resolve("sentinel")
        sentinel.writeText("keep")
        val error = assertThrows(j2me.validation.MappingValidationException::class.java) { decompile(jar) }
        assertTrue(error.message.orEmpty().contains("override"), error.message)
        assertEquals("keep", sentinel.readText())
    }

    @Tag("integration")
    @Test fun `separated bridge names compile with the actual legacy compiler`() {
        val installed = Path.of(requireNotNull(System.getProperty("j2me.test.installation")))
        org.junit.jupiter.api.Assumptions.assumeTrue(installed.resolve("vendor/compilers/legacy-javac/legacy-javac.jar").exists())
        val source = root.resolve("source/p").createDirectories().resolve("Subject.java")
        source.writeText("""
            package p;
            public class Subject implements Contract {
                public String value() { return "ok"; }
                public static Object direct() { return new Subject().value(); }
                public static Object through() { return ((Contract)new Subject()).value(); }
            }
            interface Contract { Object value(); }
        """.trimIndent())
        val original = compile(root.resolve("source"))
        val jar = writeJar(listOf("p/Subject", "p/Contract").associateWith { name ->
            original.resolve("$name.class").readBytes().also { it[7] = 48 }
        })
        decompile(jar)
        root.resolve("j2me.toml").writeText("jar = \"input.jar\"\n")
        val paths = ToolkitPaths(installed, installed.resolve("config/global.toml"), installed.resolve("guide.md"),
            Path.of(requireNotNull(System.getProperty("sporeflower.test.jar"))))
        val result = compileStubs(root, paths, ProcessCompilerRunner(RealProcessRunner()), CompileStubsArgs(
            compiler = CompileBackend.LEGACY, sourceLevel = "1.3", targetLevel = "1.1",
        ), quiet = true)
        result.requireSuccess()
        for (input in listOf(jar, requireNotNull(result.restoredClasses))) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val subject = loader.loadClass("p.Subject")
                assertEquals("ok", subject.getMethod("direct").invoke(null))
                assertEquals("ok", subject.getMethod("through").invoke(null))
            }
        }
    }

    @Test fun `correspondence follows formatted source and original-line comments`() {
        val jar = nestedInput()
        val sources = decompile(jar, mapOf("preferred-line-length" to "45", "__dump_original_lines__" to "true"))
        val restored = restoreCompiledClasses(jar, compile(sources), sources, root.resolve("restored")).artifact
        URLClassLoader(arrayOf(restored.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            assertEquals("11", loader.loadClass("p.Subject").getMethod("pick", Boolean::class.javaPrimitiveType).invoke(null, true).toString())
        }
    }

    private fun nestedInput(root: Path = this.root): Path {
        val source = root.resolve("source/p").createDirectories().resolve("Subject.java")
        source.writeText("""
            package p;
            public class Subject {
                public static Object pick(boolean first) {
                    if (first) return new Object() {
                        class Member { int value() { return 11; } }
                        public String toString() { return "" + new Member().value(); }
                    };
                    return new Object() { public String toString() { return "second"; } };
                }
                public static Object left() {
                    class Local { public String toString() { return "left"; } }
                    return new Local();
                }
                public static Object right() {
                    class Local { public String toString() { return "right"; } }
                    return new Local();
                }
            }
        """.trimIndent())
        val original = compile(root.resolve("source"), root)
        val bytes = linkedMapOf<String, ByteArray>()
        val rename = object : Remapper(ASM9) {
            override fun map(name: String): String = if (name == "p/Subject\$1" || name.startsWith("p/Subject\$1\$"))
                "p/Subject\$9" + name.removePrefix("p/Subject\$1") else name
        }
        Files.walk(original).use { files -> files.filter { it.extension == "class" }.forEach { file ->
            val reader = ClassReader(file.readBytes())
            val writer = ClassWriter(0)
            reader.accept(ClassRemapper(writer, rename), 0)
            bytes[rename.map(reader.className)] = writer.toByteArray()
        } }
        root.resolve("mappings").createDirectories().resolve("Subject.map")
            .writeText("package q; class Readable /* was p/Subject */ {}")
        return writeJar(bytes, root)
    }

    private fun writeJar(classes: Map<String, ByteArray>, root: Path = this.root): Path = classJar(root.resolve("input.jar"), classes)

    private fun decompile(jar: Path, options: Map<String, String> = emptyMap(), root: Path = this.root): Path =
        decompileForRestoration(root, jar, options).also { sources ->
            assertTrue(SourceMetadata.read(sources.resolve(".sporeflower.json")).classes().isNotEmpty())
        }

    private fun compile(source: Path, root: Path = this.root): Path = compileJava(source, Files.createTempDirectory(root, "compiled-"))
}
