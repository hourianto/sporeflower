package j2me.map

import java.nio.file.Path

internal data class BuiltinSemanticMapSource(
    val path: Path,
    val displayPath: String,
    val source: String,
)

private data class BuiltinMapResource(
    val name: String,
    val requiredOwners: Set<String>,
)

private fun builtinMap(name: String, vararg requiredOwners: String) =
    BuiltinMapResource(name, requiredOwners.toSet())

private val builtinMapResources = listOf(
    builtinMap("cldc/connector.map", "javax/microedition/io/Connector", "javax/microedition/io/Connection"),
    builtinMap("midp/command.map", "javax/microedition/lcdui/Command"),
    builtinMap("midp/canvas.map", "javax/microedition/lcdui/Canvas"),
    builtinMap("midp/font.map", "javax/microedition/lcdui/Font"),
    builtinMap(
        "midp/graphics.map",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image",
    ),
    builtinMap("midp/sprite.map", "javax/microedition/lcdui/game/Sprite"),
    builtinMap(
        "midp/graphics-region.map",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image",
        "javax/microedition/lcdui/game/Sprite",
    ),
    builtinMap("midp/game-canvas.map", "javax/microedition/lcdui/game/GameCanvas"),
    builtinMap("midp/record-store.map", "javax/microedition/rms/RecordStore"),
    builtinMap("optional-jsrs/media-player.map", "javax/microedition/media/Player"),
)

private const val resourceRoot = "/j2me/builtin-mappings/"

internal fun loadBuiltinSemanticMapSources(knownClasspathClasses: Set<String>): List<BuiltinSemanticMapSource> =
    builtinMapResources
        // A partial or optional API must not make otherwise valid projects fail
        // because one of its authoritative declarations cannot be resolved.
        .filter { resource -> resource.requiredOwners.all { it in knownClasspathClasses } }
        .map { resource ->
            val resourcePath = "$resourceRoot${resource.name}"
            val source = ResourceMarker::class.java.getResourceAsStream(resourcePath)?.bufferedReader()?.use { it.readText() }
                ?: error("Missing built-in semantic mapping resource: $resourcePath")
            BuiltinSemanticMapSource(
                path = Path.of("<builtin-mappings>", resource.name),
                displayPath = "builtin-mappings/${resource.name}",
                source = source,
            )
        }

// Keep resource lookup independent of Kotlin's generated file facade name.
private object ResourceMarker
