# Architecture

Sporeflower is one Gradle project with a decompiler engine, its renaming plugin,
and the `j2me` CLI. This reference explains their responsibilities and the main
processing stages. Installation and configuration are in [INSTALL.md](INSTALL.md);
mapping syntax is in [the mapping reference](MAPPINGS.md).

## Code map

| Location | Responsibility |
| --- | --- |
| `src/org/jetbrains/java/decompiler/` | Class-file input, analysis, Java reconstruction, and engine API |
| `plugins/variable-renaming/` | Variable naming plugin packaged with the engine |
| `toolkit/src/main/kotlin/j2me/` | Project configuration, maps, reports, bytecode renaming, and compilation orchestration |
| `toolkit/src/main/resources/j2me/builtin-mappings/` | Semantic descriptions of supported APIs |
| `test/`, `testFixtures/`, `testData/` | Engine tests, shared test helpers, and bytecode/source fixtures |
| `toolkit/src/test/` | Toolkit unit tests and complete workflow tests |

The engine keeps the Vineflower-derived source layout. Its standalone JAR
contains engine code, required libraries, and explicitly selected plugins.
The CLI is a normal consumer of that artifact; it is not an embedded plugin.
Gradle builds the matching engine before CLI packaging and integration tests.

## Project processing

`RemapPipeline.kt` coordinates the project workflow:

```text
Input JAR + authored maps + available API stubs
                      |
            Symbol analysis and map validation
                      |
       Renamed JAR, Tiny names, semantic facts, reports
                      |
             Decompiler API and Java output
```

Symbol analysis reads class and member identities and usage. The map parser
resolves readable declarations against those identities; validation catches
missing members, type mismatches, and source-level naming conflicts before
replacing existing output.

The bytecode remapper writes a renamed JAR for project use. Decompilation reads
the original JAR with the generated Tiny names, preserving the bytecode the
engine needs to analyze. Resource extraction is a separate CLI operation.

## Semantic facts and engine calls

The toolkit turns validated annotations into `SemanticMappingData`, an input
type in the engine API. Owners and descriptors use mapped names; parameter
indices count declared parameters rather than local-variable slots.

Scoped call bindings additionally retain original bytecode offsets and mapped
callee identities. The symbol reader records invocation offsets and persists
them in the versioned symbol cache. The decompiler consumes the original input
JAR, so bytecode rewriting for the separately emitted renamed JAR cannot shift
these offsets. Call bindings never use inherited-member lookup.

Array semantics include per-dimension repeated-record layouts. The record
access analyzer proves index residues under JVM integer overflow and preserves
uncertainty around headers. Flag domains may define exclusive selector masks;
constant rendering keeps unknown selectors numeric instead of combining their
bits into unrelated enum alternatives.

`SemanticContext` captures immutable expression keys and lexical guard facts
before printable variables merge. Local definitions retain range and record
alignment facts only when their sources agree; incoming parameters, unknown
writes and cyclic dependencies cannot establish alignment on their own.
Conditional domains and bit-field selectors
use these keys, while range proofs support non-overflowing record indexes and
field planes. Container roles remain separate from scalar and array meanings;
supported boxing and collection operations transfer only the declared role.
String tokens and numeric presentation use the same conflict resolution as
integral constants. Numeric presentation retains the integer value and type;
fixed-point decoding is an explanatory comment, not a program transformation.

`VineflowerRunner.kt` normally calls `Decompiler.Builder` directly, passing
semantic facts in memory. The engine turns them into its lookup and propagation
model. Neither component negotiates a schema with the other. The JSON reader
and writer support subprocess transport, standalone command-line input and
explicit `remap --export-semantic-map` inspection output.

API semantic packs activate when their owner classes are available. Their
declarations describe constants and annotated call sites; project maps can
reuse the domains and supply additional application-specific bindings.

Each decompilation uses its own context. The CLI initializes the bundled engine
once before concurrent corpus workers create their contexts. Corpus concurrency
and the engine's method concurrency are separate controls; increasing both
multiplies CPU demand.

## Method reconstruction

The engine represents expressions as **exprents**, statements as a structured
graph, and branches between statements as edges. `MethodProcessor` coordinates
the transformations. Several stages repeat as earlier rewrites expose new
structure, so the following is a guide to responsibilities, not a rigid pass list.

| Stage | Main code | Purpose |
| --- | --- | --- |
| Read bytecode | `StructMethod`, instruction sequences | Decode instructions and class-file metadata |
| Build control flow | `ControlFlowGraph`, `DeadCodeHelper`, exception-range handling | Form basic blocks and normalize legacy jumps and handlers |
| Recover statements | `DomHelper`, postdominance analysis | Structure branches into loops, conditionals, switches, and exits |
| Recover cleanup | `FinallyProcessor`, synchronized processing | Recognize duplicated cleanup and monitor patterns |
| Recover expressions | `ExprProcessor`, `StackVarsProcessor`, SSA/SSAU forms | Model stack values, follow assignments and uses, and simplify temporaries |
| Refine source | Loop/branch helpers, `VarProcessor`, `VarDefinitionHelper` | Improve structure and choose printable variables, scopes, and types |
| Render semantics | `SemanticConstantsProcessor` | Propagate domains and replace eligible constants with symbolic names |
| Write Java | `ClassWriter`, statement and exprent writers | Emit declarations and method bodies |

Legacy programs may reuse local slots across incompatible types or contain
unusual exception ranges. A source-level symptom can therefore originate in
bytecode normalization, variable analysis, or rendering. Fix the responsible
stage rather than relying on output text rewriting.

Semantic facts are captured before printable local merging loses expression
identity and rendered after final cleanup. The fixed point distinguishes a
literal or unevaluated definition from an unknown dynamic value, so unknown
alternatives cannot silently inherit another branch's domain. Constant contexts
are collected before rendering; conflicting requests leave the literal numeric.
Generated constant interfaces are saved alongside decompiled classes.

## Compilation and regression checks

`CompileStubs.kt` selects a compiler, constructs the API classpath, and compiles
generated source. Its local stub cache is separate from project source output.
This checks whether the emitted Java can be compiled against the selected API
surface; it does not prove semantic equivalence.

`FullrunCommand.kt` runs decompilation and compilation over a selected corpus.
It defaults to raw mode; `j2me fullrun --mapped --root /path/to/corpus` exercises
authored names and semantic mappings in the same scratch workspaces. Use a
separate `--history-dir` when comparing mapped runs with raw runs.
`FullrunHistory.kt` normalizes source snapshots and records compact status
changes. Mapping integration tests also include behavioral comparisons after
recompilation, since compilable symbolic output can still change numeric meaning.
