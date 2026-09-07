package j2me.reports

import j2me.common.internalToJava
import j2me.common.parseMethodDescriptor
import j2me.common.parseTypeDescriptor
import j2me.common.displayType
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

data class UsageReportStats(
    val symbolTotal: Int,
    val mappedTotal: Int,
    val unmappedTotal: Int,
    val deadTotal: Int,
    val methodRefTotal: Int,
    val fieldAccessTotal: Int,
    val ignoredClassTotal: Int = 0,
)

private enum class MappingStatus {
    MAPPED,
    UNMAPPED,
    DEAD,
}

private enum class UsageKind(val label: String) {
    METHOD("method"),
    FIELD("field"),
}

fun writeSymbolIndex(path: Path, symbolsByClass: Map<String, ClassSymbols>) {
    val content = buildString {
        appendLine("kind\towner\tname\tdesc")
        for (owner in symbolsByClass.keys.sorted()) {
            val symbols = symbolsByClass.getValue(owner)
            symbols.fields.sortedWith(compareBy({ it.name }, { it.desc })).forEach {
                appendLine("field\t${it.owner}\t${it.name}\t${it.desc}")
            }
            symbols.methods.sortedWith(compareBy({ it.name }, { it.desc })).forEach {
                appendLine("method\t${it.owner}\t${it.name}\t${it.desc}")
            }
        }
    }
    path.parent?.createDirectories()
    path.writeText(content)
}

internal fun simpleOwnerName(ownerInternal: String): String {
    val dotted = internalToJava(ownerInternal)
    return if (dotted.startsWith("defpackage.")) dotted.substringAfter('.') else dotted
}

private fun mappedOwnerName(ownerInternal: String, cmap: CanonicalMap): String = simpleOwnerName(cmap.classes[ownerInternal] ?: ownerInternal)

internal fun ownerWithWas(ownerInternal: String, cmap: CanonicalMap): String {
    val mapped = mappedOwnerName(ownerInternal, cmap)
    val raw = simpleOwnerName(ownerInternal)
    return if (mapped == raw) mapped else "$mapped (was $raw)"
}

private data class UsageMember(
    val owner: String,
    val ownerMapped: String,
    val nameRaw: String,
    val nameMapped: String?,
    val desc: String,
    val signature: String,
)

private sealed class UsageRow(val member: UsageMember, val status: MappingStatus) {
    abstract val kind: UsageKind
    abstract val traffic: Int
    abstract val score: Int
    val canonicalId get() = "${kind.label}:${member.owner}.${member.nameRaw} ${member.desc}"
    val isMapped get() = status == MappingStatus.MAPPED
    val isDead get() = status == MappingStatus.DEAD

    class Method(member: UsageMember, status: MappingStatus, val refCount: Int, val callerMethodCount: Int) : UsageRow(member, status) {
        override val kind = UsageKind.METHOD
        override val traffic get() = refCount
        override val score get() = refCount + 2 * callerMethodCount
    }

    class Field(member: UsageMember, status: MappingStatus, val readCount: Int, val writeCount: Int, val accessorMethodCount: Int) : UsageRow(member, status) {
        override val kind = UsageKind.FIELD
        override val traffic get() = readCount + writeCount
        override val score get() = if (isDead) 0 else traffic + 2 * accessorMethodCount
    }
}

private data class ClassUsageSummary(
    val owner: String,
    val ownerDisplay: String,
    val mappedCount: Int,
    val totalCount: Int,
    val deadCount: Int,
    val unmappedImpact: Int,
    val totalTraffic: Int,
    val peakUnmapped: Int,
    val methods: List<UsageRow.Method>,
    val fields: List<UsageRow.Field>,
) {
    val mappedPercent: Double
        get() = if (totalCount <= 0) 0.0 else (100.0 * mappedCount) / totalCount

    val unmappedRows: List<UsageRow>
        get() = (methods + fields)
            .filter { it.status == MappingStatus.UNMAPPED }
            .sortedWith(usageRowOrder)
}

private val usageRowOrder = compareBy<UsageRow>(
    { if (it.isDead) 1 else 0 },
    { -it.score },
    { -it.traffic },
    { it.member.owner },
    { it.member.nameRaw },
    { it.member.desc },
    { it.kind.label },
)

private fun renderMethodSignature(sig: MethodSig, cmap: CanonicalMap, mappedName: String?, argNames: List<String>): String {
    val (args, ret) = parseMethodDescriptor(sig.desc)
    val renderedArgs = args.mapIndexed { idx, arg ->
        val base = displayType(arg, cmap)
        val pname = argNames.getOrNull(idx)
        if (pname.isNullOrBlank()) base else "$base $pname"
    }
    val useName = mappedName ?: sig.name
    val suffix = if (mappedName != null && mappedName != sig.name) " /* was ${sig.name} */" else ""
    return "${displayType(ret, cmap)} $useName(${renderedArgs.joinToString(", ")})$suffix"
}

