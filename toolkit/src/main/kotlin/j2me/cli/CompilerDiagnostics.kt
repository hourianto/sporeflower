package j2me.cli

internal data class CompilerDiagnostics(
    val errors: List<CompilerDiagnosticError>,
    val warningCount: Int,
) {
    val errorLines: List<String> get() = errors.map { it.line }
}

internal data class CompilerDiagnosticError(
    val line: String,
    val message: String? = null,
    val file: String? = null,
)

private val javacErrorRegex = Regex("^(.*):[0-9]+:\\s*error:\\s*(.*)$")
private val javacBareErrorRegex = Regex("^.*\\berror:\\s*(.*)$")
private val javacWarningRegex = Regex("^(.*):[0-9]+:\\s*warning:\\s*(.*)$")
private val javacBareWarningRegex = Regex("^\\s*warning:\\s*.*$")

private val ecjErrorRegex = Regex("^\\d+\\. ERROR in (.*) \\(at line \\d+\\)$")
private val ecjWarningRegex = Regex("^\\d+\\. WARNING in ")

private val legacyProblemRegex = Regex("^(.+\\.java):[0-9]+:\\s*(.*)$")
private val legacyWarningMessageRegex = Regex("^warning:\\s+.*", RegexOption.IGNORE_CASE)
private val legacyWarningSummaryRegex = Regex("^\\s*(\\d+)\\s+warnings?\\s*$")

internal fun parseJavacDiagnostics(stderr: String): CompilerDiagnostics {
    var warningCount = 0
    val errors = buildList {
        stderr.lines().forEach { line ->
            when {
                javacWarningRegex.matches(line) || javacBareWarningRegex.matches(line) -> warningCount++
                else -> addJavacError(line)
            }
        }
    }
    return CompilerDiagnostics(errors, warningCount)
}

private fun MutableList<CompilerDiagnosticError>.addJavacError(line: String) {
    val richMatch = javacErrorRegex.matchEntire(line)
    if (richMatch != null) {
        add(
            CompilerDiagnosticError(
                line = line,
                file = richMatch.groupValues[1],
                message = richMatch.groupValues[2],
            ),
        )
        return
    }

    javacBareErrorRegex.matchEntire(line)?.let { match ->
        add(CompilerDiagnosticError(line = line, message = match.groupValues[1]))
    }
}

internal fun parseEcjDiagnostics(stderr: String): CompilerDiagnostics {
    val lines = stderr.lines()
    val errors = lines.mapIndexedNotNull { index, line ->
        ecjErrorRegex.matchEntire(line)?.let { match ->
            CompilerDiagnosticError(
                line = line,
                file = match.groupValues[1],
                message = lines.diagnosticBlockMessageAfter(index),
            )
        }
    }
    return CompilerDiagnostics(
        errors = errors,
        warningCount = lines.count { ecjWarningRegex.containsMatchIn(it) },
    )
}

private fun List<String>.diagnosticBlockMessageAfter(headerIndex: Int): String? =
    asSequence()
        .drop(headerIndex + 1)
        .takeWhile { it != "----------" }
        .map { it.trim() }
        .filter { it.isNotEmpty() }
        .lastOrNull()

internal fun parseLegacyJavacDiagnostics(stderr: String): CompilerDiagnostics {
    var warningCount = 0
    var summaryWarningCount = 0
    val errors = buildList {
        stderr.lines().forEach { line ->
            legacyWarningSummaryRegex.matchEntire(line)?.groupValues?.get(1)?.toIntOrNull()?.let {
                summaryWarningCount = maxOf(summaryWarningCount, it)
            }
            val match = legacyProblemRegex.matchEntire(line) ?: return@forEach
            val message = match.groupValues[2].trim()
            if (message.isBlank()) {
                return@forEach
            }
            if (legacyWarningMessageRegex.matches(message)) {
                warningCount++
                return@forEach
            }
            add(
                CompilerDiagnosticError(
                    line = line,
                    file = match.groupValues[1],
                    message = message,
                ),
            )
        }
    }

    return CompilerDiagnostics(errors, maxOf(warningCount, summaryWarningCount))
}
