package j2me.validation

import j2me.common.displayOwner
import j2me.common.formatFieldSignatureJava
import j2me.common.formatMethodSignatureJava
import j2me.common.isValidIdentifier
import j2me.common.internalPackageName
import j2me.common.mappedClassName
import j2me.common.parseMethodDescriptor
import j2me.common.primitiveTypeNames
import j2me.common.validateClassName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.MappingOrigin
import j2me.model.isConstructor
import org.objectweb.asm.Opcodes
import java.nio.file.Path
import kotlin.math.abs

data class ValidationIssue(
    val title: String,
    val notes: List<String> = emptyList(),
    val hint: String? = null,
    val origins: List<MappingOrigin> = emptyList(),
)

class MappingValidationException(
    val issues: List<ValidationIssue>,
    val mapsDir: Path? = null,
) : IllegalArgumentException(renderValidationIssues(issues, mapsDir?.toAbsolutePath()?.normalize()))

private fun formatOriginPath(origin: MappingOrigin, mapsDir: Path?): String {
    val path = origin.path
    if (mapsDir != null) {
        val normalizedPath = path.toAbsolutePath().normalize()
        if (normalizedPath.startsWith(mapsDir)) {
            val rel = mapsDir.relativize(normalizedPath)
            return "${mapsDir.fileName}/${rel.toString().replace('\\', '/')}"
        }
    }
    return path.toString().replace('\\', '/')
}

fun formatIssue(issue: ValidationIssue, mapsDir: Path?): String {
    val lines = mutableListOf("error: ${issue.title}")

    for (origin in issue.origins) {
        lines += "  --> ${formatOriginPath(origin, mapsDir)}:${origin.line}"
        if (!origin.source.isNullOrBlank()) {
            lines += "   |  ${origin.source}"
        }
    }

    if (issue.origins.isNotEmpty()) {
        lines += "   |"
    }

    for (note in issue.notes) {
        lines += "   = $note"
    }
    if (!issue.hint.isNullOrBlank()) {
        lines += "   hint: ${issue.hint}"
    }

    return lines.joinToString("\n")
}

fun renderValidationIssues(issues: List<ValidationIssue>, mapsDir: Path?): String =
    issues.joinToString("\n\n") { formatIssue(it, mapsDir) }

private fun methodArgCount(sig: MethodSig): Int = parseMethodDescriptor(sig.desc).first.size

private fun sortMethodCandidates(candidates: List<MethodSig>, wanted: MethodSig): List<MethodSig> {
    val (wantedArgs, wantedRet) = parseMethodDescriptor(wanted.desc)

    data class Candidate(val sig: MethodSig, val index: Int, val argCount: Int, val ret: String)

    return candidates
        .mapIndexed { index, sig ->
            val (args, ret) = parseMethodDescriptor(sig.desc)
            Candidate(sig, index, args.size, ret)
        }
        .sortedWith(compareBy<Candidate>(
            { if (it.argCount == wantedArgs.size) 0 else 1 },
            { abs(it.argCount - wantedArgs.size) },
            { if (it.ret == wantedRet) 0 else 1 },
            { it.sig.name },
            { it.index },
        ))
        .map { it.sig }
}

private fun <T> appendMemberCandidates(
    notes: MutableList<String>,
    ownerLabel: String,
    memberName: String,
    sameNameCandidates: List<T>,
    availableCandidates: List<T>,
    labelForList: String,
    sort: (List<T>) -> List<T>,
    formatCandidate: (T) -> String,
): Boolean {
    if (sameNameCandidates.isNotEmpty()) {
        val ordered = sort(sameNameCandidates)
        val shown = ordered.take(8)
        notes += "candidates in $ownerLabel matching name '$memberName' (shown ${shown.size}/${ordered.size}):"
        notes += shown.map { "  ${formatCandidate(it)}" }
        val hidden = ordered.size - shown.size
        if (hidden > 0) {
            notes += "  ... $hidden more $labelForList not shown"
        }
        return true
    }

    if (availableCandidates.isNotEmpty()) {
        val ordered = sort(availableCandidates)
        val shown = ordered.take(8)
        notes += "available $labelForList in $ownerLabel (shown ${shown.size}/${ordered.size}):"
        notes += shown.map { "  ${formatCandidate(it)}" }
        val hidden = ordered.size - shown.size
        if (hidden > 0) {
            notes += "  ... $hidden more $labelForList not shown"
        }
    }
    return false
}

