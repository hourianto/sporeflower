# Installation

Sporeflower requires Java 21 or newer and provides the `j2me` command.

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

API stubs and older compilers are not included. When needed, put your local copies in the installation directory:

* `vendor/j2me-api/` — CLDC, MIDP, and optional API stub JARs
* `vendor/compilers/legacy-javac/legacy-javac.jar` — legacy compiler
* `vendor/compilers/ecj/ecj.jar` — optional ECJ compiler

Recompilation can also use the current JDK with `j2me compile-stubs --compiler javac`.

## Configuration

The bundled decompiler works without configuration. For overrides, copy the installed `config/global.example.toml` to `config/global.toml`, or set `J2ME_CONFIG` to another TOML file. Reinstalling preserves `global.toml`.

To use another decompiler, set `SPOREFLOWER_JAR` or configure `vineflower.bin`. Relative JAR paths resolve beside the configuration file; external decompilers run as subprocesses.

## Building from source

From the repository root, run `./gradlew :toolkit:installDist`. The installation is written to `toolkit/build/install/j2me/`; add its `bin/` directory to `PATH` as above. Local inputs under gitignored `toolkit/vendor/` are copied into this installation but excluded from release archives.

To build only the decompiler engine, run `./gradlew jar`. Its JAR is written to `build/libs/` and requires Java 17 or newer.
