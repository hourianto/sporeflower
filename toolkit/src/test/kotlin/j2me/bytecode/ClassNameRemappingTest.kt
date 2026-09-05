package j2me.bytecode

import io.kotest.core.spec.style.FunSpec
import io.kotest.engine.spec.tempdir
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.map.loadJavaLikeMappings
import j2me.output.writeTinyMapping
import j2me.semantic.buildSemanticMappings
import j2me.semantic.validateSemanticMap
import j2me.symbols.collectSymbolsByClass
import org.jetbrains.java.decompiler.api.Decompiler
import org.jetbrains.java.decompiler.main.Init
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver
import java.net.URLClassLoader
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import javax.tools.ToolProvider
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText

class ClassNameRemappingTest : FunSpec({
    val root = tempdir("class-name-remapping").toPath()

    test("class names relocate consistently in bytecode and source without changing ordinary strings") {
        val source = root.resolve("src/sample").createDirectories()
        source.resolve("Target.java").writeText("package sample; public class Target {}")
        source.resolve("Subject.java").writeText("""
            package sample;
            public class Subject {
                public static final String TYPE = "sample.Target";
                public static String[] names = {"sample.Target"};
                public static String[] ordinaryNames = {"sample.Target"};
                static String display;
                public static Class helper(String name) throws Exception { return Class.forName(name); }
                public static String direct() throws Exception { return Class.forName("sample.Target").getName(); }
                public static String copied() throws Exception { String a = "sample.Target"; String b = a; return helper(b).getName(); }
                public static String table() throws Exception { return Class.forName(names[0]).getName(); }
                public static String name() { return "sample.Target"; }
                public static String returned() throws Exception { return Class.forName(name()).getName(); }
                public static String[] array() { return new String[]{"sample.Target"}; }
                public static String returnedArray() throws Exception { return Class.forName(array()[0]).getName(); }
                public static void set(String[] array) { array[0] = "sample.Target"; }
                public static String parameterArray() throws Exception { String[] a = new String[1]; set(a); return Class.forName(a[0]).getName(); }
                public static String descriptor() throws Exception { return Class.forName("[[Lsample.Target;").getName(); }
                public static String external() throws Exception { return Class.forName("java.lang.String").getName(); }
                public static String primitiveArray() throws Exception { return Class.forName("[I").getName(); }
                public static String text() { return "sample.Target"; }
                public static String textArray() { return ordinaryNames[0]; }
                public static String mixed() throws Exception {
                    String name = "sample.Target";
                    display = name;
                    return Class.forName(name).getName();
                }
                public static String branch(boolean first) throws Exception {
                    String name;
                    if (first) name = "sample.Target"; else name = "java.lang.String";
                    return helper(name).getName();
                }
            }
        """.trimIndent())
        val originalClasses = root.resolve("original").createDirectories()
        compile(listOf(source.resolve("Target.java"), source.resolve("Subject.java")), originalClasses)
        val jar = root.resolve("input.jar")
        ZipOutputStream(Files.newOutputStream(jar)).use { zip ->
            Files.walk(originalClasses).use { paths -> paths.filter { Files.isRegularFile(it) }.sorted().forEach {
                zip.putNextEntry(ZipEntry(originalClasses.relativize(it).toString()))
                Files.copy(it, zip)
                zip.closeEntry()
            } }
        }
        val maps = root.resolve("mappings").createDirectories()
        maps.resolve("Names.map").writeText("""
            package named;
            class RenamedTarget /* was sample/Target */ {}
            class RenamedSubject /* was sample/Subject */ {
                @ClassName static final String TYPE /* was TYPE */;
                @ClassName static String[] names /* was names */;
                static Class helper(@ClassName String name) /* was helper */;
                @ClassName static String name() /* was name */;
                @ClassName static String[] array() /* was array */;
                static void set(@ClassName String[] array) /* was set */;
            }
        """.trimIndent())
        val symbols = collectSymbolsByClass(jar, listOf("sample/Subject", "sample/Target"), 1)
        val mappings = loadJavaLikeMappings(maps, symbols.keys)
        validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
        val plan = resolveClassNameRemapping(jar, mappings.canonical, symbols, mappings.semantic.classNames)
        plan.warnings.size shouldBe 1
        plan.warnings.single() shouldContain "also used as ordinary text"
        val output = root.resolve("mapped.jar")
        remapJarBytecode(jar, output, mappings.canonical, symbols, plan.literals)
        val data = buildSemanticMappings(mappings.semantic, mappings.canonical, symbols, classNameLiterals = plan.literals)
        val tiny = root.resolve("mapping.tiny")
        writeTinyMapping(tiny, mappings.canonical, symbols, symbols.keys)
        val decompiled = root.resolve("decompiled").createDirectories()
        Init.init()
        Decompiler.builder().inputs(jar.toFile()).output(DirectoryResultSaver(decompiled.toFile()))
            .option("mappings-path", tiny.toString()).semanticMappings(data).build().decompile()
        val result = decompiled.resolve("named/RenamedSubject.java").readText()
        result shouldContain "Class.forName(\"named.RenamedTarget\")"
        result shouldContain "\"[[Lnamed.RenamedTarget;\""
        val compiled = root.resolve("recompiled").createDirectories()
        compile(Files.walk(decompiled).use { it.filter { file -> file.toString().endsWith(".java") }.toList() }, compiled)
        for (path in listOf(output, compiled)) {
            URLClassLoader(arrayOf(path.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val subject = loader.loadClass("named.RenamedSubject")
                for (name in listOf("direct", "copied", "table", "returned", "returnedArray", "parameterArray")) {
                    subject.getMethod(name).invoke(null) shouldBe "named.RenamedTarget"
                }
                subject.getField("TYPE").get(null) shouldBe "named.RenamedTarget"
                subject.getMethod("descriptor").invoke(null) shouldBe "[[Lnamed.RenamedTarget;"
                subject.getMethod("external").invoke(null) shouldBe "java.lang.String"
                subject.getMethod("primitiveArray").invoke(null) shouldBe "[I"
                for (name in listOf("text", "textArray")) subject.getMethod(name).invoke(null) shouldBe "sample.Target"
                subject.getMethod("branch", Boolean::class.javaPrimitiveType).invoke(null, true) shouldBe "named.RenamedTarget"
                subject.getMethod("branch", Boolean::class.javaPrimitiveType).invoke(null, false) shouldBe "java.lang.String"
            }
        }
    }
})

private fun compile(sources: List<Path>, output: Path) {
    val args = listOf("--release", "8", "-g:none", "-d", output.toString()) + sources.map { it.toString() }
    ToolProvider.getSystemJavaCompiler().run(null, null, null, *args.toTypedArray()) shouldBe 0
}
