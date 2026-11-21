//! CLDC/MIDP, optional JSR and handset API contracts. The original packs come
//! from the [legacy toolkit](https://github.com/hourianto/sporeflower/tree/legacy).
use super::{MappingSource, parser, resolve};
use crate::Result;
use std::sync::OnceLock;

pub(super) struct BuiltinPack {
    pub required: &'static [&'static str],
    path: &'static str,
    text: &'static str,
    parsed: OnceLock<std::result::Result<ParsedPack, String>>,
}

pub(super) struct ParsedPack {
    pub source: parser::ParsedSource,
    pub external_owners: Vec<String>,
}

impl BuiltinPack {
    pub fn parsed(&self) -> Result<&ParsedPack> {
        self.parsed
            .get_or_init(|| {
                let source = parser::parse(&MappingSource {
                    path: self.path.into(),
                    source: self.text.into(),
                    builtin: true,
                })
                .map_err(|error| format!("{error:#}"))?;
                let external_owners: Vec<String> = source
                    .declarations
                    .iter()
                    .filter(|declaration| resolve::has(&declaration.annotations, "External"))
                    .map(|declaration| {
                        resolve::qualified(&source.package, &declaration.name).replace('.', "/")
                    })
                    .collect();
                Ok(ParsedPack {
                    source,
                    external_owners,
                })
            })
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error}"))
    }
}

macro_rules! pack {
    ($file:literal, $($owner:literal),+ $(,)?) => {{
        static PACK: BuiltinPack = BuiltinPack {
            path: concat!("builtin-mappings/", $file),
            text: include_str!(concat!("builtin/", $file)),
            required: &[$($owner),+],
            parsed: OnceLock::new(),
        };
        &PACK
    }};
}

pub(super) fn packs(enabled: bool) -> &'static [&'static BuiltinPack] {
    if enabled { PACKS } else { &[] }
}

