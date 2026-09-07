package j2me.cli

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.symbols.AnalysisCachePaths
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.io.path.writeBytes

class RemapPipelineTest : FunSpec({
    test("invalid mappings leave the previous output intact") {
        val root = Files.createTempDirectory("remap-preserve-output")
        val jar = root.resolve("game.jar")
        ZipOutputStream(Files.newOutputStream(jar)).use { zip ->
            zip.putNextEntry(ZipEntry("a.class"))
            zip.write(ClassWriter(0).apply {
                visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
                visitEnd()
            }.toByteArray())
            zip.closeEntry()
        }
        val mapsDir = root.resolve("mappings").createDirectories()
        mapsDir.resolve("Broken.map").writeText("class Broken { int value /* was a */; }\n")
        val outDir = root.resolve("out").createDirectories()
        val marker = outDir.resolve("last-good.txt")
        marker.writeText("keep\n")

        shouldThrow<IllegalArgumentException> {
            runRemapPipeline(
                RemapPipelineArgs(
                    jar = jar,
                    mapsDir = mapsDir,
                    outDir = outDir,
                    classpathSymbolsByClass = emptyMap(),
                    overwriteOutputDir = true,
                    writeIndex = null,
                    raw = false,
                    noComments = false,
                    analysisWorkers = 1,
                    cache = AnalysisCachePaths(root.resolve("symbols.json"), root.resolve("usage.json")),
                    decompiler = null,
                ),
                RecordingDecompilerRunner(),
            )
        }

        marker.readText() shouldBe "keep\n"
    }

    test("raw remap requires decompilation instead of succeeding as a no-op") {
        val root = Files.createTempDirectory("raw-remap-no-decompiler")
        val jar = root.resolve("game.jar")
        jar.writeBytes(byteArrayOf(0))
        val runner = RecordingDecompilerRunner()

        val exc = shouldThrow<IllegalArgumentException> {
            runRemapPipeline(
                RemapPipelineArgs(
                    jar = jar,
                    mapsDir = root.resolve("mappings"),
                    outDir = root.resolve("out"),
                    classpathSymbolsByClass = emptyMap(),
                    overwriteOutputDir = true,
                    writeIndex = null,
                    raw = true,
                    noComments = false,
                    analysisWorkers = 1,
                    cache = AnalysisCachePaths(
                        symbols = root.resolve(".cache/remap-symbols.json"),
                        usage = root.resolve(".cache/remap-usage.json"),
                    ),
                    decompiler = null,
                ),
                runner,
            )
        }

        exc.message shouldContain "Raw remap requires decompilation"
        runner.invocation shouldBe null
    }

    test("disabled semantic mappings preserve ordinary remapping and omit the decompiler sidecar") {
        val root = Files.createTempDirectory("remap-no-semantics")
        val jar = root.resolve("game.jar")
        ZipOutputStream(Files.newOutputStream(jar)).use { zip ->
            zip.putNextEntry(ZipEntry("a.class"))
            zip.write(ClassWriter(0).apply {
                visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
                visitField(Opcodes.ACC_PRIVATE, "b", "I", null, null).visitEnd()
                visitEnd()
            }.toByteArray())
            zip.closeEntry()
        }
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
            RemapPipelineArgs(
                jar = jar,
                mapsDir = mapsDir,
                outDir = root.resolve("out"),
                classpathSymbolsByClass = emptyMap(),
                overwriteOutputDir = true,
                writeIndex = null,
                raw = false,
                noComments = false,
                semanticMappingsEnabled = false,
                analysisWorkers = 1,
                cache = AnalysisCachePaths(root.resolve("symbols.json"), root.resolve("usage.json")),
                decompiler = DecompilerConfig(
                    output = root.resolve("decompiled"),
                    external = emptyList(),
                ),
            ),
            decompilerRunner,
            quiet = true,
        )

        result.decompilerMs shouldBe 123L
        decompilerRunner.invocation?.logStdoutPath shouldBe root.resolve("out/decompiler.stdout.log")
        decompilerRunner.invocation?.logStderrPath shouldBe root.resolve("out/decompiler.stderr.log")
        result.mappingPath?.exists() shouldBe true
        result.remappedJar?.exists() shouldBe true
        root.resolve("out/semantic-map.json").exists() shouldBe false
        decompilerRunner.invocation?.options?.containsKey("mappings-path") shouldBe true
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
