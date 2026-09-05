# Installation

The `j2me` toolkit requires Java 21 or newer. The standalone decompiler requires Java 17 or newer.

## Building from source

From the repository root:

```sh
./gradlew :toolkit:installDist
export PATH="$PWD/toolkit/build/install/j2me/bin:$PATH"
j2me doctor
```

This builds the toolkit and matching decompiler under `toolkit/build/install/j2me/` and puts the generated launcher on `PATH` for the current shell. To install elsewhere, copy that directory and add its `bin/` directory to your `PATH` instead.

For the standalone decompiler only, build with `./gradlew jar`. The JAR is written to `build/libs/`:

```sh
java -jar /path/to/sporeflower.jar input.jar output/
```

## Getting started

Create a project from a JAR:

```sh
j2me init --project /path/to/project --jar /path/to/input.jar
```

This creates `j2me.toml`, extracts resources, and decompiles the program. Add names and annotations in `mappings/*.map`, then regenerate the source:

```sh
j2me remap --project /path/to/project
```

Generated Java is in `decompiled/`; reports and remapped bytecode are in `out/`. See the [mapping reference](MAPPINGS.md) for syntax and examples. Use `j2me <command> --help` for command options.

## API stubs and compilers

API stubs and older compilers are not distributed with Sporeflower. In a source checkout, local copies go in:

* `toolkit/vendor/j2me-api/` — CLDC, MIDP, and optional API stub JARs
* `toolkit/vendor/compilers/legacy-javac/legacy-javac.jar` — legacy compiler
* `toolkit/vendor/compilers/ecj/ecj.jar` — optional ECJ compiler

`installDist` copies these into the installation's `vendor/` directory. With a packaged installation, place them directly in `vendor/` using the same layout. Recompilation can also use the current JDK with `j2me compile-stubs --compiler javac`.

## Configuration

The bundled decompiler works without configuration. For overrides, copy the installed `config/global.example.toml` to `config/global.toml`, or set `J2ME_CONFIG` to another TOML file. Reinstalling preserves `global.toml`.

To use another decompiler, set `SPOREFLOWER_JAR` or configure `vineflower.bin`. Relative JAR paths resolve beside the configuration file; external decompilers run as subprocesses.
