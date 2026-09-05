package j2me.common

fun isJavaClassFile(bytes: ByteArray): Boolean =
    bytes.size >= 4 &&
        bytes[0] == 0xCA.toByte() &&
        bytes[1] == 0xFE.toByte() &&
        bytes[2] == 0xBA.toByte() &&
        bytes[3] == 0xBE.toByte()
