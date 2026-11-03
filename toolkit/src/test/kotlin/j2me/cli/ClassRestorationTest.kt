package j2me.cli

import j2me.bytecode.restoreCompiledClasses
import j2me.process.RealProcessRunner
import j2me.testing.classJar
import j2me.testing.compileJava
import j2me.testing.copyDirectory
import j2me.testing.jarDirectory
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.BeforeAll
import org.junit.jupiter.api.Tag
import org.junit.jupiter.api.TestInstance
import org.junit.jupiter.api.Assumptions.assumeTrue
import org.junit.jupiter.api.io.TempDir
import org.junit.jupiter.api.io.CleanupMode
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.ClassReader
import org.objectweb.asm.tree.ClassNode
import org.objectweb.asm.tree.MethodInsnNode
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.Type
import kotlin.io.path.*

@TestInstance(TestInstance.Lifecycle.PER_CLASS)
class ClassRestorationTest {
    @TempDir(cleanup = CleanupMode.ON_SUCCESS) lateinit var root: Path
    private lateinit var fixtureRoot: Path
    private val modern by lazy { modernFixture(fixtureRoot.resolve("modern").createDirectories()) }
    private val legacy by lazy { legacyFixture(fixtureRoot.resolve("legacy").createDirectories()) }

    @BeforeAll fun prepareFixtureDirectory(@TempDir(cleanup = CleanupMode.ON_SUCCESS) directory: Path) {
        // The class-scoped directory outlives each test's scratch directory; fixtures stay read-only.
        fixtureRoot = directory
    }

    private fun modernFixture(root: Path): Fixture {
        val source = root.resolve("src/p").createDirectories().resolve("Subject.java")
        source.writeText("""
            package p;
            public class Subject extends Exception {
                public Subject() { super("message"); }
                public static final String NAME = "p.Subject";
                public static Class reflected() throws Exception { return Class.forName(NAME); }
                public static Class computed() throws Exception { return Class.forName("p." + new String(new char[]{'S','u','b','j','e','c','t'})); }
                public static Class literal() { return Subject.class; }
                public static String exception() { return new Subject().toString(); }
                public static int read(Class type, String path) throws Exception {
                    java.io.InputStream stream = type.getResourceAsStream(path);
                    return stream == null ? -1 : stream.read();
                }
                public int resource(String path) throws Exception { return read(getClass(), path); }
                public static int nested() { return new Nested().value(); }
                public static class Nested { private Nested() {} public int value() { return 31; } }
                public static Object local() {
                    class Local { public String toString() { return "local"; } }
                    return new Local();
                }
                public static Object anonymous() { return new Object() { public String toString() { return "anonymous"; } }; }
                public Object body() {
                    class Local { public String toString() { return "special-local"; } }
                    return new Local();
                }
                public Object special() { return body(); }
            }
        """.trimIndent())
        val child = root.resolve("src/q").createDirectories().resolve("Child.java")
        child.writeText("package q; public class Child extends p.Subject { public Object body() { return \"override\"; } }")
        val original = root.resolve("original").createDirectories()
        compileJava(listOf(source, child), original, "-g")
        // An overridden same-owner special target moves its local class into a private helper.
        val classFile = original.resolve("p/Subject.class")
        val node = ClassNode().also { ClassReader(classFile.readBytes()).accept(it, 0) }
        node.methods.single { it.name == "special" }.instructions.filterIsInstance<MethodInsnNode>()
            .single { it.name == "body" }.opcode = INVOKESPECIAL
        classFile.writeBytes(ClassWriter(0).also { node.accept(it) }.toByteArray())
        val jar = jarDirectory(root.resolve("input.jar"), original,
            mapOf("p/item" to byteArrayOf(17), "q/item" to byteArrayOf(29), "root" to byteArrayOf(41)))
        root.resolve("mappings").createDirectories().resolve("Subject.map").writeText("""
            package readable;
            class Subject /* was p/Subject */ {}
        """.trimIndent())
        val sources = decompileForRestoration(root, jar)
        assertTrue(sources.resolve("readable/Subject.java").readText().contains("\"p.Subject\""))
        val compiled = root.resolve("compiled").createDirectories()
        compileJava(sources, compiled, "-g")
        val restored = restoreCompiledClasses(jar, compiled, sources, root.resolve("restored")).artifact
        val resources = root.resolve("resources")
        extractResources(jar, resources)
        return Fixture(jar, sources, compiled, restored, resources)
    }

