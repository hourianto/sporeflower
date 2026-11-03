package j2me.cli

import j2me.process.RealProcessRunner
import j2me.testing.compileJava
import j2me.testing.jarDirectory
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.net.URLClassLoader
import java.nio.file.Path
import kotlin.io.path.*

class RealizedNamingFlowTest {
    @TempDir lateinit var root: Path

    @Test fun `semantics bytecode and independently compiled source use completed names`() {
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
        val original = compileJava(listOf(source), root.resolve("original"))
        val jar = jarDirectory(root.resolve("input.jar"), original)
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
        val paths = testToolkitPaths(root)
        val result = runRemapPipeline(buildRemapPipelineArgs(root, paths, null, jar, false, true),
            SporeflowerRunner(paths, RealProcessRunner()), quiet = true)
        val sources = root.resolve("decompiled")
        val text = sources.resolve("p/Existing_1.java").readText()
        assertTrue(text.contains("Direction.ONE"), text)
        assertTrue(text.contains("Class.forName(\"p.Existing_1\")"), text)
        assertFalse(sources.resolve(".sporeflower.json").exists())
        val base = sources.resolve("p/Base.java").readText()
        assertTrue(base.contains("return Direction.ONE;"), base)
        assertTrue(base.contains("return Position.TWO;"), base)
        val compiled = compileJava(sources, root.resolve("compiled"))
        assertTrue(compiled.resolve("p/Direction.class").exists())
        for (artifact in listOf(result.mappings!!.remappedJar.path, compiled)) {
            URLClassLoader(arrayOf(artifact.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val type = loader.loadClass("p.Existing_1")
                assertEquals(3, type.getMethod("read").invoke(null))
                assertEquals(3, type.getMethod("callBase").invoke(null))
                assertEquals(47, type.getMethod("readBaseFields").invoke(null))
                assertEquals(1, type.getMethod("value").invoke(null))
                assertEquals(type, type.getMethod("reflected").invoke(null))
            }
        }
    }
}
