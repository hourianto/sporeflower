package j2me.common

import com.github.javaparser.ast.type.Type

data class TypeDescriptorResolution(
    val readableToObf: Map<String, String>,
    val knownProjectClasses: Set<String>,
    val knownClasspathClasses: Set<String> = emptySet(),
    val packageName: String = "",
    val imports: JavaImports = JavaImports(),
    val simpleReadableToObf: Map<String, String> = emptyMap(),
    val ambiguousSimpleReadableNames: Map<String, List<String>> = emptyMap(),
    val fallbackToInferredInternalName: Boolean = false,
)

fun typeExprToDescriptor(
    typeExpr: String,
    resolution: TypeDescriptorResolution,
    allowVoid: Boolean,
): String = normalizedTypeToDescriptor(
    normalizedType = normalizeTypeExpr(typeExpr),
    sourceLabel = typeExpr,
    resolution = resolution,
    allowVoid = allowVoid,
)

fun typeNodeToDescriptor(
    type: Type,
    resolution: TypeDescriptorResolution,
    allowVoid: Boolean,
    extraArrayDimensions: Int = 0,
): String {
    require(extraArrayDimensions >= 0) { "extra array dimensions must be >= 0" }
    return normalizedTypeToDescriptor(
        normalizedType = normalizeTypeNode(type) + "[]".repeat(extraArrayDimensions),
        sourceLabel = type.asString(),
        resolution = resolution,
        allowVoid = allowVoid,
    )
}

private fun normalizedTypeToDescriptor(
    normalizedType: String,
    sourceLabel: String,
    resolution: TypeDescriptorResolution,
    allowVoid: Boolean,
): String {
    var normalized = normalizedType
    require(normalized.isNotBlank()) { "invalid type expression: '$sourceLabel'" }

    var arrayDim = 0
    while (normalized.endsWith("[]")) {
        arrayDim += 1
        normalized = normalized.removeSuffix("[]").trim()
    }
    require(normalized.isNotBlank()) { "invalid type expression: '$sourceLabel'" }

    val primitive = primitiveToDescriptor[normalized]
    val baseDesc = if (primitive != null) {
        if (normalized == "void") {
            require(arrayDim == 0) { "void[] is invalid" }
            require(allowVoid) { "void is only allowed as method return type" }
        }
        primitive
    } else {
        val resolvedOwner = resolveOwnerInternalName(normalized, resolution)
        "L$resolvedOwner;"
    }
    return "[".repeat(arrayDim) + baseDesc
}

private fun resolveOwnerInternalName(
    normalizedType: String,
    resolution: TypeDescriptorResolution,
): String {
    fun resolveKnownFqcn(fqcn: String): String? {
        resolution.readableToObf[fqcn]?.let { return it }
        val internal = readableClassToInternal(fqcn)
        if (internal in resolution.knownProjectClasses || internal in resolution.knownClasspathClasses) {
            return internal
        }
        return null
    }

    fun resolveImportedFqcn(fqcn: String): String {
        resolveKnownFqcn(fqcn)?.let { return it }
        if (resolution.fallbackToInferredInternalName) {
            return readableClassToInternal(fqcn)
        }
        throw IllegalArgumentException(
            "unknown class type '$fqcn' (not mapped by class ownership and not found in jar owners)"
        )
    }

    fun resolveKnownDottedName(name: String): String? {
        resolveKnownFqcn(name)?.let { return it }
        val parts = name.split('.')
        if (parts.size < 2) {
            return null
        }
        return (0 until parts.lastIndex)
            .asSequence()
            .map { packagePartCount ->
                parts.take(packagePartCount + 1).joinToString(".") + "$" + parts.drop(packagePartCount + 1).joinToString("$")
            }
            .mapNotNull(::resolveKnownFqcn)
            .firstOrNull()
    }

    resolveKnownFqcn(normalizedType)?.let { return it }

    if ('.' in normalizedType) {
        resolveKnownDottedName(normalizedType)?.let { return it }
        if (resolution.fallbackToInferredInternalName) {
            return readableClassToInternal(normalizedType)
        }
        throw IllegalArgumentException(
            "unknown class type '$normalizedType' (not mapped by class ownership and not found in jar owners)"
        )
    }

    resolution.imports.explicit[normalizedType]?.let { return resolveImportedFqcn(it) }

    if (resolution.packageName.isNotBlank()) {
        resolveKnownFqcn("${resolution.packageName}.$normalizedType")?.let { return it }
    }

    val javaLangInternal = readableClassToInternal("java.lang.$normalizedType")
    val onDemandPackages = mutableListOf<String>()
    if (
        normalizedType in javaLangCommonTypes ||
        javaLangInternal in resolution.knownProjectClasses ||
        javaLangInternal in resolution.knownClasspathClasses
    ) {
        onDemandPackages += "java.lang"
    }
    onDemandPackages += resolution.imports.wildcardPackages

    val wildcardMatches = onDemandPackages
        .map { packageName -> "$packageName.$normalizedType" }
        .mapNotNull { fqcn ->
            resolveKnownFqcn(fqcn)?.let { fqcn to it }
                ?: if (fqcn == "java.lang.$normalizedType" && normalizedType in javaLangCommonTypes) fqcn to javaLangInternal else null
        }
        .distinctBy { it.second }
    require(wildcardMatches.size <= 1) {
        val labels = wildcardMatches.map { it.first }.sorted().joinToString(", ")
        "ambiguous class type '$normalizedType' matches wildcard imports: $labels"
    }
    wildcardMatches.singleOrNull()?.let { return it.second }

    resolution.simpleReadableToObf[normalizedType]?.let { return it }
    val ambiguous = resolution.ambiguousSimpleReadableNames[normalizedType].orEmpty()
    require(ambiguous.isEmpty()) {
        "ambiguous class type '$normalizedType' matches mapped classes: ${ambiguous.sorted().joinToString(", ")}"
    }

    if (resolution.fallbackToInferredInternalName) {
        return readableClassToInternal(
            if (resolution.packageName.isBlank()) normalizedType else "${resolution.packageName}.$normalizedType",
        )
    }

    throw IllegalArgumentException(
        "unknown class type '$normalizedType' (not mapped by class ownership and not found in jar owners)"
    )
}
