package j2me.common

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking
import java.util.concurrent.atomic.AtomicInteger

fun <T, R> parallelMap(
    workers: Int,
    items: List<T>,
    transform: (T) -> R,
): List<R> {
    if (items.isEmpty()) {
        return emptyList()
    }
    val parallelism = workers.coerceAtLeast(1).coerceAtMost(items.size)
    if (parallelism == 1) {
        return items.map(transform)
    }
    return runBlocking {
        val nextIndex = AtomicInteger()
        val results = arrayOfNulls<Any?>(items.size)
        List(parallelism) {
            async(Dispatchers.Default) {
                while (true) {
                    val index = nextIndex.getAndIncrement()
                    if (index >= items.size) {
                        break
                    }
                    results[index] = transform(items[index])
                }
            }
        }.awaitAll()

        @Suppress("UNCHECKED_CAST")
        results.map { it as R }
    }
}