    private data class Fixture(val jar: Path, val sources: Path, val compiled: Path, val restored: Path, val resources: Path)

    private fun Fixture.compare(check: (ClassLoader, ClassLoader) -> Unit) {
        URLClassLoader(arrayOf(jar.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { before ->
            URLClassLoader(arrayOf(restored.toUri().toURL(), resources.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { after ->
                check(before, after)
            }
        }
    }

    @Test fun `reflection class literals and exception names use restored identities`() {
        modern.compare { before, after ->
            val old = before.loadClass("p.Subject")
            val new = after.loadClass("p.Subject")
            for (method in listOf("reflected", "computed", "literal")) {
                assertEquals((old.getMethod(method).invoke(null) as Class<*>).name, (new.getMethod(method).invoke(null) as Class<*>).name)
            }
            assertEquals(old.getMethod("exception").invoke(null), new.getMethod("exception").invoke(null))
        }
    }

    @Test fun `relative absolute missing and subclass resource lookups agree`() {
        modern.compare { before, after ->
            for (owner in listOf("p.Subject", "q.Child")) for (path in listOf("item", "/root", "/absent", "missing", "/readable/item")) {
                val oldType = before.loadClass(owner)
                val newType = after.loadClass(owner)
                assertEquals(oldType.getMethod("resource", String::class.java).invoke(oldType.getConstructor().newInstance(), path),
                    newType.getMethod("resource", String::class.java).invoke(newType.getConstructor().newInstance(), path), "$owner $path")
            }
        }
    }

    @Test fun `nested classes restore with special dispatch and lexical scope`() {
        modern.compare { before, after ->
            for (loader in listOf(before, after)) {
                val type = loader.loadClass("q.Child")
                val instance = type.getConstructor().newInstance()
                assertEquals("special-local", type.getMethod("special").invoke(instance).toString())
                assertEquals("override", type.getMethod("body").invoke(instance))
            }
            for (method in listOf("nested", "local", "anonymous")) {
                assertEquals(before.loadClass("p.Subject").getMethod(method).invoke(null).toString(),
                    after.loadClass("p.Subject").getMethod(method).invoke(null).toString(), method)
            }
        }
    }

    @Test fun `restored classes cannot fall back to original bytecode`() {
        val restored = copyDirectory(modern.restored, root.resolve("restored"))
        Files.delete(restored.resolve("p/Subject.class"))
        URLClassLoader(arrayOf(restored.toUri().toURL(), modern.resources.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            assertThrows(ClassNotFoundException::class.java) { loader.loadClass("p.Subject") }
        }
    }

    @Test fun `renamed application strings require regeneration before restoration`() {
        val sources = copyDirectory(modern.sources, root.resolve("sources"))
        val metadataPath = sources.resolve(".sporeflower.json")
        val metadata = org.jetbrains.java.decompiler.api.SourceMetadata.read(metadataPath)
        org.jetbrains.java.decompiler.api.SourceMetadata("renamed", metadata.names(), metadata.classes()).write(metadataPath)
        assertThrows(IllegalArgumentException::class.java) {
            restoreCompiledClasses(modern.jar, modern.compiled, sources, root.resolve("rejected"))
        }
    }

    private fun legacyFixture(root: Path): Pair<Path, Path> {
        val installed = Path.of(requireNotNull(System.getProperty("j2me.test.installation")))
        assumeTrue(installed.resolve("vendor/compilers/legacy-javac/legacy-javac.jar").exists())
        val probe = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_3, ACC_PUBLIC, "Probe", null, "java/lang/Object", null)
            visitField(ACC_STATIC, "log", "I", null, null).visitEnd()
            visitMethod(ACC_PUBLIC or ACC_STATIC, "mark", "(I)I", null, null).apply {
                visitCode(); visitFieldInsn(GETSTATIC, "Probe", "log", "I"); visitIntInsn(BIPUSH, 10); visitInsn(IMUL)
                visitVarInsn(ILOAD, 0); visitInsn(IADD); visitFieldInsn(PUTSTATIC, "Probe", "log", "I")
                visitVarInsn(ILOAD, 0); visitInsn(IRETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC or ACC_STATIC, "result", "()I", null, null).apply {
                visitCode(); visitFieldInsn(GETSTATIC, "Probe", "log", "I"); visitInsn(IRETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val host = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_3, ACC_PUBLIC or ACC_INTERFACE or ACC_ABSTRACT, "Host", null, "java/lang/Object", null)
            visitField(ACC_PUBLIC or ACC_STATIC or ACC_FINAL, "VALUE", "I", null, null).visitEnd()
            visitMethod(ACC_STATIC, "<clinit>", "()V", null, null).apply {
                visitCode(); visitInsn(ICONST_1); visitMethodInsn(INVOKESTATIC, "Probe", "mark", "(I)I", false); visitInsn(POP)
                visitInsn(ICONST_2); visitMethodInsn(INVOKESTATIC, "Probe", "mark", "(I)I", false)
                visitFieldInsn(PUTSTATIC, "Host", "VALUE", "I"); visitInsn(RETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val literal = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_8, ACC_PUBLIC, "LiteralProbe", null, "java/lang/Object", null)
            visitInnerClass("LiteralProbe\$1", null, null, 0)
            visitMethod(ACC_PUBLIC or ACC_STATIC, "type", "()Ljava/lang/Class;", null, null).apply {
                visitCode(); visitLdcInsn(Type.getObjectType("Host")); visitInsn(ARETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC or ACC_STATIC, "arrayType", "()Ljava/lang/Class;", null, null).apply {
                visitCode(); visitLdcInsn(Type.getType("[[LHost;")); visitInsn(ARETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC or ACC_STATIC, "text", "()Ljava/lang/String;", null, null).apply {
                visitCode(); visitLdcInsn("defpackage.Host"); visitInsn(ARETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC or ACC_STATIC, "anonymous", "()Ljava/lang/Object;", null, null).apply {
                visitCode(); visitTypeInsn(NEW, "LiteralProbe\$1"); visitInsn(DUP)
                visitMethodInsn(INVOKESPECIAL, "LiteralProbe\$1", "<init>", "()V", false)
                visitInsn(ARETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val anonymous = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_3, ACC_SUPER, "LiteralProbe\$1", null, "java/lang/Object", null)
            visitInnerClass("LiteralProbe\$1", null, null, 0)
            visitMethod(0, "<init>", "()V", null, null).apply {
                visitCode(); visitVarInsn(ALOAD, 0); visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
                visitInsn(RETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC, "toString", "()Ljava/lang/String;", null, null).apply {
                visitCode(); visitLdcInsn("anonymous"); visitInsn(ARETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        val jar = root.resolve("input.jar")
        val contract = ClassWriter(0).apply {
            visit(V1_3, ACC_PUBLIC or ACC_INTERFACE or ACC_ABSTRACT, "Contract", null, "java/lang/Object", null)
            visitMethod(ACC_PUBLIC or ACC_ABSTRACT, "if", "()I", null, null).visitEnd()
            visitEnd()
        }
        val incomplete = ClassWriter(ClassWriter.COMPUTE_MAXS).apply {
            visit(V1_3, ACC_PUBLIC or ACC_FINAL, "Incomplete", null, "java/lang/Object", arrayOf("Contract"))
            visitMethod(ACC_PUBLIC, "<init>", "()V", null, null).apply {
                visitCode(); visitVarInsn(ALOAD, 0); visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
                visitInsn(RETURN); visitMaxs(0, 0); visitEnd()
            }
            visitMethod(ACC_PUBLIC or ACC_STATIC, "run", "()I", null, null).apply {
                visitCode(); visitTypeInsn(NEW, "Incomplete"); visitInsn(DUP)
                visitMethodInsn(INVOKESPECIAL, "Incomplete", "<init>", "()V", false)
                visitMethodInsn(INVOKEINTERFACE, "Contract", "if", "()I", true)
                visitInsn(IRETURN); visitMaxs(0, 0); visitEnd()
            }
            visitEnd()
        }
        classJar(jar, mapOf("Host" to host, "Probe" to probe, "LiteralProbe" to literal, "LiteralProbe\$1" to anonymous,
            "Contract" to contract, "Incomplete" to incomplete).mapValues { it.value.toByteArray() })
        root.resolve("j2me.toml").writeText("jar = \"input.jar\"\n")
        root.resolve("mappings").createDirectories()
        val paths = ToolkitPaths(installed, installed.resolve("config/global.toml"), installed.resolve("guide.md"),
            Path.of(requireNotNull(System.getProperty("sporeflower.test.jar"))))
        val process = RealProcessRunner()
        runRemapPipeline(buildRemapPipelineArgs(root, paths, null, jar, false, true).copy(restorationAudit = true),
            SporeflowerRunner(paths, process), quiet = true)
        assertTrue(root.resolve("decompiled/defpackage/Host.java").readText().contains("VFInterfaceInitializer"))
        val result = compileStubs(root, paths, ProcessCompilerRunner(process), CompileStubsArgs(
            compiler = CompileBackend.LEGACY, sourceLevel = "1.3", targetLevel = "1.1",
        ), quiet = true)
        result.requireSuccess()
        val restored = requireNotNull(result.restoredClasses)
        assertTrue(restored.resolve("Host\$VFInterfaceInitializer.class").exists())
        return jar to restored
    }

    @Tag("integration")
    @Test fun `legacy hosted initializer follows its restored package`() {
        val (jar, restored) = legacy
        for (input in listOf(jar, restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                Class.forName("Host", true, loader)
                assertEquals(12, loader.loadClass("Probe").getMethod("result").invoke(null))
            }
        }
    }

    @Tag("integration")
    @Test fun `legacy class literal caches restore arrays without changing ordinary strings`() {
        val (jar, restored) = legacy
        for (input in listOf(jar, restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val type = loader.loadClass("LiteralProbe")
                assertEquals("Host", (type.getMethod("type").invoke(null) as Class<*>).name)
                assertEquals("[[LHost;", (type.getMethod("arrayType").invoke(null) as Class<*>).name)
                assertEquals("defpackage.Host", type.getMethod("text").invoke(null))
                assertEquals("anonymous", type.getMethod("anonymous").invoke(null).toString())
            }
        }
    }

    @Tag("integration")
    @Test fun `generated missing-method stub fails as specified by the selected API`() {
        val (jar, restored) = legacy
        for (input in listOf(jar, restored)) {
            URLClassLoader(arrayOf(input.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val failure = assertThrows(java.lang.reflect.InvocationTargetException::class.java) {
                    loader.loadClass("Incomplete").getMethod("run").invoke(null)
                }
                // The selected CLDC API lacks AbstractMethodError, so the existing stub uses Error.
                assertEquals(if (input == restored) Error::class.java else AbstractMethodError::class.java, failure.cause!!.javaClass)
                if (input == restored) assertNotNull(loader.loadClass("Incomplete").getDeclaredMethod("if"))
            }
        }
    }
}
