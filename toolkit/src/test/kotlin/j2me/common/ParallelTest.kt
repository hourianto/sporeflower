package j2me.common

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldContainExactly
import io.kotest.matchers.shouldBe
import java.util.concurrent.ConcurrentHashMap

class ParallelTest : FunSpec({
    test("parallelMap preserves input order and nullable results") {
        parallelMap(3, listOf(3, 1, 2)) { value ->
            if (value == 1) null else value * 2
        } shouldContainExactly listOf(6, null, 4)
    }

    test("parallelMap limits work to the requested number of workers") {
        val workerThreads = ConcurrentHashMap.newKeySet<String>()

        val results = parallelMap(2, (1..100).toList()) { value ->
            workerThreads += Thread.currentThread().name
            value * value
        }

        results.size shouldBe 100
        workerThreads.isNotEmpty() shouldBe true
        (workerThreads.size <= 2) shouldBe true
    }

    test("parallelMap handles empty input and coerces non-positive worker counts") {
        parallelMap(4, emptyList<Int>()) { it } shouldBe emptyList()
        parallelMap(0, listOf(1, 2, 3)) { it + 1 } shouldContainExactly listOf(2, 3, 4)
    }
})