private fun buildMissingMethodIssue(
    sig: MethodSig,
    target: String,
    origin: MappingOrigin?,
    classSymbols: ClassSymbols?,
    cmap: CanonicalMap,
): ValidationIssue {
    val ownerLabel = displayOwner(sig.owner, cmap)
    val notes = mutableListOf(
        "resolved symbol: ${formatMethodSignatureJava(sig, cmap)} -> $target",
        "raw descriptor: ${sig.name}${sig.desc}",
    )
    val sameNameMethods = classSymbols?.methods?.filter { it.name == sig.name }.orEmpty()
    val hasSameNameCandidates = appendMemberCandidates(
        notes = notes,
        ownerLabel = ownerLabel,
        memberName = sig.name,
        sameNameCandidates = sameNameMethods,
        availableCandidates = classSymbols?.methods?.filterNot { it.isConstructor() }.orEmpty(),
        labelForList = "method(s)",
        sort = { sortMethodCandidates(it, sig) },
        formatCandidate = { "${formatMethodSignatureJava(it, cmap)} [raw: ${it.name}${it.desc}]" },
    )
    val hint = if (hasSameNameCandidates) {
        val ordered = sortMethodCandidates(sameNameMethods, sig)
        val desiredCount = methodArgCount(sig)
        val closestCount = methodArgCount(ordered.first())
        if (closestCount != desiredCount) {
            "signature has $desiredCount parameters but closest match has $closestCount"
        } else {
            "parameter/return types do not match existing overloads"
        }
    } else {
        "no methods named '${sig.name}' exist in class $ownerLabel; verify `/* was ... */`"
    }

    return ValidationIssue(
        title = "method not found in class $ownerLabel",
        notes = notes,
        hint = hint,
        origins = listOfNotNull(origin),
    )
}

private fun buildMissingFieldIssue(
    sig: FieldSig,
    target: String,
    origin: MappingOrigin?,
    classSymbols: ClassSymbols?,
    cmap: CanonicalMap,
): ValidationIssue {
    val ownerLabel = displayOwner(sig.owner, cmap)
    val notes = mutableListOf(
        "resolved field: ${formatFieldSignatureJava(sig, cmap)} -> $target",
        "raw field descriptor: ${sig.name}:${sig.desc}",
    )

    val sameNameFields = classSymbols?.fields?.filter { it.name == sig.name }.orEmpty()
    val hasSameNameCandidates = appendMemberCandidates(
        notes = notes,
        ownerLabel = ownerLabel,
        memberName = sig.name,
        sameNameCandidates = sameNameFields,
        availableCandidates = classSymbols?.fields.orEmpty(),
        labelForList = "field(s)",
        sort = { it.sortedWith(compareBy({ field -> field.name }, { field -> field.desc })) },
        formatCandidate = { "${formatFieldSignatureJava(it, cmap)} [raw: ${it.name}:${it.desc}]" },
    )
    val hint = if (hasSameNameCandidates) {
        "check the `/* was ... */` name and declared field type"
    } else {
        "no fields named '${sig.name}' exist in class $ownerLabel; verify `/* was ... */`"
    }

    return ValidationIssue(
        title = "field not found in class $ownerLabel",
        notes = notes,
        hint = hint,
        origins = listOfNotNull(origin),
    )
}

private data class MethodMappingSurface(
    val sig: MethodSig,
    val targetName: String,
    val access: Int,
    val parameterDescriptor: String,
    val isMapped: Boolean,
)

private data class FieldMappingSurface(
    val sig: FieldSig,
    val targetName: String,
)

private fun isJavaVirtualMethod(surface: MethodMappingSurface): Boolean {
    if (surface.sig.isConstructor() || (surface.access and Opcodes.ACC_STATIC) != 0) {
        return false
    }
    if ((surface.access and Opcodes.ACC_PRIVATE) != 0) {
        return false
    }
    return true
}

