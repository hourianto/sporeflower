package j2me.common

import j2me.model.CanonicalMap
import j2me.model.FieldSig
import j2me.model.MethodSig

fun displayType(typeName: String, cmap: CanonicalMap): String {
    val mapped = remapTypeName(typeName, cmap.classes)
    if (mapped in primitiveTypeNames) {
        return mapped
    }
    val dotted = internalToJava(mapped)
    return when {
        dotted.startsWith("java.lang.") -> dotted.substringAfterLast('.')
        dotted.startsWith("defpackage.") -> dotted.substringAfter('.')
        else -> dotted
    }
}

fun displayOwner(owner: String, cmap: CanonicalMap): String {
    val mappedInternal = cmap.classes[owner] ?: owner
    val mapped = internalToJava(mappedInternal).removePrefix("defpackage.")
    val obf = internalToJava(owner)
    return if (mapped == obf) obf else "$mapped ($obf)"
}

fun formatMethodSignatureJava(sig: MethodSig, cmap: CanonicalMap): String {
    val (args, ret) = parseMethodDescriptor(sig.desc)
    val renderedArgs = args.joinToString(", ") { displayType(it, cmap) }
    return "${displayType(ret, cmap)} ${sig.name}($renderedArgs)"
}

fun formatFieldSignatureJava(sig: FieldSig, cmap: CanonicalMap): String {
    return try {
        val parsed = parseTypeDescriptor(sig.desc)
        if (parsed.nextIndex != sig.desc.length) {
            "${sig.name}:${sig.desc}"
        } else {
            "${displayType(parsed.typeName, cmap)} ${sig.name}"
        }
    } catch (_: IllegalArgumentException) {
        "${sig.name}:${sig.desc}"
    }
}
