# Testing

The test suite uses [bundled fixtures](../tests/fixtures/README.md); no external
game corpus or verifier is needed. Java round trips compile a fixture, decompile
it, rebuild the generated source, and compare execution in separate classpaths.

Use **JDK 17** for the full suite. The round trips need Java 7 source/target
support, which JDK 20 and newer removed. Some tests also use
`--release`, which JDK 8 does not support.

```sh
JAVA_HOME=/path/to/jdk-17 cargo test --locked
```

The test helpers select `JAVA` / `JAVAC` first, then `JAVA_HOME/bin`, then `PATH`.
Keep `java` and `javac` on the same JDK; missing tools or invalid overrides fail
the tests.

Optional Java 1.3 compilation checks need a
[separately prepared compiler JAR](projects.md#compile-for-older-java-targets).
Set `LEGACY_JAVAC=/path/to/legacy-javac.jar` to enable them. Without it, those
checks are skipped; the checked-in bytecode fixtures still run.

## Focused test groups

Integration tests share one binary. Select a module while working on a specific
area, for example:

```sh
cargo test --test integration roundtrip::
```

- `roundtrip::` checks execution behavior after decompilation and recompilation.
- `source_quality::` protects readable Java idioms with focused source assertions
  and runtime checks.
- `project_commands::` covers initialization, remapping, resources and compilation.
- `semantic_mappings::` covers authored meanings and their effect on generated code.

## Adding a regression

For a Java behavior regression, add a small fixture under `tests/fixtures/java/`
and a test in the relevant module. `tests/common.rs` provides compilation,
execution and round-trip helpers; `fixture_roundtrip` is enough for a
self-contained Java main class. Give each test its own temporary output directory
and register new modules in `tests/integration.rs`.

Use JASM or retained class files when the bytecode pattern cannot be produced by
a modern Java compiler. Keep the source and provenance beside the fixture;
recompiling a legacy fixture can erase the pattern it exists to test.

For readability regressions, inspect the emitted method before adding assertions
to `source_quality`. Assert the intended idiom and the degradation it should
prevent, rather than snapshotting the whole method or preserving a known flaw.

For checks against a separately obtained game collection, see the
[corpus tools](../tools/README.md).
