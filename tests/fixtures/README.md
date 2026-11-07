# Test fixtures

`java/` contains sources compiled by the tests. The groups below retain bytecode
patterns that modern javac may not reproduce. See the
[testing guide](../../docs/testing.md) for prerequisites and test commands.

| Directory | Coverage | Tests |
| --- | --- | --- |
| `constructors/` | Values and effects before `super` or `this`, reused argument slots, and values live after initialization | [constructors.rs](../constructors.rs) |
| `exception/` | Exception-table priority, shadowed handlers, shared rethrows and cleanup paths | [structure_exceptions.rs](../structure_exceptions.rs) |
| `legacy/` | Evaluation order, allocation identity, stack values across slot reuse and array post-increments | [roundtrip.rs](../roundtrip.rs) |
| `legacy-jsr/` | Finally blocks implemented with `jsr`/`ret` subroutines | [roundtrip.rs](../roundtrip.rs) |

The `constructors/`, `exception/` and `legacy/` class files were assembled from
JASM sources in `testData/` on the
[legacy branch](https://github.com/hourianto/sporeflower/tree/legacy)
(Apache-2.0). Sources are included in each group's `jasm/` directory.
`constructors/support/pkg/TestConstructorPreludeInput.java` and
`TestConstructorPreludeBase.java` also come from that branch; the effect-order
helper and constructor drivers are local fixtures.

The `constructors/boundary/` fixture writes through uninitialized `this`. It tests
that unsupported reconstruction fails explicitly instead of moving the write
after `super`.

`legacy-jsr/classes/LegacyFinally.class` was compiled from
[LegacyFinally.java](legacy-jsr/LegacyFinally.java) with
[JDK 1.4.2_30 javac](../../docs/projects.md#compile-for-older-java-targets), using
`-XDjsrlimit=0 -source 1.3 -target 1.1`. Tests use the retained class;
recompiling it with modern javac would remove the subroutine pattern.
