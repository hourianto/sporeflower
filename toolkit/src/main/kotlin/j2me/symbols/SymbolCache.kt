package j2me.symbols

import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText

private const val usageCacheVersion = 4
private const val symbolCacheVersion = 8

private val json = Json {
    ignoreUnknownKeys = false
    encodeDefaults = true
    explicitNulls = false
}

@Serializable
data class JarFingerprint(
    val path: String,
    val size: Long,
    @SerialName("mtime_ms") val mtimeMs: Long,
)

@Serializable
private data class CachedMethodEntry(
    val owner: String,
    val name: String,
    val desc: String,
    val refs: Int,
    val callers: List<String>,
) {
    val sig: MethodSig get() = MethodSig(owner, name, desc)
}

@Serializable
private data class CachedFieldEntry(
    val owner: String,
    val name: String,
    val desc: String,
    val reads: Int,
    val writes: Int,
    val accessors: List<String>,
) {
    val sig: FieldSig get() = FieldSig(owner, name, desc)
}

@Serializable
private data class UsageCachePayload(
    val version: Int,
    val jar: JarFingerprint,
    val methods: List<CachedMethodEntry>,
    val fields: List<CachedFieldEntry>,
)

@Serializable
private data class SymbolFieldEntry(
    val name: String,
    val desc: String,
    val access: Int,
    val constantValue: String? = null,
)

@Serializable
private data class SymbolMethodEntry(
    val name: String,
    val desc: String,
    val access: Int,
)

@Serializable
private data class SymbolClassEntry(
    val owner: String,
    val superName: String? = null,
    val interfaces: List<String> = emptyList(),
    val fields: List<SymbolFieldEntry>,
    val methods: List<SymbolMethodEntry>,
) {
    fun toClassSymbols(): ClassSymbols {
        val methodSigs = mutableListOf<MethodSig>()
        val methodAccess = linkedMapOf<MethodSig, Int>()
        val fieldSigs = mutableListOf<FieldSig>()
        val fieldAccess = linkedMapOf<FieldSig, Int>()
        val fieldConstantValues = linkedMapOf<FieldSig, String>()
        for (field in fields) {
            val sig = FieldSig(owner, field.name, field.desc)
            fieldSigs += sig
            fieldAccess[sig] = field.access
            field.constantValue?.let { fieldConstantValues[sig] = it }
        }
        for (method in methods) {
            val sig = MethodSig(owner, method.name, method.desc)
            methodSigs += sig
            methodAccess[sig] = method.access
        }
        return ClassSymbols(
            fields = fieldSigs,
            methods = methodSigs,
            methodAccess = methodAccess,
            fieldAccess = fieldAccess,
            fieldConstantValues = fieldConstantValues,
            superName = superName,
            interfaces = interfaces,
        )
    }
}

@Serializable
private data class SymbolCachePayload(
    val version: Int,
    val jar: JarFingerprint,
    val classes: List<SymbolClassEntry>,
)

fun jarFingerprint(jarPath: Path): JarFingerprint {
    val attrs = Files.readAttributes(jarPath, java.nio.file.attribute.BasicFileAttributes::class.java)
    return JarFingerprint(
        path = jarPath.toAbsolutePath().normalize().toString(),
        size = attrs.size(),
        mtimeMs = attrs.lastModifiedTime().toMillis(),
    )
}

fun loadUsageCache(cachePath: Path, jarPath: Path): UsageStats? {
    val payload = readJsonCache(cachePath, UsageCachePayload.serializer()) ?: return null
    if (payload.version != usageCacheVersion || payload.jar != jarFingerprint(jarPath)) {
        return null
    }

    return UsageStats(
        methodRefs = payload.methods.associateTo(linkedMapOf()) { it.sig to it.refs },
        methodCallers = payload.methods.associateTo(linkedMapOf()) { it.sig to it.callers.toSet() },
        fieldReads = payload.fields.associateTo(linkedMapOf()) { it.sig to it.reads },
        fieldWrites = payload.fields.associateTo(linkedMapOf()) { it.sig to it.writes },
        fieldAccessors = payload.fields.associateTo(linkedMapOf()) { it.sig to it.accessors.toSet() },
    )
}

fun writeUsageCache(cachePath: Path, jarPath: Path, usage: UsageStats) {
    val methodEntries = usage.methodRefs.keys
        .sortedWith(compareBy({ it.owner }, { it.name }, { it.desc }))
        .map { sig ->
            CachedMethodEntry(
                owner = sig.owner,
                name = sig.name,
                desc = sig.desc,
                refs = usage.methodRefs[sig] ?: 0,
                callers = usage.methodCallers[sig].orEmpty().sorted(),
            )
        }

    val fieldEntries = (usage.fieldReads.keys + usage.fieldWrites.keys + usage.fieldAccessors.keys)
        .toSet()
        .sortedWith(compareBy({ it.owner }, { it.name }, { it.desc }))
        .map { sig ->
            CachedFieldEntry(
                owner = sig.owner,
                name = sig.name,
                desc = sig.desc,
                reads = usage.fieldReads[sig] ?: 0,
                writes = usage.fieldWrites[sig] ?: 0,
                accessors = usage.fieldAccessors[sig].orEmpty().sorted(),
            )
        }

    writeJsonCache(cachePath, UsageCachePayload.serializer(), UsageCachePayload(usageCacheVersion, jarFingerprint(jarPath), methodEntries, fieldEntries))
}

fun loadSymbolCache(cachePath: Path, jarPath: Path): Pair<List<String>, Map<String, ClassSymbols>>? {
    val payload = readJsonCache(cachePath, SymbolCachePayload.serializer()) ?: return null
    if (payload.version != symbolCacheVersion || payload.jar != jarFingerprint(jarPath)) {
        return null
    }

    val classes = payload.classes.map { it.owner }
    val symbolsByClass = payload.classes.associateTo(linkedMapOf()) { it.owner to it.toClassSymbols() }
    return classes to symbolsByClass
}

fun writeSymbolCache(
    cachePath: Path,
    jarPath: Path,
    classes: List<String>,
    symbolsByClass: Map<String, ClassSymbols>,
) {
    val classEntries = classes.mapNotNull { owner ->
        val symbols = symbolsByClass[owner] ?: return@mapNotNull null
        SymbolClassEntry(
            owner = owner,
            superName = symbols.superName,
            interfaces = symbols.interfaces,
            fields = symbols.fields.map {
                SymbolFieldEntry(it.name, it.desc, symbols.fieldAccess[it] ?: 0, symbols.fieldConstantValues[it])
            },
            methods = symbols.methods.map { sig ->
                SymbolMethodEntry(sig.name, sig.desc, symbols.methodAccess[sig] ?: 0)
            },
        )
    }

    writeJsonCache(cachePath, SymbolCachePayload.serializer(), SymbolCachePayload(symbolCacheVersion, jarFingerprint(jarPath), classEntries))
}

private fun <T> readJsonCache(path: Path, serializer: KSerializer<T>): T? {
    return try {
        json.decodeFromString(serializer, path.readText())
    } catch (_: IOException) {
        null
    } catch (_: SerializationException) {
        null
    }
}

private fun <T> writeJsonCache(path: Path, serializer: KSerializer<T>, payload: T) {
    path.parent?.createDirectories()
    val tmp = path.resolveSibling(path.fileName.toString() + ".tmp")
    tmp.writeText(json.encodeToString(serializer, payload))
    moveReplace(tmp, path)
}

private fun moveReplace(tmp: Path, target: Path) {
    try {
        Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
        Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING)
    }
}
