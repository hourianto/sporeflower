# Bundled API declarations

These catalogs contain class and member declarations used for J2ME analysis and
compile checks. `./gradlew generateApiStubs` builds class resources under
`build/generated/api-resources/META-INF/j2me-api/`. The engine JAR embeds these
resources, grouped by API variant, together with an index and this document.
Normal builds use only these text files.

Each JSON file represents one API variant. Keep variants separate: some SDKs
define the same method with different return types, checked exceptions, or
static/instance access. The engine's shared `J2meApi` resolver selects complete class
definitions against the input bytecode. `fallback: true` marks supplementary,
partial declarations; those lose ties to complete SDK declarations. Compatible
local SDK binaries take precedence over generated declarations. Standalone and
toolkit decompilation read the selected declarations in memory through
`IContextSource`. No API directories, extracted stub JARs or disk caches are
needed for decompilation. External compile checks still write their selected
classpath to the toolkit's existing compile inputs.

## Format and generation

The catalog records class-file versions, JVM access flags, names, descriptors,
superclasses, interfaces, nesting, optional generic signatures, checked exceptions,
and field constants. Constructors retain their visibility, including private
constructors. Other private members, static initializers, method bodies, debug
information, and SDK resources are omitted. Floating-point constants use their
raw IEEE bits in hexadecimal; integer constants use decimal strings.

The generator is `buildSrc/src/main/groovy/org/vineflower/apistubs/ApiStubs.groovy`.
It uses ASM to emit declarations and two-instruction throwing bodies for concrete,
non-native methods. Abstract and native methods keep their original flags and
have no bodies. These are analysis libraries, not runtime implementations.

To import updated declarations, place the input JARs in a local directory and run
`./gradlew :toolkit:extractApiCatalog -PapiInputs=/path/to/api-jars`.
The result goes to `toolkit/build/api-catalog/`; review it before copying changed
JSON files here. With no override, the importer reads `toolkit/vendor/j2me-api/`.
Record the input's origin below when updating a catalog. Imported class and
member order is normalized; Gradle packages resources with fixed timestamps.

## Input provenance

Catalog filenames identify the local input JARs. This records where the
declarations came from; regenerating them is not a statement that the original
libraries or their declarations have been relicensed.

* **MicroEmulator 2.0.4:** `cldcapi10`, `cldcapi11`, `midpapi20`,
  `microemu-jsr-75`, `microemu-jsr-82`, `microemu-jsr-120`, `microemu-jsr-135`,
  `microemu-nokiaui`, and `microemu-siemensapi`, all with the `-2.0.4` suffix.
  These are the matching `org.microemu` Maven artifacts. Upstream release:
  <https://github.com/barteo/microemu/tree/microemulator_2_0_4>.
* **KEmulator v2.21.4 UEI API inputs:** `mascotv3-2.21.4`, `jsr256-sensor`,
  `jsr211-sdk`, `jsr177-crypto`, `pigler-api`, `nokiaui-sdk`, and `samsung-api`.
  Original UEI names were `mascotv3.jar`, `jsr256.jar`, `jsr211.jar`,
  `jsr177_crypto.jar`, `javapiglerapi.jar`, `nokiaui.jar`, and `samsungapi.jar`.
  <https://github.com/shinovon/KEmulator/releases/tag/v2.21.4>.
* **Nokia S40:** `nokia-s40-internal-api` was imported from the
  `com/nokia/mid/s40/`, `com/nokia/mid/impl/jms/`, `com/nokia/mid/ui/lcdui/`, and
  `com/nokia/mid/ui/s40/` entries of `S40v6/jar/cldc11.jar` at
  <https://github.com/0x726564/S40-SDK-Classes/tree/decec09b9479214ce790ed3ad2e9e07ef084bdc6>.
* **Supplementary declarations:** `local-api-stubs` contains 11 partial external
  contracts. Motorola file mutations, Siemens lighting methods, the TextEditor
  interface, IAPLib, and advertising wrapper descriptors were checked against
  original external bytecode references. MMPP BackLight, Sprint Location, Samsung
  LCDLight, and TextEditorUtils were checked against KEmulator declarations.
  BGUtils also uses the S40 declarations and
  <https://github.com/mozilla/pluotsorbet/blob/master/java/custom/com/nokia/mid/s40/bg/BGUtils.java>.
  The legacy Motorola exception contract was checked against
  <https://github.com/rorist/maps-lib-nutiteq/blob/c9d0217f32d7c151afde166af5136a3c8a2ebc3f/android/src/com/motorola/io/FileConnection.java>.
  Private client identities are not part of the catalog.
* **Earlier local API imports with incomplete upstream provenance:**
  `blackberry-api-7.1.0`, `jsr179-location`, `jsr180-sip`, `jsr184`,
  `jsr211-chapi`, `jsr226-svg`, `jsr229-payment`, `jsr234-amms`, `jsr238-global`,
  `jsr75-pim`, `motorola-api`, `motorola-io-file`, `siemens-extra-api`,
  `siemens-jsr75-file`, and `vodafone-api`. The original archive locations were
  not retained. Their catalog declarations were imported from the correspondingly
  named local API JARs; no stronger provenance is claimed.
