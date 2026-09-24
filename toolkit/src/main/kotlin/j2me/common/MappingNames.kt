package j2me.common

import j2me.model.CanonicalMap

/** Explicit names only: default placement belongs to name preparation. */
fun mappedClassName(owner: String, cmap: CanonicalMap): String = cmap.classes[owner] ?: owner

fun internalPackageName(owner: String): String = owner.substringBeforeLast('/', missingDelimiterValue = "")
