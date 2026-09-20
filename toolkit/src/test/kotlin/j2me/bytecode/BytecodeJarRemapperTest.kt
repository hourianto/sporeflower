package j2me.bytecode

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldContain
import io.kotest.matchers.shouldBe
import io.kotest.matchers.shouldNotBe
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.SemanticTarget
import j2me.symbols.parseClassSymbols
import org.objectweb.asm.Attribute
import org.objectweb.asm.ByteVector
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Label
import org.objectweb.asm.Opcodes
import org.objectweb.asm.tree.ClassNode
import org.objectweb.asm.tree.FieldInsnNode
import org.objectweb.asm.tree.MethodInsnNode
import org.objectweb.asm.tree.LineNumberNode
import java.net.URLClassLoader
import java.nio.file.Files
import java.util.zip.ZipEntry
import java.util.zip.ZipFile
import java.util.zip.ZipOutputStream

class BytecodeJarRemapperTest : FunSpec({
    test("remapJarBytecode writes renamed classes members and resources") {
        val root = Files.createTempDirectory("bytecode-remap")
        val inputJar = root.resolve("game.jar")
        val outputJar = defaultRemappedJarPath(inputJar, root.resolve("out"))
        val field = FieldSig("a", "a", "I")
        val method = MethodSig("a", "a", "()I")

        writeInputJar(inputJar)

        val stats = remapJarBytecode(
            inputJar = inputJar,
            outputJar = outputJar,
            mappings = CanonicalMap(
                classes = mapOf("a" to "Foo"),
                fields = mapOf(field to "value"),
                methods = mapOf(method to "getValue"),
            ),
            symbolsByClass = mapOf("a" to ClassSymbols(listOf(field), listOf(method))),
        )

        stats.path shouldBe outputJar
        stats.classCount shouldBe 1
        stats.resourceCount shouldBe 2
        outputJar.fileName.toString() shouldBe "game_remapped.jar"

        ZipFile(outputJar.toFile()).use { zip ->
            zip.getEntry("a.class") shouldBe null
            zip.getEntry("defpackage/Foo.class") shouldNotBe null
            zip.getEntry("assets/data.txt") shouldNotBe null
            zip.getInputStream(zip.getEntry("assets/data.txt")).use { it.readBytes().decodeToString() } shouldBe "hello"
            zip.getInputStream(zip.getEntry("main.class")).use { input ->
                input.readBytes() shouldBe pngHeaderBytes()
            }

            val node = ClassNode()
            zip.getInputStream(zip.getEntry("defpackage/Foo.class")).use { input ->
                ClassReader(input.readBytes()).accept(node, 0)
            }
            node.name shouldBe "defpackage/Foo"
            node.fields.map { it.name } shouldContain "value"
            node.methods.map { it.name } shouldContain "getValue"
        }
    }

    test("remapJarBytecode renames references whose owner inherits the declaration") {
        val root = Files.createTempDirectory("bytecode-inherited-reference")
        val inputJar = root.resolve("game.jar")
        val outputJar = root.resolve("out.jar")
        val field = FieldSig("Parent", "f", "I")
        val method = MethodSig("Parent", "m", "()V")

        ZipOutputStream(Files.newOutputStream(inputJar)).use { zip ->
            for ((owner, bytes) in mapOf(
                "Parent" to parentClassBytes(),
                "Child" to childClassBytes(),
            )) {
                zip.putNextEntry(ZipEntry("$owner.class"))
                zip.write(bytes)
                zip.closeEntry()
            }
        }

        remapJarBytecode(
            inputJar,
            outputJar,
            CanonicalMap(fields = mapOf(field to "value"), methods = mapOf(method to "tick")),
            mapOf(
                "Parent" to ClassSymbols(listOf(field), listOf(method)),
                "Child" to ClassSymbols(emptyList(), emptyList(), superName = "Parent"),
            ),
        )

        ZipFile(outputJar.toFile()).use { zip ->
            val node = ClassNode()
            zip.getInputStream(zip.getEntry("defpackage/Child.class")).use { ClassReader(it.readBytes()).accept(node, 0) }
            val instructions = node.methods.single { it.name == "useInherited" }.instructions.toArray()
            instructions.filterIsInstance<MethodInsnNode>().single().name shouldBe "tick"
            instructions.filterIsInstance<FieldInsnNode>().single().name shouldBe "value"
        }
    }

    for ((attribute, start, length) in listOf(
        Triple("LocalVariableTable", 0, 24),
        Triple("LocalVariableTable", 24, 1),
        Triple("LineNumberTable", 24, 0),
    )) {
        test("remapJarBytecode tolerates stale $attribute at $start with length $length") {
            val root = Files.createTempDirectory("bytecode-stale-debug")
            val input = root.resolve("input.jar")
            val output = root.resolve("output.jar")
            val bytes = debugClassBytes("sample/Subject", staleDebugAttribute(attribute, start, length))
            // Faithfully reproduce a stale debug range on a five-byte constructor.
            shouldThrow<ArrayIndexOutOfBoundsException> { ClassReader(bytes).accept(ClassNode(), 0) }
            ZipOutputStream(Files.newOutputStream(input)).use { zip ->
                for ((owner, data) in mapOf("sample/Subject" to bytes, "sample/Healthy" to debugClassBytes("sample/Healthy"))) {
                    zip.putNextEntry(ZipEntry("$owner.class"))
                    zip.write(data)
                    zip.closeEntry()
                }
            }
            val field = FieldSig("sample/Subject", "TYPE", "Ljava/lang/String;")
            val method = MethodSig("sample/Subject", "name", "(Z)Ljava/lang/String;")
            val stats = remapJarBytecode(
                input, output,
                CanonicalMap(
                    classes = mapOf("sample/Subject" to "named/Subject", "sample/Healthy" to "named/Healthy"),
                    fields = mapOf(field to "CLASS_NAME"),
                    methods = mapOf(method to "getName"),
                ),
                listOf("sample/Subject", "sample/Healthy").associateWith { parseClassSymbols(input, it) },
                listOf(
                    ClassNameLiteral(SemanticTarget.Field(field), -1, "sample.Subject", "named.Subject"),
                    ClassNameLiteral(SemanticTarget.Return(method), 4, "sample.Subject", "named.Subject"),
                ),
            )
            stats.classCount shouldBe 2
            ZipFile(output.toFile()).use { zip ->
                val subject = ClassNode()
                ClassReader(zip.getInputStream(zip.getEntry("named/Subject.class")).use { it.readBytes() }).accept(subject, 0)
                subject.sourceFile shouldBe null
                subject.fields.map { it.name } shouldBe listOf("CLASS_NAME")
                subject.methods.map { it.name } shouldBe listOf("getName", "<init>")
                subject.methods.all { it.localVariables.isEmpty() } shouldBe true

                val healthy = ClassNode()
                ClassReader(zip.getInputStream(zip.getEntry("named/Healthy.class")).use { it.readBytes() }).accept(healthy, 0)
                healthy.sourceFile shouldBe "Subject.java"
                val constructor = healthy.methods.single { it.name == "<init>" }
                constructor.localVariables.single().desc shouldBe "Lnamed/Healthy;"
                constructor.instructions.filterIsInstance<LineNumberNode>().single().line shouldBe 10
            }
            // Loading and exercising both branches verifies that frames, code, and literal
            // offsets survive the retry, including the method visited before the bad table.
            URLClassLoader(arrayOf(output.toUri().toURL()), ClassLoader.getPlatformClassLoader()).use { loader ->
                val subject = loader.loadClass("named.Subject")
                subject.getConstructor().newInstance().javaClass shouldBe subject
                subject.getField("CLASS_NAME").get(null) shouldBe "named.Subject"
                val getName = subject.getMethod("getName", Boolean::class.javaPrimitiveType)
                getName.invoke(null, true) shouldBe "named.Subject"
                getName.invoke(null, false) shouldBe "plain"
            }
        }
    }

    test("remapJarBytecode still rejects an out of bounds branch target") {
        val root = Files.createTempDirectory("bytecode-invalid-code")
        val input = root.resolve("input.jar")
        // The branch target is outside Code, independently of any debug attributes.
        val bytes = ClassWriter(0).apply {
            visit(Opcodes.V1_2, Opcodes.ACC_PUBLIC, "Broken", null, "java/lang/Object", null)
            visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "run", "()V", null, null).apply {
                visitAttribute(object : Attribute("Code") {
                    override fun write(writer: ClassWriter, code: ByteArray?, codeLength: Int, maxStack: Int, maxLocals: Int): ByteVector =
                        ByteVector().putShort(0).putShort(0).putInt(4)
                            .putByte(Opcodes.GOTO).putShort(24).putByte(Opcodes.RETURN)
                            .putShort(0).putShort(0)
                })
                visitEnd()
            }
            visitEnd()
        }.toByteArray()
        ZipOutputStream(Files.newOutputStream(input)).use { zip ->
            zip.putNextEntry(ZipEntry("Broken.class"))
            zip.write(bytes)
            zip.closeEntry()
        }
        val failure = shouldThrow<IndexOutOfBoundsException> {
            remapJarBytecode(input, root.resolve("output.jar"), CanonicalMap(), emptyMap(), emptyList())
        }
        failure.suppressed.size shouldBe 1
    }
})

