package j2me.symbols

import j2me.model.ClassSymbols
import java.nio.file.Path

data class AnalysisCachePaths(
    val symbols: Path,
    val usage: Path,
)

data class JarAnalysis(
    val symbolsByClass: Map<String, ClassSymbols>,
    val usage: UsageStats = UsageStats(),
) {
    val classes: List<String>
        get() = symbolsByClass.keys.toList()
}

/** Loads the immutable bytecode facts used by mapping, validation, and reports. */
fun analyzeJar(
    jar: Path,
    workers: Int,
    cache: AnalysisCachePaths,
    includeUsage: Boolean,
): JarAnalysis {
    var classBytes: Map<String, ByteArray>? = null

    fun bytes(): Map<String, ByteArray> = classBytes ?: readClassBytesByOwner(jar).also { classBytes = it }

    val cachedSymbols = loadSymbolCache(cache.symbols, jar)
    val symbolsByClass = if (cachedSymbols != null) {
        cachedSymbols.second
    } else {
        val owners = bytes().keys.sorted()
        collectSymbolsByClass(bytes(), owners, workers).also {
            writeSymbolCache(cache.symbols, jar, owners, it)
        }
    }

    if (!includeUsage) {
        return JarAnalysis(symbolsByClass)
    }

    val usage = loadUsageCache(cache.usage, jar) ?: collectSymbolUsage(
        classBytesByOwner = bytes(),
        classes = symbolsByClass.keys.toList(),
        workers = workers,
        symbolsByClass = symbolsByClass,
    ).also { writeUsageCache(cache.usage, jar, it) }

    return JarAnalysis(symbolsByClass, usage)
}