private fun safeDisplayFieldType(desc: String, cmap: CanonicalMap): String =
    try {
        val parsed = parseTypeDescriptor(desc)
        if (parsed.nextIndex == desc.length) displayType(parsed.typeName, cmap) else desc
    } catch (_: IllegalArgumentException) {
        desc
    }

private fun renderFieldSignature(sig: FieldSig, cmap: CanonicalMap, mappedName: String?): String {
    val useName = mappedName ?: sig.name
    val suffix = if (mappedName != null && mappedName != sig.name) " /* was ${sig.name} */" else ""
    return "${safeDisplayFieldType(sig.desc, cmap)} $useName$suffix"
}

private fun buildUsageRows(inventory: MemberInventory): List<ClassUsageSummary> {
    val cmap = inventory.cmap
    val usage = inventory.usage
    val out = mutableListOf<ClassUsageSummary>()

    for (members in inventory.classes) {
        val owner = members.owner
        val methods = mutableListOf<UsageRow.Method>()
        val fields = mutableListOf<UsageRow.Field>()

        for (method in members.methods) {
            val mappedName = cmap.methods[method]
            val refCount = usage.methodRefs[method] ?: 0
            val callerMethodCount = usage.methodCallers[method]?.size ?: 0
            methods += UsageRow.Method(
                UsageMember(owner, mappedOwnerName(owner, cmap), method.name, mappedName, method.desc,
                    renderMethodSignature(method, cmap, mappedName, cmap.methodArgs[method].orEmpty())),
                if (mappedName != null) MappingStatus.MAPPED else MappingStatus.UNMAPPED,
                refCount, callerMethodCount,
            )
        }

        for (field in members.fields) {
            val mappedName = cmap.fields[field]
            val isDead = field !in members.activeFields
            val readCount = usage.fieldReads[field] ?: 0
            val writeCount = usage.fieldWrites[field] ?: 0
            val accessorMethodCount = usage.fieldAccessors[field]?.size ?: 0
            fields += UsageRow.Field(
                UsageMember(owner, mappedOwnerName(owner, cmap), field.name, mappedName, field.desc,
                    renderFieldSignature(field, cmap, mappedName)),
                when { isDead -> MappingStatus.DEAD; mappedName != null -> MappingStatus.MAPPED; else -> MappingStatus.UNMAPPED },
                readCount, writeCount, accessorMethodCount,
            )
        }

        val sortedMethods = methods.sortedWith(usageRowOrder)
        val sortedFields = fields.sortedWith(usageRowOrder)
        val allRows = sortedMethods + sortedFields
        val activeRows = allRows.filterNot { it.isDead }
        val mappedCount = activeRows.count { it.isMapped }
        val unmappedRows = activeRows.filterNot { it.isMapped }

        out += ClassUsageSummary(
            owner = owner,
            ownerDisplay = ownerWithWas(owner, cmap),
            mappedCount = mappedCount,
            totalCount = activeRows.size,
            deadCount = allRows.count { it.isDead },
            unmappedImpact = unmappedRows.sumOf { it.score },
            totalTraffic = activeRows.sumOf { it.traffic },
            peakUnmapped = unmappedRows.maxOfOrNull { it.score } ?: 0,
            methods = sortedMethods,
            fields = sortedFields,
        )
    }

    return out.sortedWith(compareBy<ClassUsageSummary>({ -it.unmappedImpact }, { -it.peakUnmapped }, { -it.totalTraffic }, { it.owner }))
}

private fun renderUsagePriorityMarkdown(
    classSummaries: List<ClassUsageSummary>,
    ignoredClassTotal: Int,
): String = buildString {
    appendLine("Unmapped priorities")
    appendLine("===================")
    appendLine()
    appendLine("Classes are ordered by remaining impact; every active unmapped member appears once.")
    appendLine("Priority = bytecode uses + 2 x distinct caller/accessor methods. Full metrics and mapped/dead members are in usage-priority.tsv.")
    appendLine()
    if (ignoredClassTotal > 0) {
        appendLine("Ignored classes: $ignoredClassTotal (already named in bytecode, excluded from this report)")
        appendLine()
    }

    val remainingClasses = classSummaries.filter { it.unmappedRows.isNotEmpty() }
    if (remainingClasses.isEmpty()) {
        appendLine("All members are mapped.")
    } else {
        for (summary in remainingClasses) {
            appendLine("${summary.ownerDisplay} - ${summary.unmappedRows.size} unmapped; impact ${summary.unmappedImpact}")
            for (row in summary.unmappedRows) {
                val activity = when (row) {
                    is UsageRow.Method ->
                        "${counted(row.refCount, "call")}; ${counted(row.callerMethodCount, "caller")}"
                    is UsageRow.Field ->
                        "${counted(row.readCount, "read")}; ${counted(row.writeCount, "write")}; " +
                            counted(row.accessorMethodCount, "accessor")
                }
                appendLine("  ${row.kind.label} ${row.member.signature} - $activity")
            }
            appendLine()
        }
    }
}