private fun isInheritedMemberVisible(
    declaringOwner: String,
    visibleFrom: String,
    access: Int,
    cmap: CanonicalMap,
): Boolean {
    if ((access and Opcodes.ACC_PRIVATE) != 0) return false
    if ((access and (Opcodes.ACC_PUBLIC or Opcodes.ACC_PROTECTED)) != 0) return true
    return internalPackageName(mappedClassName(declaringOwner, cmap)) ==
        internalPackageName(mappedClassName(visibleFrom, cmap))
}

private fun directParents(symbols: ClassSymbols): List<String> = buildList {
    symbols.superName?.let { add(it) }
    addAll(symbols.interfaces)
}

private fun collectHierarchyOwners(
    owner: String,
    symbolsByClass: Map<String, ClassSymbols>,
): List<String> {
    val seen = mutableSetOf<String>()
    val owners = mutableListOf<String>()

    fun visit(current: String) {
        if (!seen.add(current)) {
            return
        }
        owners += current
        symbolsByClass[current]?.let { directParents(it).forEach(::visit) }
    }

    visit(owner)
    return owners
}

private fun methodParameterDescriptor(desc: String): String = desc.substring(0, desc.indexOf(')') + 1)

private fun isTypeSubtype(child: String, parent: String, symbolsByClass: Map<String, ClassSymbols>): Boolean {
    if (child == parent) return true
    if (child in primitiveTypeNames || parent in primitiveTypeNames) return false

    if (child.endsWith("[]")) {
        if (parent == "java/lang/Object" || parent == "java/lang/Cloneable" || parent == "java/io/Serializable") {
            return true
        }
        return parent.endsWith("[]") && isTypeSubtype(child.removeSuffix("[]"), parent.removeSuffix("[]"), symbolsByClass)
    }
    if (parent.endsWith("[]")) return false
    if (parent == "java/lang/Object") return true

    val seen = mutableSetOf<String>()
    fun visit(current: String): Boolean {
        if (!seen.add(current)) return false
        if (current == parent) return true
        val symbols = symbolsByClass[current] ?: return false
        return directParents(symbols).any(::visit)
    }
    return visit(child)
}

private fun haveCompatibleReturns(
    first: MethodMappingSurface,
    second: MethodMappingSurface,
    symbolsByClass: Map<String, ClassSymbols>,
): Boolean {
    val firstReturn = parseMethodDescriptor(first.sig.desc).second
    val secondReturn = parseMethodDescriptor(second.sig.desc).second
    return isTypeSubtype(firstReturn, secondReturn, symbolsByClass) ||
        isTypeSubtype(secondReturn, firstReturn, symbolsByClass)
}

private fun methodPairKey(first: MethodSig, second: MethodSig): String =
    listOf(
        "${first.owner}.${first.name}${first.desc}",
        "${second.owner}.${second.name}${second.desc}",
    ).sorted().joinToString("|")

private fun fieldPairKey(first: FieldSig, second: FieldSig): String =
    listOf(
        "${first.owner}.${first.name}:${first.desc}",
        "${second.owner}.${second.name}:${second.desc}",
    ).sorted().joinToString("|")

private fun buildSourceOverrideIssue(
    owner: String,
    method: MethodMappingSurface,
    inherited: MethodMappingSurface,
    cmap: CanonicalMap,
): ValidationIssue {
    val sameBytecodeOverride = method.sig.name == inherited.sig.name &&
        method.parameterDescriptor == inherited.parameterDescriptor
    val title = if (sameBytecodeOverride) {
        "method override family renamed inconsistently in class ${displayOwner(owner, cmap)}"
    } else {
        "method rename creates new source override in class ${displayOwner(owner, cmap)}"
    }
    val hint = if (sameBytecodeOverride) {
        "use the same target name for every method in the bytecode override family"
    } else {
        "choose a distinct target name so unrelated bytecode methods do not become a Java override"
    }

    return ValidationIssue(
        title = title,
        notes = listOf(
            "subclass method: ${method.sig.owner}.${method.sig.name}${method.sig.desc} -> ${method.targetName}",
            "inherited method: ${inherited.sig.owner}.${inherited.sig.name}${inherited.sig.desc} -> ${inherited.targetName}",
        ),
        hint = hint,
        origins = listOfNotNull(
            cmap.methodOrigins[method.sig],
            cmap.methodOrigins[inherited.sig],
        ),
    )
}

