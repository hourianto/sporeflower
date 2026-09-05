package j2me.symbols

import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig

/** Resolves bytecode reference owners to the class or interface that declares the member. */
class MemberResolver(
    private val symbolsByClass: Map<String, ClassSymbols>,
) {
    private val methodCache = mutableMapOf<MethodSig, MethodSig>()
    private val fieldCache = mutableMapOf<FieldSig, FieldSig>()

    fun method(reference: MethodSig): MethodSig = methodCache.getOrPut(reference) {
        findDeclaringOwner(reference.owner, preferInterfaces = false) { owner ->
            symbolsByClass[owner]?.methods?.any { it.name == reference.name && it.desc == reference.desc } == true
        }?.let { reference.copy(owner = it) } ?: reference
    }

    fun field(reference: FieldSig): FieldSig = fieldCache.getOrPut(reference) {
        findDeclaringOwner(reference.owner, preferInterfaces = true) { owner ->
            symbolsByClass[owner]?.fields?.any { it.name == reference.name && it.desc == reference.desc } == true
        }?.let { reference.copy(owner = it) } ?: reference
    }

    private fun findDeclaringOwner(
        owner: String,
        preferInterfaces: Boolean,
        declaresMember: (String) -> Boolean,
    ): String? {
        val visited = mutableSetOf<String>()

        fun visit(candidate: String?): String? {
            if (candidate == null || !visited.add(candidate)) return null
            if (declaresMember(candidate)) return candidate

            val symbols = symbolsByClass[candidate] ?: return null
            val parents = if (preferInterfaces) {
                symbols.interfaces + listOfNotNull(symbols.superName)
            } else {
                listOfNotNull(symbols.superName) + symbols.interfaces
            }
            for (parent in parents) {
                visit(parent)?.let { return it }
            }
            return null
        }

        return visit(owner)
    }
}