private fun staleDebugAttribute(name: String, start: Int, length: Int): Attribute = object : Attribute(name) {
    override fun isCodeAttribute(): Boolean = true

    override fun write(writer: ClassWriter, code: ByteArray?, codeLength: Int, maxStack: Int, maxLocals: Int): ByteVector {
        codeLength shouldBe 5
        val data = ByteVector().putShort(1).putShort(start)
        return if (name == "LineNumberTable") data.putShort(10) else
            data.putShort(length).putShort(writer.newUTF8("this")).putShort(writer.newUTF8("Lsample/Subject;")).putShort(0)
    }
}

private fun debugClassBytes(owner: String, invalidDebug: Attribute? = null): ByteArray = ClassWriter(ClassWriter.COMPUTE_FRAMES).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, owner, null, "java/lang/Object", null)
    visitSource("Subject.java", null)
    visitField(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL, "TYPE", "Ljava/lang/String;", null, "sample.Subject").visitEnd()
    visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "name", "(Z)Ljava/lang/String;", null, null).apply {
        visitCode()
        val otherwise = Label()
        visitVarInsn(Opcodes.ILOAD, 0)
        visitJumpInsn(Opcodes.IFEQ, otherwise)
        visitLdcInsn("sample.Subject")
        visitInsn(Opcodes.ARETURN)
        visitLabel(otherwise)
        visitLdcInsn("plain")
        visitInsn(Opcodes.ARETURN)
        visitMaxs(0, 0)
        visitEnd()
    }
    visitMethod(Opcodes.ACC_PUBLIC, "<init>", "()V", null, null).apply {
        visitCode()
        val start = Label()
        val end = Label()
        visitLabel(start)
        visitVarInsn(Opcodes.ALOAD, 0)
        visitMethodInsn(Opcodes.INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
        visitInsn(Opcodes.RETURN)
        visitLabel(end)
        if (invalidDebug == null) {
            visitLineNumber(10, start)
            visitLocalVariable("this", "L$owner;", null, start, end, 0)
        } else {
            visitAttribute(invalidDebug)
        }
        visitMaxs(0, 0)
        visitEnd()
    }
    visitEnd()
}.toByteArray()

