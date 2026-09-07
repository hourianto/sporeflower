package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.tomlj.Toml
import java.nio.file.Path

class ToolkitInstallationTest {
    @TempDir lateinit var temporary: Path

    @Test fun `base override preserves the bundled engine and explicit config path`() {
        val home = temporary.resolve("relocated installation")
        val base = temporary.resolve("custom assets")
        val config = temporary.resolve("settings/local.toml")
        val paths = toolkitPaths(home, base.toString(), config.toString())
        assertEquals(base, paths.base)
        assertEquals(config, paths.globalCfg)
        assertEquals(base.resolve("templates/mappings-doc.md"), paths.mappingsDocTemplate)
        assertEquals(home.resolve("decompiler/sporeflower.jar"), paths.bundledDecompiler)
    }

    @Test fun `decompilation is enabled by default and can be disabled in configuration`() {
        val paths = toolkitPaths(temporary, null, null)
        for ((config, enabled) in listOf(null to true, "true" to true, "false" to false)) {
            val global = config?.let { Toml.parse("[decompiler]\nenabled = $it\n") }
            val args = buildRemapPipelineArgs(
                temporary, paths, global, temporary.resolve("input.jar"), raw = true, noComments = false,
            )
            assertEquals(enabled, args.decompiler != null)
        }
    }
}
