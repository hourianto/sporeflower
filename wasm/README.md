# Sporeflower WebAssembly API

This crate exposes the in-memory [Session API](../src/session.rs) through
`wasm-bindgen`. The host supplies class bytes and optional manifest/mapping text,
then requests Java source by original class name. JAR extraction, resource
handling and UI state belong to the host. Native CLI builds do not use this crate.

## Build

Use Rust 1.88 or newer with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli`
0.2.130 and Binaryen's `wasm-opt`. Run these commands from the repository root:

```sh
cargo build --manifest-path wasm/Cargo.toml --target wasm32-unknown-unknown --release --locked
wasm-bindgen --target web --out-dir wasm/pkg wasm/target/wasm32-unknown-unknown/release/sporeflower_wasm.wasm
wasm-opt --strip-debug --strip-producers wasm/pkg/sporeflower_wasm_bg.wasm -o wasm/pkg/sporeflower_wasm_bg.wasm
```

Use the CLI version matching the `wasm-bindgen` dependency in
[Cargo.toml](Cargo.toml) to generate compatible bindings.

The output in `wasm/pkg/` includes `sporeflower_wasm.js`,
`sporeflower_wasm_bg.wasm` and TypeScript declarations. Keep the JavaScript and
Wasm from the same build together. The final step removes Wasm debug and producer
metadata.

For development, omit `--release`, use the `debug/` Wasm path, and skip `wasm-opt`.

## Use

Import the generated ES module and await its default initializer before creating
a session. Calls after initialization are synchronous; use a Web Worker to keep
decompilation off the UI thread. For example, with class bytes supplied as a
`Uint8Array`:

```js
import init, { SporeflowerSession } from "./pkg/sporeflower_wasm.js";

await init();
const session = new SporeflowerSession();
try {
  const originalName = session.addClass("pkg/Main.class", classBytes);
  const metadata = JSON.parse(session.prepare("{}"));
  const result = JSON.parse(session.decompile(originalName));
  // Consume result.source and inspect result.diagnostics / result.failedMethods.
} finally {
  session.free();
}
```

Add all application classes before preparing the session. Class identities use
JVM internal names such as `pkg/Main`, including after renaming; entry paths only
label parse errors. Duplicate identities are rejected.

| Method | Contract |
| --- | --- |
| `addClass(path, bytes)` | Parse one class and return its original internal name. |
| `setManifest(text)` | Replace the manifest; its CLDC configuration guides bundled API selection. |
| `setMappings(json)` | Replace mapping inputs with a JSON `SessionMappings` object. |
| `prepare(optionsJson)` | Return `classes`, `diagnostics` and `apiClasses`. Each class has its original `name`, its mapped binary `sourceName` and the `sourceUnit` that owns its Java file. |
| `decompile(originalName)` | Return the containing compilation unit with `source`, `diagnostics`, `failedClasses`, `totalMethods`, `failedMethods`, `fallbackMethods`, `inspectionFallbackMethods` and `simplifications`. |
| `mappingTiny()` | Return the original-to-source Tiny v2 mapping as text. |
| `domainSources()` | Return a JSON array of `{name, source}` semantic domain sources. |

To export Java files, group the prepared classes by `sourceUnit`, decompile
one original name per group, and write the result to `sourceUnit + ".java"`.
The source map records explicit field declarations. It omits a recovered member's
implicit enclosing-instance field, whose binary name is chosen by the Java compiler.

`prepare("{}")` uses `renameObfuscated: false`, `renameComments: true`,
`builtinMappings: true`, `bundledApi: true` and `preserveBytecode: false`.
Set `preserveBytecode: true` to inspect retained reads, local stores and control
flow. Switching this option reuses the session’s parsed classes, names and
shared analyses. In class results, `simplifications.folded_decisions` and
`simplifications.redundant_field_reads` count the corresponding IR rewrites.
If a method needs simplified Java, `inspectionFallbackMethods` counts it
and `diagnostics` gives the reason. Options and mappings reject unknown fields. Mapping inputs
accept authored text in `sources: [{path, source}]` and portable `classes`,
`fields` and `methods` renames. See the [mapping guide](../docs/mappings.md),
[SessionMappings](../src/session.rs) and [rename schemas](../src/session/renames.rs)
for their formats; method parameter indexes are JVM local-variable slots.

Successful input changes invalidate preparation. Call `prepare` again before
decompiling or exporting mappings/domain sources. Repeating it with unchanged
inputs and options reuses the prepared analysis. Invalid requests throw a JavaScript
`Error`. A valid compilation-unit request reports declaration and method failures
in `failedClasses`, `failedMethods` and `diagnostics`, including units containing
only one class.

This boundary uses bundled [API declarations](../api/README.md); it does not
expose the native filesystem, project commands or additional classpaths. The
[wrapper](src/lib.rs) defines the JavaScript exports, while the
[session tests](../src/session/tests.rs) exercise the shared lifecycle and mapping
behavior.
