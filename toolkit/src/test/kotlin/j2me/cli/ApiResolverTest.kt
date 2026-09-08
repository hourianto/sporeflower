package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.tree.ClassNode
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.Attributes
import java.util.jar.JarEntry
import java.util.jar.JarFile
import java.util.jar.JarOutputStream
import java.util.jar.Manifest
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

class ApiResolverTest {
    @TempDir lateinit var root: Path

    @Test fun `selects a complete vendor definition using the original return descriptor`() {
        val old = api("a-old.jar", "vendor/File" to definition("vendor/File", "mkdir", "()V"))
        val newer = api("z-new.jar", "vendor/File" to definition("vendor/File", "mkdir", "()Z"))
        val input = project("vendor/File", "mkdir", "()Z")
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        assertArrayEquals(classBytes(newer, "vendor/File"), classBytes(resolved, "vendor/File"))
        assertEquals(resolved, resolveApiJars(input, listOf(newer, old), root.resolve("cache")).single())
        val voidInput = project("vendor/File", "mkdir", "()V", "void.jar")
        val voidResolved = resolveApiJars(voidInput, listOf(old, newer), root.resolve("cache")).single()
        assertArrayEquals(classBytes(old, "vendor/File"), classBytes(voidResolved, "vendor/File"))
    }

