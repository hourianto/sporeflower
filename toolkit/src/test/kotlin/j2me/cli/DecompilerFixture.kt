package j2me.cli

import j2me.process.RealProcessRunner
import java.nio.file.Path
import kotlin.io.path.createDirectories

internal fun testToolkitPaths(root: Path): ToolkitPaths = ToolkitPaths(
    root, root.resolve("global.toml"), root.resolve("guide.md"),
)

internal fun decompileForRestoration(root: Path, input: Path, options: Map<String, String> = emptyMap()): Path {
    root.resolve("mappings").createDirectories()
    val paths = testToolkitPaths(root)
    val args = buildRemapPipelineArgs(root, paths, null, input, raw = false, noComments = true).copy(
        restorationAudit = true, decompilerOptions = options + ("__unit_test_mode__" to "true"),
    )
    runRemapPipeline(args, SporeflowerRunner(paths, RealProcessRunner()), quiet = true)
    return root.resolve("decompiled")
}
