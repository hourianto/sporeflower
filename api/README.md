# J2ME API declarations

The JSON catalogs supply external class and member declarations for J2ME analysis
and compilation. Cargo's [build script](../build.rs) generates classfile stubs and
an indexed declaration pack from them; normal builds need no SDK JARs or Java
toolchain. [Provenance](PROVENANCE.md) lists the declaration sources.

## Selection and export

The resolver chooses API variants using the input's member descriptors and
static/instance access. Application declarations take precedence, followed by
explicit `--classpath` libraries; later classpath entries override earlier ones.
`--stubs api.jar` exports selected catalog classes and their dependencies.
Exported JARs have
`J2ME-Stub-Kind: compile-only`: concrete methods throw immediately, while abstract
and native methods retain their flags. These are compilation libraries, not emulators.

## Missing classes

Decoded allocations and class method invocations can establish enough information
to export a placeholder for a missing class, including static-only utility APIs.
It contains only observed members; conflicting references prevent inference.
Descriptor-only types without class-kind constraints receive opaque compile
shapes, with their unknown kind recorded in the export metadata. Inherited member
references can extend an absent superclass placeholder; they remain unresolved
during analysis. The assumed superclass and empty throws list never feed back
into type or exception analysis. Supply the real SDK through `--classpath` when
that missing information matters.

The JAR marks the presence of placeholders with `J2ME-Stub-Fallback: true` and
`J2ME-Inferred-Stub-Count`. `META-INF/sporeflower-inferred-api.json` records their
observed members and unknown properties; `META-INF/sporeflower-api.tsv` lists the
selected classes and their catalog providers.

## Catalog format

Each JSON file represents an API variant. Variants stay separate because SDKs may
disagree on return types, checked exceptions or static/instance access.
`fallback: true` marks partial declarations, which lose ties to complete ones.

The [schema](../src/api/schema.rs) records class versions, access flags, names,
descriptors, inheritance, nesting, optional signatures, checked exceptions and
field constants. Constructors retain their visibility. Other private members,
static initializers, method bodies, debug information and SDK resources are
omitted. Floating-point constants use hexadecimal IEEE bits; integer constants
use decimal strings. Record the source in [PROVENANCE.md](PROVENANCE.md) when
adding or updating declarations. Application call sites belong to per-input
stub inference, not the bundled catalog.
