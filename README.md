# Sporeflower

A Rust decompiler and mapping toolkit for J2ME applications. Turn JARs into
readable Java, then use mappings to give obfuscated code meaningful names.
Decompilation needs no Java installation or SDK.

## Download

Download the [latest release](https://github.com/hourianto/sporeflower/releases/latest)
or [build from source](docs/building.md).

## Decompile a JAR

```sh
sporeflower game.jar
```

Read the result in `decompiled/`, or use `-o directory` to choose another location.
Each run clears the source output directory. Copy the files elsewhere before
editing them by hand.

For bytecode inspection, run `sporeflower game.jar --preserve-bytecode`.
The default favors concise Java. Inspection keeps repeated field reads, unused
local stores, discarded computations and control decisions visible where Java
can express them. A stack-only choice can appear as `?:`, and consecutive
element stores into a fresh array can appear as an array literal. Local updates
reuse one Java variable when the previous value is no longer live. The same
flag works with `init`, `decompile` and `remap`.

When class metadata and constructor bytecode establish nesting, the enclosing
Java file includes its member classes. For example, a recovered `Outer$Member`
appears as `Outer.Member` in source and rebuilds to `Outer$Member.class`.
Recognized local and anonymous classes can share that file as package-private
declarations with explicit capture fields and constructors.

## Work with mappings

Create a project wherever you like; `init` copies your JAR into it:

```sh
mkdir game
cd game
sporeflower init --jar /path/to/game.jar
```

Add `.map` files in `mappings/` to name classes, methods, fields and values.
The [mapping guide](docs/mappings.md) explains the format with examples.
After editing your maps, regenerate the sources:

```sh
sporeflower remap
```

Sources go to `decompiled/` and the renamed JAR to `out/game-mapped.jar`.
Project commands use the current directory by default; use `--project path/to/game`
to select a project from elsewhere.
For other commands and options, run `sporeflower --help` or
`sporeflower <command> --help`.

## Documentation

- [Project configuration](docs/projects.md)
- [Testing](docs/testing.md) and [architecture](docs/architecture.md) for contributors
- [WebAssembly API](wasm/README.md) for browser integrations

The original Sporeflower, a Vineflower fork, is available on the
[legacy branch](https://github.com/hourianto/sporeflower/tree/legacy).

> [!NOTE]
> AI usage disclosure: Sporeflower is developed by AI agents.

## License

[Apache-2.0](LICENSE).
Sources for the bundled API declarations are listed in [API provenance](api/PROVENANCE.md).
