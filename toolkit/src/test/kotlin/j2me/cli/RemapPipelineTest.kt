package j2me.cli

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.engine.spec.tempdir
import j2me.testing.classJar
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.symbols.AnalysisCachePaths
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.io.path.writeBytes

class RemapPipelineTest : FunSpec({
    val temporary = tempdir("remap-pipeline").toPath()
    test("invalid mappings leave the previous output intact") {
        val root = Files.createTempDirectory(temporary, "remap-preserve-output")
        val jar = root.resolve("game.jar")
        classJar(jar, mapOf("a" to ClassWriter(0).apply {
            visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
            visitEnd()
        }.toByteArray()))
        val mapsDir = root.resolve("mappings").createDirectories()
        mapsDir.resolve("Broken.map").writeText("class Broken { int value /* was a */; }\n")
        val outDir = root.resolve("out").createDirectories()
        val marker = outDir.resolve("last-good.txt")
        marker.writeText("keep\n")

        shouldThrow<IllegalArgumentException> {
            runRemapPipeline(
                pipelineArgs(root),
                RecordingDecompilerRunner(),
            )
        }

        marker.readText() shouldBe "keep\n"
    }

    test("raw remap requires decompilation instead of succeeding as a no-op") {
        val root = Files.createTempDirectory(temporary, "raw-remap-no-decompiler")
        val jar = root.resolve("game.jar")
        jar.writeBytes(byteArrayOf(0))
        val runner = RecordingDecompilerRunner()

        val exc = shouldThrow<IllegalArgumentException> {
            runRemapPipeline(
                pipelineArgs(root).copy(raw = true),
                runner,
            )
        }

        exc.message shouldContain "Raw remap requires decompilation"
        runner.invocation shouldBe null
    }

    test("disabled semantic mappings preserve ordinary remapping and omit the decompiler sidecar") {
        val root = Files.createTempDirectory(temporary, "remap-no-semantics")
        val jar = root.resolve("game.jar")
        classJar(jar, mapOf("a" to ClassWriter(0).apply {
            visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
            visitField(Opcodes.ACC_PRIVATE, "b", "I", null, null).visitEnd()
            visitEnd()
        }.toByteArray()))
        val mapsDir = root.resolve("mappings").createDirectories()
        mapsDir.resolve("Entity.map").writeText(
            """
            @ValueDomain interface Direction { int LEFT = -1; }
            class Entity /* was a */ {
                @Domain(Direction.class) int direction /* was b */;
            }
            """.trimIndent() + "\n",
        )
        val decompilerRunner = RecordingDecompilerRunner()

        val result = runRemapPipeline(
            pipelineArgs(root).copy(
                semanticMappingsEnabled = false,
                decompiler = DecompilerConfig(output = root.resolve("decompiled"), external = emptyList()),
            ),
            decompilerRunner,
            quiet = true,
        )

        result.decompiled?.elapsedMs shouldBe 123L
        decompilerRunner.invocation?.logStdoutPath shouldBe root.resolve("out/decompiler.stdout.log")
        decompilerRunner.invocation?.logStderrPath shouldBe root.resolve("out/decompiler.stderr.log")
        result.mappings?.tinyPath?.exists() shouldBe true
        result.mappings?.remappedJar?.path?.exists() shouldBe true
        root.resolve("out/semantic-map.json").exists() shouldBe false
        decompilerRunner.invocation?.preparedNames?.classes()?.isNotEmpty() shouldBe true
        decompilerRunner.invocation?.options?.containsKey("semantic-mappings-path") shouldBe false
    }
})

private class RecordingDecompilerRunner : DecompilerRunner {
    var invocation: DecompilerInvocation? = null

    override fun run(invocation: DecompilerInvocation): Long {
        this.invocation = invocation
        return 123L
    }
}

private fun pipelineArgs(root: Path) = RemapPipelineArgs(
    jar = root.resolve("game.jar"),
    mapsDir = root.resolve("mappings"),
    outDir = root.resolve("out"),
    classpathSymbolsByClass = emptyMap(),
    overwriteOutputDir = true,
    writeIndex = null,
    raw = false,
    noComments = false,
    analysisWorkers = 1,
    cache = AnalysisCachePaths(root.resolve("symbols.json"), root.resolve("usage.json")),
    decompiler = null,
)
