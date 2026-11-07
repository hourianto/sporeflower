# Architecture

Sporeflower turns legacy JVM bytecode into Java source and can also rename the
original class files. Both outputs use original JVM class/member identities for
analysis and one `NamePlan` for output names. Java source recovery and native JAR
rewriting then follow different paths.

This guide follows those paths and explains the boundaries that matter when
changing them. Command usage is in the [README](../README.md); mapping syntax is in
[Mappings](mappings.md), and external declarations are described in
[API declarations](../api/README.md).

## Preparing a program

`src/main.rs` dispatches commands; `src/project.rs` handles project workflows.
The decompilation pipeline in `src/decompile.rs` loads application classes,
classpaths, API declarations and mappings, constructs a `Program`, resolves the
mappings, and allocates names. Names are fixed before class workers start.
`NamePlan` also requests initialization analysis, so building names can analyze
method bodies. The CLI prepares checked-exception facts in its selected worker
pool, then emits classes in parallel. Methods within a class share imports and
support members and are processed in order.

`Input` owns the parsed application classes. `Program` borrows these and the
library declarations, and owns hierarchy indexes and cached analysis facts.
Declarations keep their original names throughout this lifetime. The caches need
no invalidation while those declarations remain immutable; method working graphs
are created separately for each analysis or source-lowering request.

For in-memory callers, `src/session.rs` provides the same preparation boundary.
A `Session` accepts class bytes, a manifest and mappings, then `prepare` builds the
program, resolved mappings and names for repeated class requests. Changing an
input invalidates that prepared state. Repeating `prepare` with unchanged options
reuses it; failed preparation retains parsed classes so the caller can correct
its inputs and retry. `wasm/src/lib.rs` exposes this path to browser hosts.

## From a method to Java

The main entry is `java::decompile_class`; its method path crosses these boundaries:

| Stage | Input and work | Result / code entry |
|---|---|---|
| Decode and build control flow | Decode the method's `Code` attribute, normalize legacy JSR subroutines, and retain ordered exception handlers. | `cfg::PreparedCode`: instructions, control-flow graph and original bytecode locations. See `src/cfg/prepared.rs`. |
| Lift and schedule | Simulate JVM frames, join values and recover expressions using program facts and selected initialization triggers. Choose local storage while preserving operation order and exception state. | `ir::MethodIr`: values, blocks and locals. See `src/ir/mod.rs` and `src/ir/schedule.rs`. |
| Prepare Java target operations | Normalize constructor field writes, preserve linkage failures, and lower allocations that Java cannot express directly. | A Java-target graph and allocation helpers. See `src/java/prepare.rs`. |
| Freeze statement locations | Split nonthrowing prefixes from protected blocks so Java can see assignments before their `try` scopes. | Immutable `structure::PreparedIr`, bundled with helpers in `java::prepare::PreparedBody`. See `src/structure/prepare.rs`. |
| Recover control flow | Prove cleanup/monitor regions and express branches, loops, switches and handlers as a tree. Select a fallback representation when ordinary regions do not fit. | `structure::StructuredMethod`, referring to the prepared graph. See `src/structure/mod.rs`. |
| Plan and bind source | Settle lexical scopes, declarations and source idioms; then resolve expression types, conversions, overloads, qualification and checked exceptions. | An IR-free syntax plan. See `src/java/source.rs`, `src/java/source/bind.rs` and `src/java/expr.rs`. |
| Write text | Render bound expressions and lay out syntax lines. | Java method text. See `src/java/syntax.rs` and `src/java/body.rs`. |

Constructor and interface initialization lowering may extract additional method
graphs. Each extracted graph needs its own `PreparedIr` and source plan.
`ir::fragments` handles rebinding values, locals and blocks;
`java::constructors`, `java::allocations` and `java::interfaces` own the respective
Java representations.

### Values, storage and locations

`ValueId`, `LocalId` and `BlockId` identify different things: a computed value,
mutable storage, and a control-flow block. A JVM `dup` creates another reference
to the same value. It does not authorize evaluating the value's producer twice.
Scheduled assignments record this distinction in `ValueUse`: `Definition`
evaluates a producer, while `Reference` reads assigned storage when present.
Transforms and helper extraction must preserve it.

Lifting first produces a mutable `RawIr`. Its edit methods keep statement origins
aligned when statements are emitted, removed or split. Scheduling introduces
storage boundaries and copies, and reconciles storage types and lifetimes. Use
`ir::scheduled`, `ir::local_effects` and the scheduled liveness analysis when
examining that representation; raw JVM slot liveness answers a different question.

Statement positions can change during graph rewrites. Only `PreparedIr` freezes
the positions used by structured statements, cleanup proofs and source plans.
If a later lowering changes the graph, build a new prepared graph and its plans.
Bytecode positions are a separate coordinate system: JSR normalization can clone
instructions, and `PreparedCode::original_pc` maps them back to input offsets.
Do not carry a scheduled statement position into a fresh lift of the method.

