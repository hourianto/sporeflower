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
    builtinMap("midp/text-field.map", "javax/microedition/lcdui/TextField"),
    builtinMap("midp/text-box.map", "javax/microedition/lcdui/TextField", "javax/microedition/lcdui/TextBox"),
    builtinMap("midp/choice.map", "javax/microedition/lcdui/Choice"),
    builtinMap("midp/list.map", "javax/microedition/lcdui/Choice", "javax/microedition/lcdui/List", "javax/microedition/lcdui/Image"),
    builtinMap("midp/choice-group.map", "javax/microedition/lcdui/Choice", "javax/microedition/lcdui/ChoiceGroup", "javax/microedition/lcdui/Image"),
    builtinMap("midp/item.map", "javax/microedition/lcdui/Item"),
    builtinMap("midp/string-item.map", "javax/microedition/lcdui/Item", "javax/microedition/lcdui/StringItem"),
    builtinMap("midp/image-item.map", "javax/microedition/lcdui/Item", "javax/microedition/lcdui/ImageItem", "javax/microedition/lcdui/Image"),
    builtinMap("midp/alert.map", "javax/microedition/lcdui/Alert"),
    builtinMap("midp/gauge.map", "javax/microedition/lcdui/Gauge"),
    builtinMap("midp/date-field.map", "javax/microedition/lcdui/DateField", "java/util/TimeZone"),
    builtinMap("midp/display.map", "javax/microedition/lcdui/Display"),
    builtinMap("midp/image-transform.map", "javax/microedition/lcdui/Image", "javax/microedition/lcdui/game/Sprite"),
    builtinMap("midp/image-colors.map", "javax/microedition/lcdui/Image", "javax/microedition/lcdui/Graphics"),
    builtinMap("cldc/http.map", "javax/microedition/io/HttpConnection"),
    builtinMap("cldc/socket.map", "javax/microedition/io/SocketConnection"),
    builtinMap("midp/record-comparator.map", "javax/microedition/rms/RecordComparator"),
    builtinMap("optional-jsrs/m3g-compositingmode.map", "javax/microedition/m3g/CompositingMode"),
    builtinMap("optional-jsrs/m3g-polygonmode.map", "javax/microedition/m3g/PolygonMode"),
    builtinMap("optional-jsrs/m3g-texture2d.map", "javax/microedition/m3g/Texture2D"),
    builtinMap("optional-jsrs/m3g-graphics3d.map", "javax/microedition/m3g/Graphics3D"),
    builtinMap("optional-jsrs/m3g-image2d.map", "javax/microedition/m3g/Image2D"),
    builtinMap("optional-jsrs/m3g-background.map", "javax/microedition/m3g/Background"),
    builtinMap("optional-jsrs/m3g-light.map", "javax/microedition/m3g/Light"),
    builtinMap("optional-jsrs/m3g-fog.map", "javax/microedition/m3g/Fog"),
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
