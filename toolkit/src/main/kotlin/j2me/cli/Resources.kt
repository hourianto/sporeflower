package j2me.cli

import j2me.common.isJavaClassFile
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipFile
import kotlin.io.path.absolute
import kotlin.io.path.createDirectories

internal fun runExtractResources(root: Path) {
    val jar = resolveProjectJar(root)

    val outDir = root.resolve("resources")
    deleteRecursivelyIfExists(outDir)
    outDir.createDirectories()

    val outResolved = outDir.absolute().normalize()
    try {
        ZipFile(jar.toFile()).use { archive ->
            for (info in archive.entries()) {
                val entry = info.name
                if (entry.isBlank() || info.isDirectory) {
                    continue
                }
                if (entry.endsWith(".class") && archive.getInputStream(info).use { isJavaClassFile(it.readNBytes(4)) }) {
                    continue
                }

                val dest = outDir.resolve(entry).absolute().normalize()
                require(dest.startsWith(outResolved)) { "Refusing to extract unsafe path: $entry" }
                dest.parent.createDirectories()
                archive.getInputStream(info).use { src ->
                    Files.newOutputStream(dest).use { dst -> src.copyTo(dst) }
                }
            }
        }
    } catch (exc: IOException) {
        throw IllegalArgumentException("Failed to read JAR archive: $jar\n${exc.message}", exc)
    }

    println("Extracted non-code resources to: $outDir")
}
