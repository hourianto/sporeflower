package j2me.cli

import j2me.bytecode.restoreCompiledClasses
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
import kotlin.io.path.*

class RealizedNamingFlowTest {
    @TempDir lateinit var root: Path

    @Test fun `semantics bytecode and restoration use collision repairs rather than requested names`() {
        val source = root.resolve("Original.java")
        source.writeText("""
            package p;
            public class Original extends Base {
                public static int other = 2;
                public static int a() { return 1; }
                public static int read() { return Original.value + other; }
                public static int callBase() { return Base.a() + Base.b(); }
                public static int readBaseFields() { return Base.x * 10 + Base.y; }
                public static Class reflected() throws Exception { return Class.forName("p.Original"); }
            }
            class Existing {}
            class Base {
                public static int value = 1;
                public static int x = 4, y = 7;
                public static int a() { return 1; }
                public static int b() { return 2; }
            }
        """.trimIndent())
        val compiler = requireNotNull(ToolProvider.getSystemJavaCompiler())
        val original = root.resolve("original").createDirectories()
        assertEquals(0, compiler.run(null, null, null, "--release", "8", "-d", original.toString(), source.toString()))
        val jar = root.resolve("input.jar")
        JarOutputStream(Files.newOutputStream(jar)).use { output ->
            Files.walk(original).use { files -> files.filter { it.isRegularFile() }.forEach {
                output.putNextEntry(JarEntry(original.relativize(it).toString())); Files.copy(it, output); output.closeEntry()
            } }
        }
        root.resolve("mappings").createDirectories().resolve("Names.map").writeText("""
            package p;
            @ValueDomain interface Direction { int ONE = 1; }
            @ValueDomain interface Position { int TWO = 2; }
            class Existing /* was p/Original */ {
                @Domain(Direction.class) static int value() /* was a */;
                static int value /* was other */;
            }
            class Base /* was p/Base */ {
                static int y /* was x */;
                static int z /* was y */;
                @Domain(Direction.class) static int b() /* was a */;
                @Domain(Position.class) static int c() /* was b */;
            }
        """.trimIndent())
        val engine = Path.of(requireNotNull(System.getProperty("sporeflower.test.jar")))
        val paths = ToolkitPaths(root, root.resolve("global.toml"), root.resolve("guide.md"), engine)
        val result = runRemapPipeline(buildRemapPipelineArgs(root, paths, null, jar, false, true),
            SporeflowerRunner(paths, RealProcessRunner()), quiet = true)
        val sources = root.resolve("decompiled")
        val text = sources.resolve("p/Existing_1.java").readText()
        assertTrue(text.contains("Direction.ONE"), text)
        assertTrue(text.contains("\"p.Original\""), text)
        val base = sources.resolve("p/Base.java").readText()
        assertTrue(base.contains("return Direction.ONE;"), base)
        assertTrue(base.contains("return Position.TWO;"), base)
        URLClassLoader(arrayOf(result.mappings!!.remappedJar.path.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            val type = loader.loadClass("p.Existing_1")
            assertEquals(3, type.getMethod("read").invoke(null))
            assertEquals(3, type.getMethod("callBase").invoke(null))
            assertEquals(47, type.getMethod("readBaseFields").invoke(null))
            assertEquals("p.Existing_1", (type.getMethod("reflected").invoke(null) as Class<*>).name)
        }
        val compiled = root.resolve("compiled").createDirectories()
        val files = Files.walk(sources).use { it.filter { file -> file.toString().endsWith(".java") }.toList() }
        assertEquals(0, compiler.run(null, null, null, *(listOf("--release", "8", "-d", compiled.toString()) + files.map(Path::toString)).toTypedArray()))
        val restored = restoreCompiledClasses(jar, compiled, sources, root.resolve("restored")).directory
        assertTrue(restored.resolve("p/Direction.class").exists())
        URLClassLoader(arrayOf(restored.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
            val type = loader.loadClass("p.Original")
            assertEquals(3, type.getMethod("read").invoke(null))
            assertEquals(3, type.getMethod("callBase").invoke(null))
            assertEquals(47, type.getMethod("readBaseFields").invoke(null))
            assertEquals(1, type.getMethod("a").invoke(null))
            assertEquals("p.Original", (type.getMethod("reflected").invoke(null) as Class<*>).name)
        }
    }
}