static PACKS: &[&BuiltinPack] = &[
    pack!("cldc/calendar.map", "java/util/Calendar"),
    pack!("cldc/thread.map", "java/lang/Thread"),
    pack!(
        "cldc/connector.map",
        "javax/microedition/io/Connector",
        "javax/microedition/io/Connection"
    ),
    pack!("midp/command.map", "javax/microedition/lcdui/Command"),
    pack!("midp/canvas.map", "javax/microedition/lcdui/Canvas"),
    pack!("midp/font.map", "javax/microedition/lcdui/Font"),
    pack!(
        "midp/graphics.map",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image"
    ),
    pack!("midp/sprite.map", "javax/microedition/lcdui/game/Sprite"),
    pack!(
        "midp/graphics-region.map",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image",
        "javax/microedition/lcdui/game/Sprite"
    ),
    pack!(
        "midp/game-canvas.map",
        "javax/microedition/lcdui/game/GameCanvas"
    ),
    pack!(
        "midp/record-store.map",
        "javax/microedition/rms/RecordStore"
    ),
    pack!(
        "optional-jsrs/media-player.map",
        "javax/microedition/media/Player"
    ),
    pack!(
        "optional-jsrs/video-control.map",
        "javax/microedition/media/control/GUIControl",
        "javax/microedition/media/control/VideoControl"
    ),
    pack!("midp/text-field.map", "javax/microedition/lcdui/TextField"),
    pack!(
        "midp/text-box.map",
        "javax/microedition/lcdui/TextField",
        "javax/microedition/lcdui/TextBox"
    ),
    pack!("midp/choice.map", "javax/microedition/lcdui/Choice"),
    pack!(
        "midp/list.map",
        "javax/microedition/lcdui/Choice",
        "javax/microedition/lcdui/List",
        "javax/microedition/lcdui/Image"
    ),
    pack!(
        "midp/choice-group.map",
        "javax/microedition/lcdui/Choice",
        "javax/microedition/lcdui/ChoiceGroup",
        "javax/microedition/lcdui/Image"
    ),
    pack!("midp/item.map", "javax/microedition/lcdui/Item"),
    pack!(
        "midp/string-item.map",
        "javax/microedition/lcdui/Item",
        "javax/microedition/lcdui/StringItem"
    ),
    pack!(
        "midp/image-item.map",
        "javax/microedition/lcdui/Item",
        "javax/microedition/lcdui/ImageItem",
        "javax/microedition/lcdui/Image"
    ),
    pack!("midp/alert.map", "javax/microedition/lcdui/Alert"),
    pack!("midp/gauge.map", "javax/microedition/lcdui/Gauge"),
    pack!(
        "midp/date-field.map",
        "javax/microedition/lcdui/DateField",
        "java/util/TimeZone"
    ),
    pack!("midp/display.map", "javax/microedition/lcdui/Display"),
    pack!(
        "midp/image-transform.map",
        "javax/microedition/lcdui/Image",
        "javax/microedition/lcdui/game/Sprite"
    ),
    pack!(
        "midp/image-colors.map",
        "javax/microedition/lcdui/Image",
        "javax/microedition/lcdui/Graphics"
    ),
    pack!("cldc/http.map", "javax/microedition/io/HttpConnection"),
    pack!("cldc/socket.map", "javax/microedition/io/SocketConnection"),
    pack!(
        "midp/record-comparator.map",
        "javax/microedition/rms/RecordComparator"
    ),
    pack!(
        "optional-jsrs/bluetooth-discovery.map",
        "javax/bluetooth/DiscoveryAgent"
    ),
    pack!(
        "optional-jsrs/bluetooth-local-device.map",
        "javax/bluetooth/LocalDevice",
        "javax/bluetooth/DiscoveryAgent"
    ),
    pack!(
        "optional-jsrs/bluetooth-listener.map",
        "javax/bluetooth/DiscoveryListener"
    ),
    pack!(
        "optional-jsrs/bluetooth-service-record.map",
        "javax/bluetooth/ServiceRecord"
    ),
    pack!(
        "optional-jsrs/bluetooth-data-element.map",
        "javax/bluetooth/DataElement"
    ),
    pack!(
        "optional-jsrs/pim.map",
        "javax/microedition/pim/PIM",
        "javax/microedition/pim/PIMList"
    ),
    pack!(
        "optional-jsrs/m3g-animationtrack.map",
        "javax/microedition/m3g/AnimationTrack",
        "javax/microedition/m3g/KeyframeSequence"
    ),
    pack!(
        "optional-jsrs/m3g-keyframesequence.map",
        "javax/microedition/m3g/KeyframeSequence"
    ),
    pack!(
        "optional-jsrs/m3g-material.map",
        "javax/microedition/m3g/Material"
    ),
    pack!(
        "optional-jsrs/m3g-compositingmode.map",
        "javax/microedition/m3g/CompositingMode"
    ),
    pack!(
        "optional-jsrs/m3g-polygonmode.map",
        "javax/microedition/m3g/PolygonMode"
    ),
    pack!(
        "optional-jsrs/m3g-texture2d.map",
        "javax/microedition/m3g/Texture2D"
    ),
    pack!(
        "optional-jsrs/m3g-graphics3d.map",
        "javax/microedition/m3g/Graphics3D"
    ),
    pack!(
        "optional-jsrs/m3g-image2d.map",
        "javax/microedition/m3g/Image2D"
    ),
    pack!(
        "optional-jsrs/m3g-background.map",
        "javax/microedition/m3g/Background"
    ),
    pack!(
        "optional-jsrs/m3g-light.map",
        "javax/microedition/m3g/Light"
    ),
    pack!("optional-jsrs/m3g-fog.map", "javax/microedition/m3g/Fog"),
    pack!(
        "nokia/direct-graphics.map",
        "com/nokia/mid/ui/DirectGraphics",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image"
    ),
    pack!(
        "nokia/direct-utils.map",
        "com/nokia/mid/ui/DirectUtils",
        "javax/microedition/lcdui/Graphics",
        "javax/microedition/lcdui/Image"
    ),
    pack!("nokia/sound.map", "com/nokia/mid/sound/Sound"),
    pack!("samsung/audio-clip.map", "com/samsung/util/AudioClip"),
];
