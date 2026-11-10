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
method bodies. The CLI prepares initialization facts before naming, then Boolean
and checked-exception facts, using its selected worker pool. Native sessions
prepare the same facts in the current Rayon pool; browser sessions use serial
analysis. The CLI then emits classes in parallel. Methods within a class share
imports and support members and are processed in order.

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
| Lower to source | Bind expressions while lowering control flow; resolve conversions, overloads and qualification, prepare defaults, and record intrinsic checked effects. | One bound source tree and declaration requirements. See `src/java/source/lower.rs`, `src/java/source/bind.rs` and `src/java/expr.rs`. |
| Simplify source | Settle choices and loops, substitute disposable bindings and simplify expressions using source-local definitions, uses and evaluation effects. | The same tree, independent of scheduled IR. See `src/java/source/simplify.rs`. |
| Finalize Java scopes | Place declarations and initializers, then settle checked catches and method effects from surviving operations. | Source with final scopes and checked effects. See `src/java/source_locals.rs` and `src/java/source/effects.rs`. |
| Write text | Render bound statements and expressions directly through `CodeWriter`. | Java method text. See `src/java/source/bound.rs`, `src/java/syntax.rs` and `src/java/body.rs`. |

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
storage boundaries and copies, and reconciles storage types and lifetimes.
`ir::execution` owns ordered scheduled evaluation; local effects and semantic
occurrences are folds over it. `ir::scheduled` provides dependency walks instead
of execution traces. Scheduled liveness and raw JVM slot liveness answer different
questions.

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

Bound calls and allocations carry their intrinsic checked effects; explicit
throws carry theirs separately from the operand. Final source effect collection
runs after simplification, so deleting an operation cannot leave stale catch
eligibility. It uses the program's type hierarchy without rereading method IR.

`finally` and `synchronized` require proofs of cleanup and ownership. Equivalent
cleanup must cover every outward path before it becomes `finally`. Monitor
recovery must account for acquisitions and releases; it cannot invent a release
to make malformed bytecode look lexical. Cleanup ownership settles before final
source continuations are projected. A cleanup witness still used by other
protected paths remains in the graph with its rethrow. A complete scope may own
failure-only cleanup whose normal exits are proved no-ops. Partial recovery needs
physical normal cleanup copies; a shared continuation alone stays after its
catch. A partial scope inside an existing try must consolidate multiple copies.
See `src/structure/finally/`,
`src/structure/synchronized.rs` and `src/structure/installation.rs`.

### Java planning and generated helpers

Java type rules depend on the expression that will actually be emitted. Binding
therefore owns conversions and overload selection, including conditional
expressions and narrow storage. Truth tests and boolean values also differ: a
nonzero test is not always equivalent to the conversion of a raw JVM boolean
slot. Use the canonical-value facts and the consumer's test/value context when
folding them. The text writer has no program-analysis context and must not make
these decisions.

Lowering constructs bound source directly, retaining local identities, apparent
Java types, literal values, Boolean proofs and explicit conversions. Temporary
identity and field-publication facts support default bindings and final-field
checks. Static initializers retain publication identities for declaration recovery;
binding clears them from other bodies before source simplification.
Local writes retain one target across assignment, compound and increment forms.
Final simplification works on these trees without consulting bytecode or
scheduled values. `source/evaluation.rs` owns the shared ordered expression
traversal, counting actual reads and writes and distinguishing assignment targets,
conditional evaluation and effects. Binding, declaration placement and checked
effects use this traversal directly. Removing a binding preserves its
value conversion; the consumer determines which explicit casts remain. Calls
retain types that select overloads, while assignments can use implicit conversions.
Calls, fields, arrays, returns and monitor expressions use the same substitution
rules; shared snapshots and named source bindings can retain their storage.
`source/lifetimes.rs` owns structured transfers and the continuations supplied to
nested rewrites. Handler liveness includes the code reached after a catch or
cleanup, including enclosing break and continue targets.
`source/normalize.rs` owns value identities and contextual conversions in one
expression pipeline. `source/simplify.rs` coordinates substitution, choice
formation and loop recovery, updating method occurrence counts as statements
change. Choice presentation finishes before loop presentation reads those counts;
finalized child scopes are not run again when a parent exposes a new expression.
Declaration placement follows this normalization, so new initializers and loop
exits get their actual source scopes. Scheduled loop-site recognition and larger
iteration proofs remain in preparation; final loop spelling uses bound facts.

Static initialization becomes an ordered sequence of field declarations and
static blocks. Recovery splits only where no local spans the boundary; it can
resume after that local's last use. Runtime assignments to final primitive or
String fields stay in blocks when a declaration would create a compile-time
constant and change initialization behavior.

Boolean local recovery follows producers and uses together: copies, joins,
eager Boolean operations and equality comparisons can share Boolean storage;
arithmetic, indices and numeric signatures require numeric values. Program analysis
reduces each original method to dependencies between fields, parameters and
returns, then propagates these compact summaries. Invocation targets are shared
with exception analysis. Loose classes and library inputs remain partial;
application scope assumes the supplied classes contain the application's callers
and writers. Input manifests with supplied entry classes select that scope;
in-memory callers can select it explicitly. Platform callbacks and missing
hierarchy information remain external inputs. Extracted helpers carry caller
identity separately from original or captured parameter provenance.

Raw expression recovery normalizes stack values and Boolean predicates before
structuring; other storage joins stay structured. The source planner owns the
complete decision tree when choosing expressions, assignment ladders and guarded
updates. Evaluation-order checks stay in the IR; presentation policy does not.
Handler-observed locals are computed once for each frozen method.

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
