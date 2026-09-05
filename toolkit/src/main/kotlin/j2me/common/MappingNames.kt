package j2me.common

import j2me.model.CanonicalMap

/** The class name emitted to Tiny mappings, remapped bytecode, and decompiled sources. */
fun mappedClassName(owner: String, cmap: CanonicalMap): String {
    val mapped = cmap.classes[owner]
    if (mapped == null) {
        return if ('/' in owner) owner else "defpackage/$owner"
    }
    return if ('/' !in owner && '/' !in mapped) "defpackage/$mapped" else mapped
}

fun internalPackageName(owner: String): String = owner.substringBeforeLast('/', missingDelimiterValue = "")
