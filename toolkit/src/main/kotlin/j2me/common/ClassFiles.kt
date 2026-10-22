package j2me.common

import java.io.InputStream

/** Read a bounded header, retaining short input so callers can reject it. */
internal fun InputStream.readHeader(size: Int): ByteArray {
    val bytes = ByteArray(size)
    var offset = 0
    while (offset < size) {
        val count = read(bytes, offset, size - offset)
        if (count < 0) break
        if (count == 0) {
            val next = read()
            if (next < 0) break
            bytes[offset++] = next.toByte()
        } else {
            offset += count
        }
    }
    return if (offset == size) bytes else bytes.copyOf(offset)
}

fun isJavaClassFile(bytes: ByteArray): Boolean =
    bytes.size >= 4 &&
        bytes[0] == 0xCA.toByte() &&
        bytes[1] == 0xFE.toByte() &&
        bytes[2] == 0xBA.toByte() &&
        bytes[3] == 0xBE.toByte()
