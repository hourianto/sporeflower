package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test

class StageResultTest {
    @Test fun `failed stages retain elapsed time and their cause`() {
        for (failure in listOf(IllegalArgumentException("failed"), AssertionError("invalid bytecode"))) {
            val clock = ArrayDeque(listOf(0L, 13_000_000L))
            val result = StageResult.run(nanoTime = { clock.removeFirst() }) { throw failure }
            assertEquals(StageStatus.FAIL, result.status)
            assertEquals(13, result.elapsedMs)
            assertSame(failure, result.failure)
        }
    }

    @Test fun `compiler failures retain diagnostics without reading a summary file`() {
        val output = CompileResult(2, CompilerDiagnostics(listOf(CompilerDiagnosticError("broken source")), 1), "compilation failed")
        val result = StageResult.run(failureMessage = CompileResult::failureMessage) { output }
        assertEquals(StageStatus.FAIL, result.status)
        assertSame(output, result.value)
        assertEquals("compilation failed", result.failure?.message)
        assertEquals(StageStatus.SKIPPED, StageResult.skipped<Unit>().status)
    }
}