private fun writeInputJar(path: java.nio.file.Path) {
    ZipOutputStream(Files.newOutputStream(path)).use { zip ->
        zip.putNextEntry(ZipEntry("a.class"))
        zip.write(inputClassBytes())
        zip.closeEntry()

        zip.putNextEntry(ZipEntry("assets/data.txt"))
        zip.write("hello".encodeToByteArray())
        zip.closeEntry()

        zip.putNextEntry(ZipEntry("main.class"))
        zip.write(pngHeaderBytes())
        zip.closeEntry()
    }
}

private fun pngHeaderBytes(): ByteArray =
    byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

private fun inputClassBytes(): ByteArray {
    val writer = ClassWriter(0)
    writer.visit(Opcodes.V1_2, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
    writer.visitField(Opcodes.ACC_PUBLIC, "a", "I", null, null).visitEnd()

    writer.visitMethod(Opcodes.ACC_PUBLIC, "<init>", "()V", null, null).also { method ->
        method.visitCode()
        method.visitVarInsn(Opcodes.ALOAD, 0)
        method.visitMethodInsn(Opcodes.INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
        method.visitInsn(Opcodes.RETURN)
        method.visitMaxs(1, 1)
        method.visitEnd()
    }

    writer.visitMethod(Opcodes.ACC_PUBLIC, "a", "()I", null, null).also { method ->
        method.visitCode()
        method.visitVarInsn(Opcodes.ALOAD, 0)
        method.visitFieldInsn(Opcodes.GETFIELD, "a", "a", "I")
        method.visitInsn(Opcodes.IRETURN)
        method.visitMaxs(1, 1)
        method.visitEnd()
    }

    writer.visitEnd()
    return writer.toByteArray()
}

private fun parentClassBytes(): ByteArray = ClassWriter(0).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Parent", null, "java/lang/Object", null)
    visitField(Opcodes.ACC_PUBLIC, "f", "I", null, null).visitEnd()
    visitMethod(Opcodes.ACC_PUBLIC, "m", "()V", null, null).visitEnd()
    visitEnd()
}.toByteArray()

private fun childClassBytes(): ByteArray = ClassWriter(0).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Child", null, "Parent", null)
    visitMethod(Opcodes.ACC_PUBLIC, "useInherited", "()V", null, null).apply {
        visitVarInsn(Opcodes.ALOAD, 0)
        visitMethodInsn(Opcodes.INVOKEVIRTUAL, "Child", "m", "()V", false)
        visitVarInsn(Opcodes.ALOAD, 0)
        visitFieldInsn(Opcodes.GETFIELD, "Child", "f", "I")
        visitInsn(Opcodes.POP)
        visitInsn(Opcodes.RETURN)
        visitMaxs(1, 1)
        visitEnd()
    }
    visitEnd()
}.toByteArray()
