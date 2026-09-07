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
    val cachedSymbols = loadSymbolCache(cache.symbols, jar)
    val cachedUsage = if (includeUsage) loadUsageCache(cache.usage, jar) else null
    if (cachedSymbols != null && (!includeUsage || cachedUsage != null)) {
        return JarAnalysis(cachedSymbols, cachedUsage ?: UsageStats())
    }

    val bytes = readClassBytesByOwner(jar)
    val facts = collectJarFacts(bytes, bytes.keys.sorted(), workers, cachedSymbols, includeUsage && cachedUsage == null)
    if (cachedSymbols == null) writeSymbolCache(cache.symbols, jar, facts.symbolsByClass)
    if (includeUsage && cachedUsage == null) writeUsageCache(cache.usage, jar, facts.usage)
    return JarAnalysis(facts.symbolsByClass, cachedUsage ?: facts.usage)
}
