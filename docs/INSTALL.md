# Installation

The standalone Sporeflower JAR requires Java 17 or newer. The `j2me` toolkit
requires Java 21 or newer.

## Standalone decompiler

Download [sporeflower.jar](https://github.com/hourianto/sporeflower/releases/download/continuous/sporeflower.jar) and run:

```sh
java -jar sporeflower.jar input.jar output/
```

API declarations are embedded in this JAR. The engine detects J2ME API references
or a `MicroEdition-Configuration` manifest entry and selects matching declarations
in memory. No companion SDK directory or disk cache is needed. Ordinary desktop
Java inputs do not automatically receive a CLDC core.
Use `--add-external=path/to/sdk.jar` for additional API definitions, or
`--bundled-j2me-api=false` to disable embedded API loading. The toolkit's authored
maps and built-in semantic mapping packs remain features of the `j2me` workflow.

## Download and install

Download [sporeflower.zip](https://github.com/hourianto/sporeflower/releases/download/continuous/sporeflower.zip) and extract it. It includes the CLI, decompiler engine, dependencies, and documentation.

Add the extracted directory's `bin/` directory to your `PATH`, then check the installation:

```sh
j2me doctor
```

## Getting started

Create a project from a JAR:

```sh
j2me init --project my-project --jar input.jar
```

This creates the project, extracts resources, and decompiles the program. Add names and annotations in `my-project/mappings/*.map`, then regenerate the source:

```sh
j2me remap --project my-project
```

Generated Java is in `my-project/decompiled/`; reports and remapped bytecode are in `my-project/out/`. See the [mapping reference](MAPPINGS.md) for syntax and examples. Use `j2me <command> --help` for command options.

## API stubs and compilers

The engine JAR includes generated, compile-only declarations for
CLDC, MIDP, optional JSRs (including M3G), and vendor extensions. They enable API
semantic mappings and supply external types for decompilation and compile checks.
They cannot run a J2ME application. API variants remain separate so the resolver
can match the original bytecode.

Older compilers and original SDK binaries are not included. When needed, put
additional local inputs in the installation directory:

* `vendor/j2me-api/` — additional API JARs, considered alongside the bundled declarations
* `vendor/j2me-stubs/src/main/java/` — additional local declaration sources
* `vendor/compilers/legacy-javac/legacy-javac.jar` — legacy compiler
* `vendor/compilers/ecj/ecj.jar` — optional ECJ compiler

Recompilation can also use the current JDK with `j2me compile-stubs --compiler javac`.

Compile checks keep intermediate classes in `out/compile_check/classes` and
restore original runtime names into `out/compile_check/restored/classes`.
Use the restored directory for comparison against the original JAR, without a
name mapping. This does not package or preverify an application.
Generated Java keeps original reflection strings for this purpose. To retain
renamed reflection strings instead, use `j2me remap --renamed-class-strings`;
those sources must be regenerated before restoration can be used.

In a source checkout, `./gradlew :toolkit:installDist` compiles declaration sources
from `toolkit/vendor/j2me-stubs/src/main/java/` into
`toolkit/vendor/j2me-api/local-api-stubs.jar` using the legacy compiler.
This makes them available to the decompiler as well as the compile check. Vendor
sources, libraries, and compilers remain local and are excluded from release archives.
The bundled declarations are generated directly from the repository's text
catalogs; building them does not require these local inputs or an old compiler.
`doctor` shows the number of bundled and local API libraries. `J2ME_BASE` changes
local asset paths while preserving the installation's bundled APIs.

The toolkit resolves overlapping API definitions against original bytecode member
descriptors, including return types and static/instance calls. It uses a dedicated
CLDC library where available, considering the declared configuration, inherited
calls, and floating-point requirements. The bundled catalog covers the API inputs
used by this project; applications targeting other SDK versions may need additional
libraries. A MIDP version declaration alone does not determine optional APIs.
Decompilation and mapping use these libraries in memory. External compile checks
write their selected classpath to `.cache/api/`, with a per-class provider list in
`META-INF/j2me-api-sources.tsv`. Class definitions are selected intact, without
adding methods to libraries or project classes.
An explicit `compile-stubs --api-jars-dir PATH` replaces the default API search
with that directory. See `META-INF/j2me-api/README.md` inside the engine JAR for
catalog provenance and regeneration instructions.

## Configuration

The bundled decompiler works without configuration. For overrides, copy the installed `config/global.example.toml` to `config/global.toml`, or set `J2ME_CONFIG` to another TOML file. Reinstalling preserves `global.toml`.

To produce mappings, reports, and remapped bytecode without decompiling, set:

```toml
[decompiler]
enabled = false
```

Raw remapping requires decompilation to be enabled.

To exclude a project from automatic `j2me fullrun --root /path/to/corpus` runs,
add this to the project's `j2me.toml`:

```toml
[fullrun]
enabled = false
```

Projects are enabled by default. Excluded projects do not count toward `--limit`
or run totals. An explicit `--project NAME` (relative to `--root`) or
`--project /path/to/project` overrides this setting; individual commands such as
`remap` also remain available. The next complete fullrun removes excluded projects
from the current history snapshot while keeping their earlier Git history.

## Building from source

From the repository root, run `./gradlew :toolkit:installDist`. The installation is written to `toolkit/build/install/j2me/`; add its `bin/` directory to `PATH` as above. Local inputs under gitignored `toolkit/vendor/` are copied into this installation but excluded from release archives.

To build only the decompiler engine, run `./gradlew jar`. Its JAR is written to `build/libs/` and requires Java 17 or newer.