private fun buildMethodCollisionIssue(
    visibleFrom: String,
    first: MethodMappingSurface,
    second: MethodMappingSurface,
    cmap: CanonicalMap,
): ValidationIssue {
    val sameOwner = first.sig.owner == second.sig.owner
    val renderedParams = first.parameterDescriptor
    val renderedSignature = if (first.sig.desc == second.sig.desc) {
        "${first.targetName}${first.sig.desc}"
    } else {
        "${first.targetName}$renderedParams in Java source (return type is not part of a Java method signature)"
    }
    return ValidationIssue(
        title = if (sameOwner) {
            "method name collision in class ${displayOwner(first.sig.owner, cmap)}"
        } else {
            "method name collision across hierarchy in class ${displayOwner(visibleFrom, cmap)}"
        },
        notes = if (sameOwner) {
            listOf(
                "both ${first.sig.name}${first.sig.desc} and ${second.sig.name}${second.sig.desc} " +
                    "map to '$renderedSignature'",
            )
        } else {
            listOf(
                "visible method: ${first.sig.owner}.${first.sig.name}${first.sig.desc} -> ${first.targetName}",
                "visible method: ${second.sig.owner}.${second.sig.name}${second.sig.desc} -> ${second.targetName}",
                "both render as '${first.targetName}$renderedParams'",
            )
        },
        hint = if (sameOwner) {
            "rename one of the mapped methods to resolve the collision"
        } else {
            "choose distinct target names so Vineflower can preserve both mappings"
        },
        origins = listOfNotNull(
            cmap.methodOrigins[first.sig],
            cmap.methodOrigins[second.sig],
        ),
    )
}

private fun buildFieldCollisionIssue(
    visibleFrom: String,
    first: FieldMappingSurface,
    second: FieldMappingSurface,
    cmap: CanonicalMap,
): ValidationIssue {
    val sameOwner = first.sig.owner == second.sig.owner
    return ValidationIssue(
        title = if (sameOwner) {
            "field name collision in class ${displayOwner(first.sig.owner, cmap)}"
        } else {
            "field name collision across hierarchy in class ${displayOwner(visibleFrom, cmap)}"
        },
        notes = if (sameOwner) {
            listOf("both ${first.sig.name}:${first.sig.desc} and ${second.sig.name}:${second.sig.desc} map to '${first.targetName}'")
        } else {
            listOf(
                "visible field: ${first.sig.owner}.${first.sig.name}:${first.sig.desc} -> ${first.targetName}",
                "visible field: ${second.sig.owner}.${second.sig.name}:${second.sig.desc} -> ${second.targetName}",
            )
        },
        hint = if (sameOwner) {
            "rename one of the mapped fields to resolve the collision"
        } else {
            "choose distinct target names so Vineflower can preserve both mappings"
        },
        origins = listOfNotNull(
            cmap.fieldOrigins[first.sig],
            cmap.fieldOrigins[second.sig],
        ),
    )
}