private fun counted(value: Int, noun: String): String = "$value $noun${if (value == 1) "" else "s"}"

private fun tsvCell(value: String): String = value.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ')

private fun tsvRow(vararg cells: String): String = cells.map(::tsvCell).joinToString("\t")

private fun UsageRow.metricCells(): List<String> =
    when (this) {
        is UsageRow.Method -> listOf(refCount.toString(), callerMethodCount.toString(), ".", ".", ".")
        is UsageRow.Field -> listOf(".", ".", readCount.toString(), writeCount.toString(), accessorMethodCount.toString())
    }

private fun renderUsagePriorityTsv(classSummaries: List<ClassUsageSummary>, allRows: List<UsageRow>): String = buildString {
    val classMetricsByOwner = classSummaries.associateBy { it.owner }

    appendLine(
        tsvRow(
            "global_rank",
            "kind",
            "canonical_id",
            "owner_raw",
            "owner_mapped",
            "name_raw",
            "name_mapped",
            "desc",
            "status",
            "score",
            "ref_count",
            "caller_methods",
            "read_count",
            "write_count",
            "accessor_methods",
            "class_mapped_count",
            "class_total_count",
            "class_mapped_percent",
            "class_unmapped_impact",
            "class_total_traffic",
            "class_peak_unmapped",
            "class_dead_count",
        ),
    )

    allRows.forEachIndexed { idx, row ->
        val summary = classMetricsByOwner.getValue(row.member.owner)
        val (refCount, callerMethods, readCount, writeCount, accessorMethods) = row.metricCells()
        appendLine(
            tsvRow(
                (idx + 1).toString(),
                row.kind.label,
                row.canonicalId,
                row.member.owner,
                row.member.ownerMapped,
                row.member.nameRaw,
                row.member.nameMapped ?: ".",
                row.member.desc,
                row.status.name,
                row.score.toString(),
                refCount,
                callerMethods,
                readCount,
                writeCount,
                accessorMethods,
                summary.mappedCount.toString(),
                summary.totalCount.toString(),
                "%.2f".format(summary.mappedPercent),
                summary.unmappedImpact.toString(),
                summary.totalTraffic.toString(),
                summary.peakUnmapped.toString(),
                summary.deadCount.toString(),
            ),
        )
    }
}

fun writeUsagePriorityReport(
    markdownPath: Path,
    tsvPath: Path,
    inventory: MemberInventory,
): UsageReportStats {
    val ignoredClassTotal = inventory.ignoredClassTotal
    val classSummaries = buildUsageRows(inventory)

    val allRows = classSummaries.flatMap { it.methods + it.fields }.sortedWith(usageRowOrder)

    val activeRows = allRows.filterNot { it.isDead }
    val totalSymbols = activeRows.size
    val mappedTotal = activeRows.count { it.status == MappingStatus.MAPPED }
    val unmappedTotal = totalSymbols - mappedTotal
    val deadTotal = allRows.count { it.isDead }
    val methodRefTotal = activeRows.filterIsInstance<UsageRow.Method>().sumOf { it.refCount }
    val fieldAccessTotal = activeRows.filterIsInstance<UsageRow.Field>().sumOf { it.readCount + it.writeCount }
    markdownPath.parent?.createDirectories()
    markdownPath.writeText(renderUsagePriorityMarkdown(classSummaries, ignoredClassTotal))

    tsvPath.parent?.createDirectories()
    tsvPath.writeText(renderUsagePriorityTsv(classSummaries, allRows))

    return UsageReportStats(
        symbolTotal = totalSymbols,
        mappedTotal = mappedTotal,
        unmappedTotal = unmappedTotal,
        deadTotal = deadTotal,
        methodRefTotal = methodRefTotal,
        fieldAccessTotal = fieldAccessTotal,
        ignoredClassTotal = ignoredClassTotal,
    )
}