### Effects and exceptions

Scheduling must preserve calls, observable reads, writes and exceptions, including
the value of a local saved before an update. An exception edge observes the state
at its throwing instruction, with earlier writes committed and later writes
absent. Expression movement must also preserve class initialization: Java lowering
supplies explicit trigger sites when moving an allocation or inlining a constant
field read would move the original active use. Discovery and native analysis do
not insert those source triggers. A graph built for one of these contexts is not
a reusable result for every other analysis.

Exception handling has three views, with different consumers:

- `IrBlock::handlers` retains ordered lexical protection, including shadowed
  entries that can establish an enclosing scope.
- `cfg::dispatch` projects executable edges using first-matching-handler rules.
  Dataflow uses this view.
- Java catch eligibility depends on the final protected body and its checked
  effects. Source planning applies this without discarding the original handlers
  needed by cleanup proofs.

`finally` and `synchronized` require proofs of cleanup and ownership. Equivalent
cleanup must cover every outward path before it becomes `finally`. Monitor
recovery must account for acquisitions and releases; it cannot invent a release
to make malformed bytecode look lexical. Cleanup ownership settles before final
source continuations are projected. See `src/structure/finally/`,
`src/structure/synchronized.rs` and `src/structure/installation.rs`.

### Java planning and generated helpers

Java type rules depend on the expression that will actually be emitted. Binding
therefore owns conversions and overload selection, including conditional
expressions and narrow storage. Truth tests and boolean values also differ: a
nonzero test is not always equivalent to the conversion of a raw JVM boolean
slot. Use the canonical-value facts and the consumer's test/value context when
folding them. The text writer has no program-analysis context and must not make
these decisions.

Generated calls need exact signatures and separate runtime/source exception
effects in `java::generated`. Use its shared symbol allocation so helper names
cannot collide with mapped members, existing calls or visible types. Planning a
body can request helpers, diagnostics and imports; `EmissionContext::artifact`
keeps those requests isolated until `commit` publishes a successful candidate to
the class. Abandoning a candidate must leave no support requests behind. Dispatch
used inside an extracted helper contributes to its owning method's fallback status.

Lowering also preserves runtime rules that Java syntax may obscure. Known missing
members retain their linkage failure after operand evaluation. An incomplete
hierarchy cannot prove that a member is missing. Final-field source eligibility
must respect original write ownership and class initialization order, even when
the emitted field has to become mutable. Exact resource-exhaustion timing,
including `OutOfMemoryError`, is outside the model.

## Mappings and native output

`mappings` binds names and semantic contracts to original JVM members before
source emission. `semantics` interprets those contracts over scheduled expression
occurrences and reaching definitions, rather than printable local names. Its
results guide typed Java expressions; the renderer does not infer meanings from
strings. Array shapes, container contents and packed layouts are independent
facts, and unknown or conflicting evidence must not become a unique meaning.

Native remapping starts with `Input::read_for_remap`, which retains original class
and resource bytes beside the parsed declarations. `Program::from_input` borrows
that input for analysis and rewriting. Ordinary source-only loading omits these
bytes. Detaching parsed classes with `Input::into_classes` drops the retained bytes.

`src/remap/` patches references in the original class files. Existing instruction
widths, bytecode offsets, exception ranges and opaque attributes remain intact.
Supported class-name consumers may need appended adapter methods with verifier
metadata appropriate to the original class-file version. Shared literal producers
and consumers must agree on relocation or adaptation before edits are committed;
ordinary text uses must remain ordinary text. Resource aliases preserve relative
lookups when packages move. Java-only helpers and semantic domain declarations
do not become native bytecode declarations.

## Recovery, failures and output

Control-flow recovery tries ordinary regions, lexical labels and bounded entry
duplication before using explicit runtime dispatch. These are alternative
representations of the graph, not successive simplifications of one tree.
`StructuredMethod::recovery_attempts` records rejected strategies and selected
fallbacks; speculative pattern probes do not belong in that history. Limits can
force storage, select another representation, or fail a method, so they need an
explicit contract at the phase that enforces them.

Failure to prove `finally` can leave ordinary cleanup code. Unrecovered monitor
operations fail source lowering. A method reconstruction error produces a
diagnostic and an explicit error body; other methods can still be emitted.
A dispatch body is a successful fallback and is counted separately from failed
methods. Fatal command errors stop the request, and Rust panics are not caught by
these `Result` boundaries.

The CLI validates artifact paths and reads inputs before clearing generated
output directories. `src/artifact_plan.rs` checks role collisions and input aliases;
`src/output.rs` implements disposable directories and direct file writes. A failed
source write can leave partial output for the next run to replace. Native JAR
writing uses temporary-file replacement after all classes succeed.

`src/compile_inputs.rs` saves API, classpath and bytecode-version settings in the
source tree. `src/compiler.rs` consumes that context without rebuilding mappings
or requiring the original JAR. Project-owned paths are stored relative to the
source tree so that moving the project preserves them.