private fun validateSourceMemberSurface(
    symbolsByClass: Map<String, ClassSymbols>,
    cmap: CanonicalMap,
    classpathSymbolsByClass: Map<String, ClassSymbols>,
): List<ValidationIssue> {
    val issues = mutableListOf<ValidationIssue>()
    val inheritanceSymbolsByClass = classpathSymbolsByClass + symbolsByClass
    val seenMethodPairs = mutableSetOf<String>()
    val seenFieldPairs = mutableSetOf<String>()

    for (owner in symbolsByClass.keys.sorted()) {
        val hierarchy = collectHierarchyOwners(owner, inheritanceSymbolsByClass)
        val methods = hierarchy.flatMapIndexed { ownerIndex, methodOwner ->
            val methodSymbols = inheritanceSymbolsByClass[methodOwner] ?: return@flatMapIndexed emptyList()
            methodSymbols.methods
                .filterNot { it.isConstructor() || methodSymbols.isGeneratedMethod(it) }
                .filter { method ->
                    ownerIndex == 0 || isInheritedMemberVisible(
                        declaringOwner = methodOwner,
                        visibleFrom = owner,
                        access = methodSymbols.methodAccess[method] ?: 0,
                        cmap = cmap,
                    )
                }
                .map {
                    MethodMappingSurface(
                        sig = it,
                        targetName = cmap.methods[it] ?: it.name,
                        access = methodSymbols.methodAccess[it] ?: 0,
                        parameterDescriptor = methodParameterDescriptor(it.desc),
                        isMapped = it in cmap.methods,
                    )
                }
        }

        for (firstIndex in methods.indices) {
            val first = methods[firstIndex]
            for (secondIndex in firstIndex + 1 until methods.size) {
                val second = methods[secondIndex]
                if (!first.isMapped && !second.isMapped) continue
                if (first.parameterDescriptor != second.parameterDescriptor) continue

                val pairKey = methodPairKey(first.sig, second.sig)
                if (pairKey in seenMethodPairs) continue

                val rawOverrideFamily = first.sig.owner != second.sig.owner &&
                    first.sig.name == second.sig.name &&
                    isJavaVirtualMethod(first) &&
                    isJavaVirtualMethod(second) &&
                    haveCompatibleReturns(first, second, inheritanceSymbolsByClass)
                if (rawOverrideFamily) {
                    if (first.targetName != second.targetName && seenMethodPairs.add(pairKey)) {
                        issues += buildSourceOverrideIssue(owner, first, second, cmap)
                    }
                    continue
                }

                if (first.targetName == second.targetName && seenMethodPairs.add(pairKey)) {
                    val createsVirtualOverride = first.sig.owner != second.sig.owner &&
                        isJavaVirtualMethod(first) &&
                        isJavaVirtualMethod(second) &&
                        haveCompatibleReturns(first, second, inheritanceSymbolsByClass)
                    issues += if (createsVirtualOverride) {
                        buildSourceOverrideIssue(owner, first, second, cmap)
                    } else {
                        buildMethodCollisionIssue(owner, first, second, cmap)
                    }
                }
            }
        }

        val fields = hierarchy.flatMapIndexed { ownerIndex, fieldOwner ->
            val fieldSymbols = inheritanceSymbolsByClass[fieldOwner] ?: return@flatMapIndexed emptyList()
            fieldSymbols.fields
                .filterNot { fieldSymbols.isGeneratedField(it) }
                .filter { field ->
                    ownerIndex == 0 || isInheritedMemberVisible(
                        declaringOwner = fieldOwner,
                        visibleFrom = owner,
                        access = fieldSymbols.fieldAccess[field] ?: 0,
                        cmap = cmap,
                    )
                }
                .map { field ->
                    FieldMappingSurface(field, cmap.fields[field] ?: field.name)
                }
        }
        for (firstIndex in fields.indices) {
            val first = fields[firstIndex]
            for (secondIndex in firstIndex + 1 until fields.size) {
                val second = fields[secondIndex]
                if (first.targetName != second.targetName) continue
                if (first.sig !in cmap.fields && second.sig !in cmap.fields) continue
                if (seenFieldPairs.add(fieldPairKey(first.sig, second.sig))) {
                    issues += buildFieldCollisionIssue(owner, first, second, cmap)
                }
            }
        }
    }

    return issues
}

