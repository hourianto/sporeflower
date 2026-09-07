package j2me.cli

internal enum class StageStatus { SKIPPED, PASS, FAIL }

/** A failed operation may still produce useful output, such as compiler diagnostics. */
internal class StageResult<out T> private constructor(
    val status: StageStatus,
    val elapsedMs: Long,
    val value: T?,
    val failure: Throwable?,
) {
    companion object {
        fun <T> skipped(): StageResult<T> = StageResult(StageStatus.SKIPPED, 0, null, null)

        fun <T> run(
            failureMessage: (T) -> String? = { null },
            nanoTime: () -> Long = System::nanoTime,
            operation: () -> T,
        ): StageResult<T> {
            val start = nanoTime()
            var value: T? = null
            val failure = try {
                value = operation()
                failureMessage(value)?.let(::IllegalStateException)
            } catch (exc: InterruptedException) {
                Thread.currentThread().interrupt()
                throw exc
            } catch (exc: Throwable) {
                // Assertions from malformed bytecode must remain per-project
                // failures rather than aborting the remaining corpus jobs.
                exc
            }
            return StageResult(
                if (failure == null) StageStatus.PASS else StageStatus.FAIL,
                (nanoTime() - start) / 1_000_000,
                value,
                failure,
            )
        }
    }
}