    @Test fun `prefers standalone CLDC to a device SDK and honors declared CLDC version`() {
        val cldc10 = api("cldc10.jar", "java/lang/Object" to definition("java/lang/Object", "old", "()V"))
        val cldc11 = api("cldc11.jar",
            "java/lang/Object" to definition("java/lang/Object", "newer", "()V"),
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val device = api("a-device.jar",
            "java/lang/Object" to definition("java/lang/Object", "deviceOnly", "()V"),
            "javax/microedition/midlet/MIDlet" to definition("javax/microedition/midlet/MIDlet", "startApp", "()V"))
        for ((configuration, expected) in listOf("CLDC-1.0" to cldc10, "CLDC-1.1" to cldc11)) {
            val input = project("vendor/Other", "call", "()V", "$configuration.jar", configuration)
            val resolved = resolveApiJars(input, listOf(device, cldc10, cldc11), root.resolve("cache")).single()
            assertArrayEquals(classBytes(expected, "java/lang/Object"), classBytes(resolved, "java/lang/Object"))
            JarFile(resolved.toFile()).use { assertEquals(configuration == "CLDC-1.1", it.getJarEntry("java/lang/Float.class") != null) }
        }
    }

    @Test fun `static invocation selects the static API revision`() {
        val instance = api("a-instance.jar", "vendor/Light" to definition("vendor/Light", "available", "()Z"))
        val static = api("z-static.jar", "vendor/Light" to definition("vendor/Light", "available", "()Z", static = true))
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Sample", null, "java/lang/Object", null)
        val method = writer.visitMethod(ACC_PUBLIC or ACC_STATIC, "check", "()Z", null, null)
        method.visitCode()
        method.visitMethodInsn(INVOKESTATIC, "vendor/Light", "available", "()Z", false)
        method.visitInsn(IRETURN)
        method.visitMaxs(1, 0)
        method.visitEnd()
        writer.visitEnd()
        val input = api("input.jar", "Sample" to writer.toByteArray())
        val resolved = resolveApiJars(input, listOf(instance, static), root.resolve("cache")).single()
        assertArrayEquals(classBytes(static, "vendor/Light"), classBytes(resolved, "vendor/Light"))
    }

    @Test fun `bytecode requirements override a stale CLDC manifest`() {
        val cldc10 = api("cldc10.jar", "java/lang/Object" to definition("java/lang/Object", "old", "()V"))
        val cldc11 = api("cldc11.jar",
            "java/lang/Object" to definition("java/lang/Object", "newer", "()V"),
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val input = project("java/lang/Float", "value", "()F", configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(cldc10, cldc11), root.resolve("cache")).single()
        assertArrayEquals(classBytes(cldc11, "java/lang/Object"), classBytes(resolved, "java/lang/Object"))
    }

    @Test fun `a required device-specific core member can select the device SDK`() {
        val cldc = api("cldc.jar", "java/lang/Object" to definition("java/lang/Object", "standard", "()V"))
        val device = api("device.jar",
            "java/lang/Object" to definition("java/lang/Object", "extension", "()I"),
            "javax/microedition/midlet/MIDlet" to definition("javax/microedition/midlet/MIDlet", "startApp", "()V"))
        val input = project("java/lang/Object", "extension", "()I")
        val resolved = resolveApiJars(input, listOf(cldc, device), root.resolve("cache")).single()
        assertArrayEquals(classBytes(device, "java/lang/Object"), classBytes(resolved, "java/lang/Object"))
    }

    @Test fun `references through project subclasses constrain the inherited core API`() {
        val objectClass = definition("java/lang/Object", "standard", "()V")
        val old = api("cldc10.jar", "java/lang/Object" to objectClass,
            "java/lang/Thread" to definition("java/lang/Thread", "run", "()V"))
        val newer = api("cldc11.jar", "java/lang/Object" to objectClass,
            "java/lang/Thread" to definition("java/lang/Thread", "interrupt", "()V"),
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Worker", null, "java/lang/Thread", null)
        writer.newMethod("Worker", "interrupt", "()V", false)
        writer.visitEnd()
        val input = api("input.jar", "Worker" to writer.toByteArray(), configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        assertArrayEquals(classBytes(newer, "java/lang/Thread"), classBytes(resolved, "java/lang/Thread"))
    }

    @Test fun `malformed advisory manifests do not prevent dependency inspection`() {
        val library = api("api.jar", "vendor/File" to definition("vendor/File", "mkdir", "()Z"))
        val valid = project("vendor/File", "mkdir", "()Z")
        val malformed = root.resolve("malformed.jar")
        ZipOutputStream(Files.newOutputStream(malformed)).use { zip ->
            zip.putNextEntry(ZipEntry("META-INF/MANIFEST.MF"))
            zip.write("Manifest-Version: 1.0\ninvalid header\n\n".toByteArray())
            zip.closeEntry()
            zip.putNextEntry(ZipEntry("Sample.class"))
            zip.write(classBytes(valid, "Sample"))
            zip.closeEntry()
        }
        assertEquals(
            resolveApiJars(valid, listOf(library), root.resolve("cache")),
            resolveApiJars(malformed, listOf(library), root.resolve("cache")),
        )
    }

    @Test fun `resources with a class suffix do not enter dependency inspection`() {
        val old = api("a-old.jar", "vendor/File" to definition("vendor/File", "mkdir", "()V"))
        val newer = api("z-new.jar", "vendor/File" to definition("vendor/File", "mkdir", "()Z"))
        val valid = project("vendor/File", "mkdir", "()Z")
        val pngHeader = byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)
        val input = api("with-resource.jar", "Sample" to classBytes(valid, "Sample"), "picture" to pngHeader)
        val original = Files.readAllBytes(input)
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        assertArrayEquals(classBytes(newer, "vendor/File"), classBytes(resolved, "vendor/File"))
        assertArrayEquals(original, Files.readAllBytes(input))
    }

    @Test fun `finds required inherited members without replacing the class definition`() {
        val old = api("a-old.jar", "vendor/Child" to definition("vendor/Child", "other", "()V"))
        val parent = definition("vendor/Parent", "call", "()I")
        val child = definition("vendor/Child", "other", "()V", "vendor/Parent")
        val newer = api("z-new.jar", "vendor/Child" to child, "vendor/Parent" to parent)
        val input = project("vendor/Child", "call", "()I")
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        assertArrayEquals(child, classBytes(resolved, "vendor/Child"))
        val node = ClassNode()
        ClassReader(classBytes(resolved, "vendor/Child")).accept(node, 0)
        assertFalse(node.methods.any { it.name == "call" })
    }

    @Test fun `inherited requirements also select a compatible parent from another jar`() {
        val old = api("a-parent.jar", "vendor/Parent" to definition("vendor/Parent", "other", "()V"))
        val newer = api("z-parent.jar", "vendor/Parent" to definition("vendor/Parent", "call", "()I"))
        val child = api("child.jar", "vendor/Child" to definition("vendor/Child", "other", "()V", "vendor/Parent"))
        val input = project("vendor/Child", "call", "()I")
        val resolved = resolveApiJars(input, listOf(old, newer, child), root.resolve("cache")).single()
        assertArrayEquals(classBytes(newer, "vendor/Parent"), classBytes(resolved, "vendor/Parent"))
        assertEquals(resolved, resolveApiJars(input, listOf(child, newer, old), root.resolve("cache")).single())
    }

    @Test fun `inherited calls through optional APIs can upgrade the core library`() {
        val objectClass = definition("java/lang/Object", "standard", "()V")
        val old = api("cldc10.jar", "java/lang/Object" to objectClass,
            "java/lang/Thread" to definition("java/lang/Thread", "run", "()V"))
        val newer = api("cldc11.jar", "java/lang/Object" to objectClass,
            "java/lang/Thread" to definition("java/lang/Thread", "interrupt", "()V"),
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val extension = api("extension.jar",
            "vendor/Worker" to definition("vendor/Worker", "work", "()V", "java/lang/Thread"))
        val input = project("vendor/Worker", "interrupt", "()V", configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(old, newer, extension), root.resolve("cache")).single()
        assertArrayEquals(classBytes(newer, "java/lang/Thread"), classBytes(resolved, "java/lang/Thread"))
    }

    @Test fun `device-only core libraries are selected coherently`() {
        val objectClass = definition("java/lang/Object", "standard", "()V")
        val midlet = definition("javax/microedition/midlet/MIDlet", "startApp", "()V")
        val old = api("a-device.jar", "java/lang/Object" to objectClass,
            "javax/microedition/midlet/MIDlet" to midlet)
        val newer = api("z-device.jar", "java/lang/Object" to objectClass,
            "javax/microedition/midlet/MIDlet" to midlet,
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val input = project("vendor/Other", "call", "()V", configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        JarFile(resolved.toFile()).use { assertNull(it.getJarEntry("java/lang/Float.class")) }
    }

    @Test fun `MIDP java classes remain available outside the standalone CLDC core`() {
        val objectClass = definition("java/lang/Object", "standard", "()V")
        val cldc = api("cldc.jar", "java/lang/Object" to objectClass)
        val exception = definition("java/lang/IllegalStateException", "getMessage", "()Ljava/lang/String;")
        val device = api("device.jar", "java/lang/Object" to objectClass,
            "javax/microedition/midlet/MIDlet" to definition("javax/microedition/midlet/MIDlet", "startApp", "()V"),
            "java/lang/IllegalStateException" to exception)
        val input = project("vendor/Other", "call", "()V", configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(cldc, device), root.resolve("cache")).single()
        assertArrayEquals(exception, classBytes(resolved, "java/lang/IllegalStateException"))
    }

    @Test fun `floating point instructions without floating point descriptors require CLDC 11`() {
        val objectClass = definition("java/lang/Object", "standard", "()V")
        val old = api("cldc10.jar", "java/lang/Object" to objectClass)
        val newer = api("device.jar", "java/lang/Object" to objectClass,
            "javax/microedition/midlet/MIDlet" to definition("javax/microedition/midlet/MIDlet", "startApp", "()V"),
            "java/lang/Float" to definition("java/lang/Float", "value", "()F"))
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Sample", null, "java/lang/Object", null)
        val method = writer.visitMethod(ACC_PUBLIC or ACC_STATIC, "convert", "()I", null, null)
        method.visitCode()
        method.visitInsn(FCONST_1)
        method.visitInsn(F2I)
        method.visitInsn(IRETURN)
        method.visitMaxs(1, 0)
        method.visitEnd()
        writer.visitEnd()
        val input = api("input.jar", "Sample" to writer.toByteArray(), configuration = "CLDC-1.0")
        val resolved = resolveApiJars(input, listOf(old, newer), root.resolve("cache")).single()
        JarFile(resolved.toFile()).use { assertNotNull(it.getJarEntry("java/lang/Float.class")) }
    }

    @Test fun `compile-only providers are fallbacks and their provenance invalidates snapshots`() {
        val bytes = definition("vendor/Api", "call", "()V")
        val local = api("a-local.jar", "vendor/Api" to bytes, compileOnly = true)
        val sdk = api("z-sdk.jar", "vendor/Api" to bytes)
        val input = project("vendor/Api", "call", "()V")
        val resolved = resolveApiJars(input, listOf(local, sdk), root.resolve("cache")).single()
        JarFile(resolved.toFile()).use {
            assertTrue(it.getInputStream(it.getJarEntry("META-INF/j2me-api-sources.tsv"))
                .bufferedReader().use { reader -> reader.readText() }.contains("vendor/Api\tz-sdk.jar\tfalse"))
        }
        val before = resolveApiJars(input, listOf(local), root.resolve("cache")).single()
        api("a-local.jar", "vendor/Api" to bytes, compileOnly = false)
        val after = resolveApiJars(input, listOf(local), root.resolve("cache")).single()
        assertNotEquals(before, after)
    }

    private fun definition(owner: String, name: String, descriptor: String, parent: String = "java/lang/Object", static: Boolean = false): ByteArray {
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC or ACC_ABSTRACT, owner, null, if (owner == "java/lang/Object") null else parent, null)
        writer.visitMethod(ACC_PUBLIC or if (static) ACC_STATIC or ACC_NATIVE else ACC_ABSTRACT, name, descriptor, null, null).visitEnd()
        writer.visitEnd()
        return writer.toByteArray()
    }

    private fun project(owner: String, name: String, descriptor: String, filename: String = "input.jar", configuration: String? = null): Path {
        val writer = ClassWriter(0)
        writer.visit(V1_1, ACC_PUBLIC, "Sample", null, "java/lang/Object", null)
        // A constant-pool method reference suffices; dependency discovery must not
        // depend on debug attributes or whether a guarded call is reachable.
        writer.newMethod(owner, name, descriptor, false)
        writer.visitEnd()
        return api(filename, "Sample" to writer.toByteArray(), configuration = configuration)
    }

    private fun api(name: String, vararg classes: Pair<String, ByteArray>, configuration: String? = null, compileOnly: Boolean = false): Path {
        val path = root.resolve(name)
        val manifest = Manifest().apply {
            mainAttributes[Attributes.Name.MANIFEST_VERSION] = "1.0"
            configuration?.let { mainAttributes.putValue("MicroEdition-Configuration", it) }
            if (compileOnly) mainAttributes.putValue("J2ME-Stub-Kind", "compile-only")
        }
        JarOutputStream(Files.newOutputStream(path), manifest).use { jar ->
            for ((owner, bytes) in classes) {
                jar.putNextEntry(JarEntry("$owner.class"))
                jar.write(bytes)
                jar.closeEntry()
            }
        }
        return path
    }

    private fun classBytes(jar: Path, owner: String): ByteArray = JarFile(jar.toFile()).use {
        it.getInputStream(it.getJarEntry("$owner.class")).use { stream -> stream.readBytes() }
    }
}