fun validateMap(
    symbolsByClass: Map<String, ClassSymbols>,
    cmap: CanonicalMap,
    mapsDir: Path? = null,
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
) {
    val issues = mutableListOf<ValidationIssue>()

    val existingClasses = symbolsByClass.keys
    for (owner in cmap.ignoredClasses.sorted()) {
        if (owner !in existingClasses) {
            issues += ValidationIssue(title = "@AlreadyMapped class not found in jar: $owner")
        }
        if (owner in cmap.classes) {
            issues += ValidationIssue(
                title = "class cannot be both @AlreadyMapped and remapped: $owner",
                notes = listOf("remapped target: ${cmap.classes.getValue(owner)}"),
                hint = "remove either @AlreadyMapped or the class rename",
            )
        }
    }

    for ((old, new) in cmap.classes.toSortedMap()) {
        if (old !in existingClasses) {
            issues += ValidationIssue(title = "class not found in jar: $old")
            continue
        }
        if (!validateClassName(new)) {
            issues += ValidationIssue(
                title = "invalid class target name: $old -> $new",
                hint = "target class name must be a valid Java/internal class identifier",
            )
        }
    }

    val finalClassNames = mutableMapOf<String, String>()
    for (old in existingClasses) {
        val target = mappedClassName(old, cmap)
        val prev = finalClassNames[target]
        if (prev != null && prev != old) {
            issues += ValidationIssue(
                title = "class name collision: $prev and $old",
                notes = listOf("both map to '$target'"),
                hint = "choose distinct target class names",
            )
        }
        finalClassNames[target] = old
    }

    val existingFields = symbolsByClass.values.flatMap { it.fields }.toSet()
    val existingMethods = symbolsByClass.values.flatMap { it.methods }.toSet()

    for ((sig, target) in cmap.fields.entries.sortedWith(compareBy({ it.key.owner }, { it.key.name }, { it.key.desc }))) {
        val origin = cmap.fieldOrigins[sig]
        if (sig !in existingFields) {
            issues += buildMissingFieldIssue(sig, target, origin, symbolsByClass[sig.owner], cmap)
        }
        if (!isValidIdentifier(target)) {
            issues += ValidationIssue(
                title = "invalid field target name in class ${displayOwner(sig.owner, cmap)}",
                notes = listOf("mapped field: ${sig.name}:${sig.desc} -> $target"),
                hint = "target field name must be a valid Java identifier",
                origins = listOfNotNull(origin),
            )
        }
    }

    for ((sig, target) in cmap.methods.entries.sortedWith(compareBy({ it.key.owner }, { it.key.name }, { it.key.desc }))) {
        val origin = cmap.methodOrigins[sig]
        if (sig !in existingMethods) {
            issues += buildMissingMethodIssue(sig, target, origin, symbolsByClass[sig.owner], cmap)
        }
        if (if (sig.isConstructor()) target != "<init>" else !isValidIdentifier(target)) {
            issues += ValidationIssue(
                title = "invalid method target name in class ${displayOwner(sig.owner, cmap)}",
                notes = listOf("mapped method: ${sig.name}${sig.desc} -> $target"),
                hint = "target method name must be a valid Java identifier",
                origins = listOfNotNull(origin),
            )
        }

        val params = cmap.methodArgs[sig]
        if (params != null) {
            val (argTypes, _) = parseMethodDescriptor(sig.desc)
            if (params.size != argTypes.size) {
                issues += ValidationIssue(
                    title = "parameter count mismatch for ${sig.owner}.${sig.name}${sig.desc}",
                    notes = listOf("expected ${argTypes.size}, got ${params.size}"),
                    hint = "parameter names must match the method descriptor arity",
                    origins = listOfNotNull(origin),
                )
            }
            params.forEachIndexed { idx, paramName ->
                if (!isValidIdentifier(paramName)) {
                    issues += ValidationIssue(
                        title = "invalid parameter target name in ${sig.owner}.${sig.name}${sig.desc}",
                        notes = listOf("index $idx: $paramName"),
                        hint = "parameter names must be valid Java identifiers",
                        origins = listOfNotNull(origin),
                    )
                }
            }
        }
    }

    issues += validateSourceMemberSurface(symbolsByClass, cmap, classpathSymbolsByClass)

    if (issues.isNotEmpty()) {
        throw MappingValidationException(issues, mapsDir)
    }
}

fun printUserFacingError(exc: Exception) {
    when (exc) {
        is MappingValidationException -> {
            System.err.println("Mapping/configuration validation failed:")
            val rendered = renderValidationIssues(exc.issues, exc.mapsDir)
            if (rendered.isNotBlank()) {
                System.err.println(rendered)
            }
            System.err.println("Fix the listed entries and run the command again.")
        }

        is IllegalArgumentException -> printCommandFailure(exc)

        else -> {
            printCommandFailure(exc)
            System.err.println()
            exc.printStackTrace(System.err)
        }
    }
}

private fun printCommandFailure(exc: Exception) {
    val message = exc.message?.trim().orEmpty().ifBlank { exc::class.simpleName ?: "Error" }
    val lines = message.lines().map { it.trimEnd() }.filter { it.isNotBlank() }
    System.err.println("Command failed:")
    if (lines.size <= 1) {
        lines.forEach { System.err.println("- $it") }
    } else {
        lines.forEach(System.err::println)
    }
}
