package j2me.cli

import org.tomlj.TomlParseResult
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.Path
import kotlin.io.path.absolute
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.isDirectory
import kotlin.io.path.isRegularFile
import kotlin.io.path.listDirectoryEntries
import kotlin.io.path.notExists
import kotlin.io.path.pathString
import kotlin.io.path.relativeTo

internal fun deleteRecursivelyIfExists(path: Path) {
    if (path.notExists()) {
        return
    }
    Files.walk(path).use { walk ->
        walk.sorted(reverseOrder()).forEach { Files.deleteIfExists(it) }
    }
}

internal fun ensureOutputDir(path: Path, overwrite: Boolean) {
    if (path.exists()) {
        require(path.isDirectory()) { "Output path exists but is not a directory: $path" }
        if (path.listDirectoryEntries().isNotEmpty()) {
            require(overwrite) {
                "Output directory is not empty: $path\nSet remap.overwrite = true in config/global.toml to replace generated output."
            }
            deleteRecursivelyIfExists(path)
        }
    }
    path.createDirectories()
}

internal fun <T> TomlParseResult?.valueOrDefault(key: String, default: T, read: TomlParseResult.(String) -> T?): T =
    this?.read(key) ?: default

internal fun countJavaFiles(root: Path): Int {
    if (!root.exists()) {
        return 0
    }
    return Files.walk(root).use { walk ->
        walk.filter { it.isRegularFile() && it.fileName.toString().endsWith(".java") }.count().toInt()
    }
}

internal fun relativeOrAbsolute(path: Path, root: Path): String = try {
    path.absolute().relativeTo(root.absolute()).pathString
} catch (_: IllegalArgumentException) {
    path.absolute().pathString
}

internal fun formatPercentOneDecimal(mapped: Int, total: Int): String =
    if (total <= 0) "0.0%" else "%.1f%%".format(mapped * 100.0 / total)

internal fun listJavaSources(srcDir: Path): List<Path> {
    if (!srcDir.exists()) {
        return emptyList()
    }
    return Files.walk(srcDir).use { walk ->
        walk
            .filter { it.isRegularFile() && it.fileName.toString().endsWith(".java") }
            .sorted(compareBy { it.pathString })
            .toList()
    }
}

internal data class FileStat(val size: Long, val mtimeMs: Long)

internal fun fileStat(path: Path): FileStat {
    val attrs = Files.readAttributes(path, java.nio.file.attribute.BasicFileAttributes::class.java)
    return FileStat(attrs.size(), attrs.lastModifiedTime().toMillis())
}

internal fun binaryExists(bin: String): Boolean {
    if (bin.isBlank()) {
        return false
    }

    val resolved = Path(bin)
    if (resolved.isAbsolute || bin.endsWith(".jar")) {
        return resolved.exists()
    }

    return try {
        ProcessBuilder("which", bin).start().waitFor() == 0
    } catch (_: IOException) {
        false
    } catch (_: InterruptedException) {
        Thread.currentThread().interrupt()
        false
    }
}
