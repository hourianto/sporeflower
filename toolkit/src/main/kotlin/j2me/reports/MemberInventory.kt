package j2me.reports

import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.isConstructor
import j2me.symbols.UsageStats

/** The common definition of a reportable member, shared by coverage and priorities. */
class MemberInventory(symbolsByClass: Map<String, ClassSymbols>, val cmap: CanonicalMap, val usage: UsageStats) {
    val ignoredClassTotal = symbolsByClass.keys.count { it in cmap.ignoredClasses }
    internal val classes = symbolsByClass.toSortedMap().mapNotNull { (owner, symbols) ->
        if (owner in cmap.ignoredClasses) return@mapNotNull null
        val fields = symbols.fields.filterNot(symbols::isGeneratedField)
        ClassMembers(
            owner, fields,
            symbols.methods.filterNot { it.isConstructor() || symbols.isGeneratedMethod(it) },
            fields.filterTo(linkedSetOf()) { (usage.fieldReads[it] ?: 0) > 0 },
        )
    }
}

internal data class ClassMembers(
    val owner: String,
    val fields: List<FieldSig>,
    val methods: List<MethodSig>,
    val activeFields: Set<FieldSig>,
)
