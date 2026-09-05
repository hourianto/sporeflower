package j2me.reports

import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.isConstructor
import j2me.symbols.UsageStats
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

data class CoverageStats(
    val classTotal: Int,
    val classDeclared: Int,
    val classRenamed: Int,
    val fieldTotal: Int,
    val fieldMapped: Int,
    val deadFieldTotal: Int,
    val methodTotal: Int,
    val methodMapped: Int,
    val memberTotal: Int,
    val memberMapped: Int,
    val ignoredClassTotal: Int = 0,
)

private data class CoverageClassSummary(
    val owner: String,
    val classDeclared: Boolean,
    val classRenamed: Boolean,
    val fieldTotal: Int,
    val fieldMapped: Int,
    val deadFieldTotal: Int,
    val methodTotal: Int,
    val methodMapped: Int,
) {
    val memberTotal: Int
        get() = fieldTotal + methodTotal

    val memberMapped: Int
        get() = fieldMapped + methodMapped
}

fun writeCoverageReport(
    path: Path,
    symbolsByClass: Map<String, ClassSymbols>,
    cmap: CanonicalMap,
    usage: UsageStats,
): CoverageStats {
    val ignoredClassTotal = symbolsByClass.keys.count { it in cmap.ignoredClasses }
    val classSummaries = symbolsByClass.keys
        .filterNot { it in cmap.ignoredClasses }
        .sorted()
        .map { owner ->
            val classSymbols = symbolsByClass.getValue(owner)
            val ownerFields = classSymbols.fields.filterNot { classSymbols.isGeneratedField(it) }
            val activeOwnerFields = ownerFields.filter { field -> isRenameRelevantField(field, usage) }
            val ownerMethods = classSymbols.methods.filterNot { it.isConstructor() || classSymbols.isGeneratedMethod(it) }

            val ownerFieldTotal = activeOwnerFields.size
            val ownerMethodTotal = ownerMethods.size
            val ownerFieldMapped = activeOwnerFields.count { it in cmap.fields }
            val ownerMethodMapped = ownerMethods.count { it in cmap.methods }
            val declared = owner in cmap.classes
            CoverageClassSummary(
                owner = owner,
                classDeclared = declared,
                classRenamed = declared && cmap.classes[owner] != owner,
                fieldTotal = ownerFieldTotal,
                fieldMapped = ownerFieldMapped,
                deadFieldTotal = ownerFields.size - activeOwnerFields.size,
                methodTotal = ownerMethodTotal,
                methodMapped = ownerMethodMapped,
            )
        }

    val classTotal = classSummaries.size
    val classDeclared = classSummaries.count { it.classDeclared }
    val classRenamed = classSummaries.count { it.classRenamed }
    val fieldTotal = classSummaries.sumOf { it.fieldTotal }
    val fieldMapped = classSummaries.sumOf { it.fieldMapped }
    val deadFieldTotal = classSummaries.sumOf { it.deadFieldTotal }
    val methodTotal = classSummaries.sumOf { it.methodTotal }
    val methodMapped = classSummaries.sumOf { it.methodMapped }
    val memberTotal = fieldTotal + methodTotal
    val memberMapped = fieldMapped + methodMapped

    val fullyMapped = classSummaries
        .filter { it.classDeclared && it.memberMapped == it.memberTotal }
        .sortedBy { ownerWithWas(it.owner, cmap) }

    val classMappedWithUnmappedMembers = classSummaries
        .filter { it.classDeclared && it.memberMapped < it.memberTotal }
        .sortedWith(compareBy<CoverageClassSummary>({ -(it.memberTotal - it.memberMapped) }, { ownerWithWas(it.owner, cmap) }))

    val unmappedClasses = classSummaries
        .filterNot { it.classDeclared }
        .sortedWith(compareBy<CoverageClassSummary>({ -it.memberTotal }, { simpleOwnerName(it.owner) }))

    val content = buildString {
        appendLine("Remap Coverage")
        appendLine("==============")
        appendLine()
        appendLine("Classes: $classDeclared/$classTotal mapped (${classTotal - classDeclared} remaining)")
        appendLine("Members: $memberMapped/$memberTotal mapped (${memberTotal - memberMapped} remaining)")
        if (deadFieldTotal > 0) {
            appendLine("Dead fields: $deadFieldTotal (zero bytecode reads, excluded from member coverage)")
        }
        if (ignoredClassTotal > 0) {
            appendLine("Ignored classes: $ignoredClassTotal (already named in bytecode, excluded from totals)")
        }
        appendLine()

        appendLine("Fully mapped (${fullyMapped.size})")
        appendLine("-----------------")
        if (fullyMapped.isEmpty()) {
            appendLine("(none)")
        } else {
            appendLine(fullyMapped.joinToString(", ") { ownerWithWas(it.owner, cmap) })
        }
        appendLine()

        appendLine("Unmapped members in mapped classes")
        appendLine("----------------------------------")
        if (classMappedWithUnmappedMembers.isEmpty()) {
            appendLine("(none)")
        } else {
            for (summary in classMappedWithUnmappedMembers) {
                val fieldsUnmapped = summary.fieldTotal - summary.fieldMapped
                val methodsUnmapped = summary.methodTotal - summary.methodMapped
                appendLine(
                    "%-28s %4d members (%df %dm)".format(
                        ownerWithWas(summary.owner, cmap),
                        fieldsUnmapped + methodsUnmapped,
                        fieldsUnmapped,
                        methodsUnmapped,
                    ),
                )
            }
        }
        appendLine()

        appendLine("Unmapped classes")
        appendLine("----------------")
        if (unmappedClasses.isEmpty()) {
            appendLine("(none)")
        } else {
            for (summary in unmappedClasses) {
                val rawName = simpleOwnerName(summary.owner)
                appendLine(
                    "%-10s %4d members (%df %dm%s)".format(
                        rawName,
                        summary.memberTotal,
                        summary.fieldTotal,
                        summary.methodTotal,
                        if (summary.deadFieldTotal > 0) ", ${summary.deadFieldTotal} dead fields" else "",
                    ),
                )
            }
        }
    }
    path.parent?.createDirectories()
    path.writeText(content)

    return CoverageStats(
        classTotal = classTotal,
        classDeclared = classDeclared,
        classRenamed = classRenamed,
        fieldTotal = fieldTotal,
        fieldMapped = fieldMapped,
        deadFieldTotal = deadFieldTotal,
        methodTotal = methodTotal,
        methodMapped = methodMapped,
        memberTotal = memberTotal,
        memberMapped = memberMapped,
        ignoredClassTotal = ignoredClassTotal,
    )
}
