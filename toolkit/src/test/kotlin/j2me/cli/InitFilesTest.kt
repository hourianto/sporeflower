package j2me.cli

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.isSymbolicLink
import kotlin.io.path.readSymbolicLink
import kotlin.io.path.readText
import kotlin.io.path.writeText

class InitFilesTest : FunSpec({
    test("writeProjectGuidanceFiles links both guidance names without owning the template") {
        val root = Files.createTempDirectory("init-guidance")
        val templateDir = root.resolve("templates").createDirectories()
        val templatePath = templateDir.resolve("mappings-doc.md")
        val templateContent = "line one\nline two\n"
        templatePath.writeText(templateContent)

        writeProjectGuidanceFiles(root, templatePath)

        val agentsPath = root.resolve("AGENTS.md")
        val claudePath = root.resolve("CLAUDE.md")

        agentsPath.isSymbolicLink() shouldBe true
        agentsPath.readSymbolicLink() shouldBe templatePath.toAbsolutePath().normalize()
        agentsPath.readText() shouldBe templateContent
        claudePath.isSymbolicLink() shouldBe true
        claudePath.readSymbolicLink() shouldBe Path("AGENTS.md")

        Files.delete(agentsPath)
        templatePath.readText() shouldBe templateContent
    }

    test("bytecode config records jar class version and javac target") {
        val root = Files.createTempDirectory("init-bytecode")
        val jar = root.resolve("game.jar")
        writeTinyJar(jar, "Example" to Opcodes.V1_2)

        val bytecode = bytecodeConfigFromJar(jar, source = "config")
        writeProjectConfig(root.resolve("j2me.toml"), "game.jar", bytecode)

        root.resolve("j2me.toml").readText() shouldBe """
            jar = "game.jar"

            [bytecode]
            major = 46
            minor = 0
            class_count = 1
            javac_source = "1.2"
            javac_target = "1.2"
        """.trimIndent() + "\n"

        loadProjectBytecodeConfig(root) shouldBe ProjectBytecodeConfig(
            version = ClassVersion(46, 0),
            javacTarget = "1.2",
            javacSource = "1.2",
            classCount = 1,
            source = "config",
        )
    }

    test("bytecode config uses highest class version in mixed-version jars") {
        val root = Files.createTempDirectory("init-bytecode-mixed")
        val jar = root.resolve("game.jar")
        writeTinyJar(
            jar,
            "OldOne" to Opcodes.V1_1,
            "OldTwo" to Opcodes.V1_1,
            "NewOne" to Opcodes.V1_4,
        )

        val bytecode = bytecodeConfigFromJar(jar, source = "config")

        bytecode.version shouldBe ClassVersion(48, 0)
        bytecode.javacSource shouldBe "1.4"
        bytecode.javacTarget shouldBe "1.4"
        bytecode.classCount shouldBe 3
    }

    test("init target validation rejects managed existing files without force") {
        val root = Files.createTempDirectory("init-existing")
        root.resolve("j2me.toml").writeText("jar = \"old.jar\"\n")

        io.kotest.assertions.throwables.shouldThrow<IllegalArgumentException> {
            validateInitTargets(root, "game.jar", force = false)
        }.message.orEmpty() shouldBe "init would overwrite existing file(s): j2me.toml\nRe-run with --force to replace files managed by init."

        validateInitTargets(root, "game.jar", force = true)
    }

    test("init target validation allows source jar already in fresh project directory") {
        val root = Files.createTempDirectory("init-existing-source-jar")
        val jar = root.resolve("game.jar")
        writeTinyJar(jar, "Example" to Opcodes.V1_2)

        validateInitTargets(root, "game.jar", force = false, sourceJar = jar)
    }

    test("init target validation still rejects different existing destination jar") {
        val root = Files.createTempDirectory("init-existing-different-jar")
        val source = root.resolve("source.jar")
        val destination = root.resolve("game.jar")
        writeTinyJar(source, "Source" to Opcodes.V1_2)
        writeTinyJar(destination, "Destination" to Opcodes.V1_2)

        io.kotest.assertions.throwables.shouldThrow<IllegalArgumentException> {
            validateInitTargets(root, "game.jar", force = false, sourceJar = source)
        }.message.orEmpty() shouldBe "init would overwrite existing file(s): game.jar\nRe-run with --force to replace files managed by init."
    }
})

private fun writeTinyJar(path: java.nio.file.Path, vararg classes: Pair<String, Int>) {
    ZipOutputStream(Files.newOutputStream(path)).use { zip ->
        classes.forEach { (name, classVersion) ->
            val writer = ClassWriter(0)
            writer.visit(classVersion, Opcodes.ACC_PUBLIC, name, null, "java/lang/Object", null)
            writer.visitEnd()
            zip.putNextEntry(ZipEntry("$name.class"))
            zip.write(writer.toByteArray())
            zip.closeEntry()
        }
    }
}
