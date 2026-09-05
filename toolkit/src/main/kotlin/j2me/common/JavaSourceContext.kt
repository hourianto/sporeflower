package j2me.common

import com.github.javaparser.ast.CompilationUnit

data class JavaImports(
    val explicit: Map<String, String> = emptyMap(),
    val wildcardPackages: List<String> = emptyList(),
)

data class JavaSourceContext(
    val packageName: String,
    val imports: JavaImports,
)

data class SimpleClassNameLookup(
    val unique: Map<String, String>,
    val ambiguous: Map<String, List<String>>,
)

fun typeDescriptorResolution(
    readableToObf: Map<String, String>,
    knownProjectClasses: Set<String>,
    knownClasspathClasses: Set<String>,
    sourceContext: JavaSourceContext,
    fallbackToInferredInternalName: Boolean,
): TypeDescriptorResolution {
    val simpleLookup = buildSimpleClassNameLookup(readableToObf)
    return TypeDescriptorResolution(
        readableToObf = readableToObf,
        knownProjectClasses = knownProjectClasses,
        knownClasspathClasses = knownClasspathClasses,
        packageName = sourceContext.packageName,
        imports = sourceContext.imports,
        simpleReadableToObf = simpleLookup.unique,
        ambiguousSimpleReadableNames = simpleLookup.ambiguous,
        fallbackToInferredInternalName = fallbackToInferredInternalName,
    )
}

fun javaSourceContext(cu: CompilationUnit): JavaSourceContext {
    val explicit = linkedMapOf<String, String>()
    val wildcardPackages = mutableListOf<String>()
    cu.imports
        .filter { !it.isStatic }
        .forEach { imp ->
            if (imp.isAsterisk) {
                wildcardPackages += imp.nameAsString
            } else {
                explicit[imp.name.identifier] = imp.nameAsString
            }
        }

    return JavaSourceContext(
        packageName = cu.packageDeclaration.map { it.nameAsString }.orElse(""),
        imports = JavaImports(explicit, wildcardPackages.distinct()),
    )
}

fun buildSimpleClassNameLookup(readableToObf: Map<String, String>): SimpleClassNameLookup {
    val grouped = linkedMapOf<String, MutableList<Pair<String, String>>>()
    readableToObf.forEach { (readable, owner) ->
        grouped.getOrPut(readable.substringAfterLast('.')) { mutableListOf() }.add(readable to owner)
    }

    val unique = linkedMapOf<String, String>()
    val ambiguous = linkedMapOf<String, List<String>>()
    for ((simple, entries) in grouped) {
        val owners = entries.map { it.second }.distinct()
        if (owners.size == 1) {
            unique[simple] = owners.single()
        } else {
            ambiguous[simple] = entries.map { it.first }.distinct()
        }
    }
    return SimpleClassNameLookup(unique, ambiguous)
}
