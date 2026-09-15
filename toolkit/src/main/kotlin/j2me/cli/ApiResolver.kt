package j2me.cli

import org.jetbrains.java.decompiler.api.J2meApi
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.createDirectories
import kotlin.io.path.isRegularFile

private val apiSnapshotLock = Any()

// Only external compilers and the native CLI's JVM transport need a file.
// Normal decompilation and mapping use the shared in-memory resolution directly.
internal fun resolveApiJars(projectJar: Path, jars: List<Path>, cacheDir: Path, includeBundled: Boolean = false): List<Path> {
    val api = J2meApi.resolve(projectJar, jars, includeBundled)
    return if (api.classes().isEmpty()) emptyList() else listOf(writeApiSnapshot(api, cacheDir))
}

internal fun writeApiSnapshot(api: J2meApi.Resolution, cacheDir: Path): Path {
    val providerIndex = api.providerIndex().toByteArray()
    val digest = MessageDigest.getInstance("SHA-256")
    digest.update("api-snapshot-v3\n".toByteArray())
    digest.update(providerIndex)
    for ((owner, bytes) in api.classes()) {
        digest.update("$owner\n".toByteArray())
        digest.update(bytes)
    }
    val key = digest.digest().joinToString("") { "%02x".format(it) }
    val snapshot = cacheDir.resolve("$key.jar")
    synchronized(apiSnapshotLock) {
        if (!snapshot.isRegularFile()) {
            cacheDir.createDirectories()
            val temporary = Files.createTempFile(cacheDir, "api-", ".jar.tmp")
            try {
                ZipOutputStream(Files.newOutputStream(temporary)).use { output ->
                    for ((owner, bytes) in api.classes()) {
                        output.putNextEntry(ZipEntry("$owner.class").apply { time = 0 })
                        output.write(bytes)
                        output.closeEntry()
                    }
                    output.putNextEntry(ZipEntry("META-INF/j2me-api-sources.tsv").apply { time = 0 })
                    output.write(providerIndex)
                    output.closeEntry()
                }
                Files.move(temporary, snapshot, java.nio.file.StandardCopyOption.REPLACE_EXISTING)
            } finally {
                Files.deleteIfExists(temporary)
            }
        }
    }
    return snapshot
}
