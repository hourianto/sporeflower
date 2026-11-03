package j2me.testing

import org.junit.jupiter.api.Assertions.assertEquals
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import javax.tools.ToolProvider
import kotlin.io.path.createDirectories
import kotlin.io.path.extension
import kotlin.io.path.isRegularFile
import kotlin.io.path.readBytes

/** Explicit entry names also support resources and intentionally misnamed class files. */
fun writeJar(path: Path, entries: Map<String, ByteArray> = emptyMap()): Path {
    path.parent.createDirectories()
    JarOutputStream(Files.newOutputStream(path)).use { jar ->
        for ((name, bytes) in entries) {
            jar.putNextEntry(JarEntry(name))
            jar.write(bytes)
            jar.closeEntry()
        }
    }
    return path
}

fun classJar(path: Path, classes: Map<String, ByteArray>): Path =
    writeJar(path, classes.mapKeys { (owner, _) -> "$owner.class" })

fun jarDirectory(path: Path, directory: Path, resources: Map<String, ByteArray> = emptyMap()): Path {
    val entries = Files.walk(directory).use { files ->
        files.filter { it.isRegularFile() }.sorted().toList().associate {
            directory.relativize(it).toString().replace('\\', '/') to it.readBytes()
        }
    }
    return writeJar(path, entries + resources)
}

fun compileJava(sources: List<Path>, output: Path, vararg options: String): Path {
    output.createDirectories()
    val diagnostics = ByteArrayOutputStream()
    // javac keeps the first debug option, so an unconditional default masks explicit fixture intent.
    val debug = if (options.any { it == "-g" || it.startsWith("-g:") }) emptyList() else listOf("-g:lines,source")
    val args = listOf("--release", "8", "-encoding", "UTF-8") + debug + listOf("-d", output.toString()) +
        options + sources.map(Path::toString)
    val result = requireNotNull(ToolProvider.getSystemJavaCompiler())
        .run(null, diagnostics, diagnostics, *args.toTypedArray())
    assertEquals(0, result, diagnostics.toString(Charsets.UTF_8))
    return output
}

fun compileJava(sourceDirectory: Path, output: Path, vararg options: String): Path =
    compileJava(Files.walk(sourceDirectory).use { files ->
        files.filter { it.extension == "java" }.sorted().toList()
    }, output, *options)

/** Mutating tests copy shared fixture trees so they remain independent of execution order. */
fun copyDirectory(source: Path, target: Path): Path {
    Files.walk(source).use { paths ->
        paths.forEach { path ->
            val destination = target.resolve(source.relativize(path))
            if (Files.isDirectory(path)) destination.createDirectories() else Files.copy(path, destination)
        }
    }
    return target
}
