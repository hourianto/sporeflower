package j2me.reports

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
    inventory: MemberInventory,
): CoverageStats {
    val cmap = inventory.cmap
    val ignoredClassTotal = inventory.ignoredClassTotal
    val classSummaries = inventory.classes.map { members ->
        val declared = members.owner in cmap.classes
        CoverageClassSummary(
            owner = members.owner,
            classDeclared = declared,
            classRenamed = declared && cmap.classes[members.owner] != members.owner,
            fieldTotal = members.activeFields.size,
            fieldMapped = members.activeFields.count { it in cmap.fields },
            deadFieldTotal = members.fields.size - members.activeFields.size,
            methodTotal = members.methods.size,
            methodMapped = members.methods.count { it in cmap.methods },
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
