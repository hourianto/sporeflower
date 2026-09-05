package j2me.reports

import j2me.common.internalToJava
import j2me.common.parseMethodDescriptor
import j2me.common.parseTypeDescriptor
import j2me.common.displayType
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.isConstructor
import j2me.symbols.UsageStats
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

private sealed class UsageRow(
    val kind: UsageKind,
    val owner: String,
    val ownerMapped: String,
    val nameRaw: String,
    val nameMapped: String?,
    val desc: String,
    val status: MappingStatus,
    val signature: String,
    val score: Int,
) {
    val canonicalId: String
        get() = "${kind.label}:$owner.$nameRaw $desc"

    abstract val traffic: Int

    val isMapped: Boolean
        get() = status == MappingStatus.MAPPED

    val isDead: Boolean
        get() = status == MappingStatus.DEAD

    class Method(
        owner: String,
        ownerMapped: String,
        nameRaw: String,
        nameMapped: String?,
        desc: String,
        status: MappingStatus,
        signature: String,
        score: Int,
        val refCount: Int,
        val callerMethodCount: Int,
    ) : UsageRow(
        kind = UsageKind.METHOD,
        owner = owner,
        ownerMapped = ownerMapped,
        nameRaw = nameRaw,
        nameMapped = nameMapped,
        desc = desc,
        status = status,
        signature = signature,
        score = score,
    ) {
        override val traffic: Int
            get() = refCount
    }

    class Field(
        owner: String,
        ownerMapped: String,
        nameRaw: String,
        nameMapped: String?,
        desc: String,
        status: MappingStatus,
        signature: String,
        score: Int,
        val readCount: Int,
        val writeCount: Int,
        val accessorMethodCount: Int,
    ) : UsageRow(
        kind = UsageKind.FIELD,
        owner = owner,
        ownerMapped = ownerMapped,
        nameRaw = nameRaw,
        nameMapped = nameMapped,
        desc = desc,
        status = status,
        signature = signature,
        score = score,
    ) {
        override val traffic: Int
            get() = readCount + writeCount
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
    { it.owner },
    { it.nameRaw },
    { it.desc },
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

internal fun isRenameRelevantField(field: FieldSig, usage: UsageStats): Boolean =
    (usage.fieldReads[field] ?: 0) > 0

private fun buildUsageRows(
    symbolsByClass: Map<String, ClassSymbols>,
    cmap: CanonicalMap,
    usage: UsageStats,
): List<ClassUsageSummary> {
    val out = mutableListOf<ClassUsageSummary>()

    for (owner in symbolsByClass.keys.sorted()) {
        if (owner in cmap.ignoredClasses) {
            continue
        }
        val classSymbols = symbolsByClass.getValue(owner)
        val methods = mutableListOf<UsageRow.Method>()
        val fields = mutableListOf<UsageRow.Field>()

        for (method in classSymbols.methods) {
            if (method.isConstructor() || classSymbols.isGeneratedMethod(method)) {
                continue
            }
            val mappedName = cmap.methods[method]
            val refCount = usage.methodRefs[method] ?: 0
            val callerMethodCount = usage.methodCallers[method]?.size ?: 0
            val score = refCount + (2 * callerMethodCount)
            methods += UsageRow.Method(
                owner = owner,
                ownerMapped = mappedOwnerName(owner, cmap),
                nameRaw = method.name,
                nameMapped = mappedName,
                desc = method.desc,
                status = if (mappedName != null) MappingStatus.MAPPED else MappingStatus.UNMAPPED,
                signature = renderMethodSignature(method, cmap, mappedName, cmap.methodArgs[method].orEmpty()),
                score = score,
                refCount = refCount,
                callerMethodCount = callerMethodCount,
            )
        }

        for (field in classSymbols.fields) {
            if (classSymbols.isGeneratedField(field)) {
                continue
            }
            val mappedName = cmap.fields[field]
            val isDead = !isRenameRelevantField(field, usage)
            val readCount = usage.fieldReads[field] ?: 0
            val writeCount = usage.fieldWrites[field] ?: 0
            val accessorMethodCount = usage.fieldAccessors[field]?.size ?: 0
            val score = if (isDead) 0 else readCount + writeCount + (2 * accessorMethodCount)
            fields += UsageRow.Field(
                owner = owner,
                ownerMapped = mappedOwnerName(owner, cmap),
                nameRaw = field.name,
                nameMapped = mappedName,
                desc = field.desc,
                status = when {
                    isDead -> MappingStatus.DEAD
                    mappedName != null -> MappingStatus.MAPPED
                    else -> MappingStatus.UNMAPPED
                },
                signature = renderFieldSignature(field, cmap, mappedName),
                score = score,
                readCount = readCount,
                writeCount = writeCount,
                accessorMethodCount = accessorMethodCount,
            )
        }

        val sortedMethods = methods.sortedWith(compareBy<UsageRow.Method>({ -it.score }, { -it.refCount }, { it.owner }, { it.nameRaw }, { it.desc }))
        val sortedFields = fields.sortedWith(compareBy<UsageRow.Field>({ if (it.isDead) 1 else 0 }, { -it.score }, { -it.traffic }, { it.owner }, { it.nameRaw }, { it.desc }))
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
                appendLine("  ${row.kind.label} ${row.signature} - $activity")
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
        val summary = classMetricsByOwner.getValue(row.owner)
        val (refCount, callerMethods, readCount, writeCount, accessorMethods) = row.metricCells()
        appendLine(
            tsvRow(
                (idx + 1).toString(),
                row.kind.label,
                row.canonicalId,
                row.owner,
                row.ownerMapped,
                row.nameRaw,
                row.nameMapped ?: ".",
                row.desc,
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
    symbolsByClass: Map<String, ClassSymbols>,
    cmap: CanonicalMap,
    usage: UsageStats,
): UsageReportStats {
    val ignoredClassTotal = symbolsByClass.keys.count { it in cmap.ignoredClasses }
    val classSummaries = buildUsageRows(
        symbolsByClass,
        cmap,
        usage,
    )

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
