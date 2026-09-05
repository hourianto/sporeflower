package j2me.reports

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.symbols.UsageStats
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.readText

private fun readTsvRows(path: Path): List<Map<String, String>> {
    val lines = path.readText().lineSequence().filter { it.isNotBlank() }.toList()
    val header = lines.first().split('\t')
    return lines.drop(1).map { line -> header.zip(line.split('\t')).toMap() }
}

class ReportsTest : FunSpec({
    test("coverage and priorities exclude generated members") {
        val outDir = Files.createTempDirectory("reports-generated-methods")
        val sourceMethod = MethodSig("a", "a", "()V")
        val bridgeMethod = MethodSig("a", "b", "()V")
        val sourceField = FieldSig("a", "x", "I")
        val syntheticField = FieldSig("a", "y", "I")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = listOf(sourceField, syntheticField),
                methods = listOf(sourceMethod, bridgeMethod),
                methodAccess = mapOf(bridgeMethod to (Opcodes.ACC_BRIDGE or Opcodes.ACC_SYNTHETIC)),
                fieldAccess = mapOf(syntheticField to Opcodes.ACC_SYNTHETIC),
            ),
        )
        val usage = UsageStats(
            methodRefs = mapOf(sourceMethod to 1, bridgeMethod to 100),
            fieldReads = mapOf(sourceField to 1, syntheticField to 100),
        )

        val coverage = writeCoverageReport(outDir.resolve("coverage.md"), symbolsByClass, CanonicalMap(), usage)
        val priorities = writeUsagePriorityReport(
            outDir.resolve("usage-priority.md"),
            outDir.resolve("usage-priority.tsv"),
            symbolsByClass,
            CanonicalMap(),
            usage,
        )

        coverage.methodTotal shouldBe 1
        coverage.fieldTotal shouldBe 1
        priorities.symbolTotal shouldBe 2
        readTsvRows(outDir.resolve("usage-priority.tsv")).map { it.getValue("name_raw") }.toSet() shouldBe setOf("a", "x")
    }

    test("writeCoverageReport counts methods named like owner when not constructors") {
        val outDir = Files.createTempDirectory("coverage-test")
        val coveragePath = outDir.resolve("coverage.md")

        val realMethodNamedLikeOwner = MethodSig("a", "a", "()V")
        val constructor = MethodSig("a", "<init>", "()V")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(realMethodNamedLikeOwner, constructor),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(
            classes = mapOf("a" to "Entity"),
            methods = mapOf(realMethodNamedLikeOwner to "renamedA"),
        )

        val stats = writeCoverageReport(coveragePath, symbolsByClass, cmap, UsageStats())
        stats.methodTotal shouldBe 1
        stats.methodMapped shouldBe 1
    }

    test("writeCoverageReport excludes zero-read fields from coverage totals") {
        val outDir = Files.createTempDirectory("coverage-dead-fields")
        val coveragePath = outDir.resolve("coverage.md")

        val activeField = FieldSig("a", "x", "I")
        val deadField = FieldSig("a", "y", "I")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = listOf(activeField, deadField),
                methods = emptyList(),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(classes = mapOf("a" to "Entity"), fields = mapOf(activeField to "counter"))
        val usage = UsageStats(
            fieldReads = mapOf(activeField to 2),
            fieldWrites = mapOf(activeField to 1, deadField to 1),
            fieldAccessors = mapOf(activeField to setOf("a"), deadField to setOf("a")),
        )

        val stats = writeCoverageReport(coveragePath, symbolsByClass, cmap, usage)
        stats.fieldTotal shouldBe 1
        stats.fieldMapped shouldBe 1
        stats.deadFieldTotal shouldBe 1
        stats.memberTotal shouldBe 1
        stats.memberMapped shouldBe 1
    }

    test("writeCoverageReport excludes ignored already-mapped classes from totals") {
        val outDir = Files.createTempDirectory("coverage-ignored-classes")
        val coveragePath = outDir.resolve("coverage.md")

        val ignoredMethod = MethodSig("game/ui/SettingsScreen", "paint", "()V")
        val obfMethod = MethodSig("a", "b", "()V")
        val symbolsByClass = mapOf(
            "game/ui/SettingsScreen" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(ignoredMethod),
                methodAccess = emptyMap(),
            ),
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(obfMethod),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(ignoredClasses = setOf("game/ui/SettingsScreen"))

        val stats = writeCoverageReport(coveragePath, symbolsByClass, cmap, UsageStats())

        stats.classTotal shouldBe 1
        stats.ignoredClassTotal shouldBe 1
        stats.methodTotal shouldBe 1
        stats.methodMapped shouldBe 0
    }

    test("writeUsagePriorityReport excludes ignored already-mapped classes") {
        val outDir = Files.createTempDirectory("usage-ignored-classes")
        val markdownPath = outDir.resolve("usage-priority.md")
        val tsvPath = outDir.resolve("usage-priority.tsv")

        val ignoredMethod = MethodSig("game/ui/SettingsScreen", "paint", "()V")
        val obfMethod = MethodSig("a", "b", "()V")
        val symbolsByClass = mapOf(
            "game/ui/SettingsScreen" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(ignoredMethod),
                methodAccess = emptyMap(),
            ),
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(obfMethod),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(ignoredClasses = setOf("game/ui/SettingsScreen"))

        val stats = writeUsagePriorityReport(
            markdownPath = markdownPath,
            tsvPath = tsvPath,
            symbolsByClass = symbolsByClass,
            cmap = cmap,
            usage = UsageStats(methodRefs = mapOf(ignoredMethod to 100, obfMethod to 1)),
        )

        stats.ignoredClassTotal shouldBe 1
        stats.symbolTotal shouldBe 1
        stats.unmappedTotal shouldBe 1

        readTsvRows(tsvPath).map { it.getValue("owner_raw") } shouldBe listOf("a")
    }

    test("writeUsagePriorityReport keeps zero-read fields as dead lowest priority") {
        val outDir = Files.createTempDirectory("usage-dead-fields")
        val markdownPath = outDir.resolve("usage-priority.md")
        val tsvPath = outDir.resolve("usage-priority.tsv")

        val activeField = FieldSig("d", "h", "I")
        val deadField = FieldSig("d", "z", "I")
        val symbolsByClass = mapOf(
            "d" to ClassSymbols(
                fields = listOf(deadField, activeField),
                methods = emptyList(),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(classes = mapOf("d" to "GameEngine"))
        val usage = UsageStats(
            fieldReads = mapOf(activeField to 5),
            fieldWrites = mapOf(activeField to 1, deadField to 3),
            fieldAccessors = mapOf(activeField to setOf("d"), deadField to setOf("d")),
        )

        val stats = writeUsagePriorityReport(
            markdownPath = markdownPath,
            tsvPath = tsvPath,
            symbolsByClass = symbolsByClass,
            cmap = cmap,
            usage = usage,
        )

        stats.symbolTotal shouldBe 1
        stats.unmappedTotal shouldBe 1
        stats.deadTotal shouldBe 1

        val rows = readTsvRows(tsvPath)
        rows.map { it.getValue("name_raw") } shouldBe listOf("h", "z")
        rows.map { it.getValue("status") } shouldBe listOf("UNMAPPED", "DEAD")
    }

    test("writeUsagePriorityReport keeps raw traffic separate from weighted priority") {
        val outDir = Files.createTempDirectory("usage-traffic")
        val method = MethodSig("d", "a", "()V")
        val tsvPath = outDir.resolve("usage-priority.tsv")

        writeUsagePriorityReport(
            markdownPath = outDir.resolve("usage-priority.md"),
            tsvPath = tsvPath,
            symbolsByClass = mapOf(
                "d" to ClassSymbols(fields = emptyList(), methods = listOf(method), methodAccess = emptyMap()),
            ),
            cmap = CanonicalMap(classes = mapOf("d" to "GameEngine")),
            usage = UsageStats(
                methodRefs = mapOf(method to 5),
                methodCallers = mapOf(method to setOf("d.a()V", "d.b()V", "d.c()V")),
            ),
        )

        val row = readTsvRows(tsvPath).single()
        row.getValue("score") shouldBe "11"
        row.getValue("class_unmapped_impact") shouldBe "11"
        row.getValue("class_total_traffic") shouldBe "5"
    }
})
