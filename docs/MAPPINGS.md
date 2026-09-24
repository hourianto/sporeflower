# Mapping reference

Write Java-like declarations in `mappings/*.map`, normally one class per file,
to rename bytecode entities and attach semantic meanings. These are not
compilable Java. See the [installation guide](https://github.com/hourianto/sporeflower/blob/master/docs/INSTALL.md)
for setup; `j2me init` links this reference as project guidance.

## Workflow

1. Use `out/coverage.md` to find unnamed classes and map them first.
2. Use `out/usage-priority.md` to choose members; its TSV has full JVM identities.
3. Edit the maps, run `j2me remap`, and resolve reported errors.
4. Inspect generated Java, checking both named substitutions and remaining literals.

`out/semantic-summary.md` lists contracts and domain references, not substitution counts.

Routine mapping work needs only `remap`. Edit maps, not generated Tiny mappings,
Java or reports. Investigate diagnostics that contradict the input rather than
inventing renames to satisfy them.

Conflicting explicit requests are mapping errors. Clashes with existing names
or automatic placement can receive suffixes or other repairs to keep Java
declarations and runtime bindings consistent. `out/mapping.tiny` records the
completed names used by Java, semantic bindings and the inspection JAR. Unnamed
classes share a default placement, including keyword-named classes that require
repairs.

## Names and types

```java
package sample.game;
import java.util.Hashtable;
import javax.microedition.lcdui.Graphics;

class GameEngine /* was af */ {
    GameEngine(int width, int height) {}
    int frameCounter /* was a */;
    Hashtable cache /* was b */;
    void draw(Graphics graphics, int x, int y) /* was a */;
    int tick(int frame, long delta) /* was c */;
}
```

Declarations give readable names and types; `/* was ... */` gives the original
class owner or simple member name. Owners may be simple, slash-separated or
fully qualified; package-relative owners resolve against input classes.
Original names may be Java keywords: `class State /* was do */ {}`.

Name every parameter. Use mapped project class names in types. Primitives,
arrays, qualified names and imports work; common `java.lang` types need no
import. Qualify ambiguous names. External types must exist in the available APIs.

Constructors use the mapped class name, an empty body and no member `was`
comment. Their parameter names and semantic bindings apply to `new`, `this`
and `super` calls.

Exclude classes whose bytecode names are already final from renaming coverage:

```java
package sample.game;
@AlreadyMapped class SettingsScreen {}
```

Use the bytecode package and class name, an empty body and no `was` marker.

## Choosing semantic annotations

Choose a contract whose scope matches the program: a whole value, array
position, condition or individual call. A matching number or readable local
name alone does not establish a meaning.

| Annotation | Use |
| --- | --- |
| `@ValueDomain` | Named integral values |
| `@FlagDomain` | Bit masks |
| `@SlotDomain` | Array positions or record fields |
| `@PackedDomain`, `@BitField(...)` | Fields within an encoded integer |
| `@NumericDomain(...)` | RGB, ARGB or scaled numeric formatting |
| `@StringDomain` | String tokens |
| `@ClassName` | Class-name string relocation |
| `@DomainValue(D.class)` | A real constant belonging to `D` |
| `@Domain(D.class)`, `@Flags(D.class)` | Scalar values or array leaf values |
| `@DomainFromParameter(n)` | A helper return with an argument's kind of value |
| `@DomainFromSlot(parameter = n, slot = k)` | Parameter/return meaning from a supplied table column |
| `@CallDomain(...)` | One specific call result or argument |
| `@DomainWhen(...)` | Parameter/return meaning selected by another argument |
| `@IndexDomain(...)` | Array index values |
| `@Slots(...)`, `@SlotValue(D.class)` | Fixed positions and their stored values |
| `@Records(...)`, `@Planes(...)` | Repeated records or parallel field planes |
| `@Elements(D.class)`, `@Keys(D.class)`, `@Values(D.class)` | J2ME container contents |

Use `@Domain` for value, packed, numeric or string domains; `@Flags` requires a
flag domain. Scalar, array and container bindings apply to fields, parameters
and returns. Parameter indexes count declared parameters from zero, excluding `this`.

`@DomainFromParameter`, `@DomainFromSlot` and `@DomainWhen` cannot share a
target with each other or a fixed contract.

Unannotated overrides can inherit method bindings. A nearer explicit binding
replaces the entire contract for that parameter or return: supplying only an
index domain drops an inherited leaf domain, so repeat it if needed. Conflicting
inherited contracts stay ambiguous. A field hiding an ancestor's field does not
inherit its meaning. `@CallDomain` bindings are not inherited.

Semantic contracts guide source presentation without changing stored values,
except `@ClassName`, which also relocates strings in the remapped JAR.
Generated Java keeps original class-name strings by default for restoration
after compilation; `remap --renamed-class-strings` selects renamed strings instead.

## Inference and limits

Meanings flow from producers and back from consumers through local copies,
compatible casts, agreeing branches and supported helpers. Return, parameter
and scoped-call contracts can name assignments to branch-built locals. Known
producers can name literal fallbacks that join their results. Conflicts or opaque
definitions suppress inference; printable local reuse does not join independent
bytecode values.

Definitions are selected at each read. If a branch replaces an array and sets
a flag, a later guard proving the flag false can retain the original array's
contract. Overwriting the flag or admitting another replacement can invalidate
the proof. Complex control flow can still leave literals unnamed.

An ascending or descending `for` counter with a constant nonnegative start and
constant step can carry an index domain inside a nonwrapping loop and into
selected values. Initializers, bounds and increments retain their arithmetic
meaning. Overflow proofs use the counter's byte, short, char or int storage.
Uses after the loop include its exit value and do not inherit the body-only proof.

Arithmetic deltas and wrap bounds remain numeric: `direction -= 2`
does not make `2` a direction. Bit-mask updates can name their operands. Standalone
ordered comparisons against zero retain numeric zero, including domains with
negative sentinels; equality comparisons and explicit intervals can name values.

## Constants and flags

Prefer existing constant fields:

```java
@ValueDomain interface Direction {}

class Entity /* was j */ {
    @DomainValue(Direction.class) static final int RIGHT /* was k */;
    @DomainValue(Direction.class) static final int LEFT /* was o */;
    @Domain(Direction.class) int direction /* was a */;
    void setDirection(@Domain(Direction.class) int value) /* was j */;
    @Domain(Direction.class) int getDirection() /* was q */;
}
```

`@DomainValue` requires a `static final` field with a bytecode `ConstantValue`:
integral for value/flag/slot domains, String for string domains. Only accessible
fields can supply names at a use site. Otherwise, declare source-only constants:

```java
@ValueDomain interface ItemKind {
    byte UNIT = 2;
    byte BUILDING = 3;
}
@FlagDomain interface InputMask {
    int UP = 1;
    int LEFT = 2;
}
class InputHandler /* was b */ {
    void update(@Flags(InputMask.class) int mask) /* was a */;
}
```

Generated interfaces appear beside the decompiled source, not in the JAR, and
must not collide with existing classes. Integral constants support `byte`, `short`,
`char`, `int` and `long` (except in slot domains). Values must be unique per domain;
names must be unique per generated interface.

Integral initializers and numeric annotation arguments must be literals,
optionally signed; expressions such as `1 << 4` or `BASE + 1` are unsupported.

Flags combine known bits with `|` and retain unknown bits numerically. Mark
mutually exclusive choices by their bit ranges:

```java
@FlagDomain(exclusiveMasks = {0x0f}) interface Options {
    int TEXT = 1;
    int NUMBER = 2;
    int PASSWORD = 0x100;
}
```

`0x102` can become `Options.NUMBER | Options.PASSWORD`; the unknown choice in
`0x103` stays numeric instead of becoming `TEXT | NUMBER`. Exclusive masks must
be nonzero and disjoint. Use `@BitField` for extracted bits with a separate meaning.
Signed masks may use byte/short casts to preserve sign extension.

## Helpers and individual reads

Use `@DomainFromParameter` for a helper returning the same kind of quantity as
an argument:

```java
class NumericHelper /* was e */ {
    @DomainFromParameter(0) static int absolute(int value) /* was b */;
}
```

Both must be integral scalars. This preserves meaning, not necessarily the number;
results with different meanings, such as lengths, hashes, signs or comparisons,
need a separate contract.

For a generic table helper, derive each call's meanings from the supplied array:

```java
class Lookup /* was lu */ {
    @DomainFromSlot(parameter = 0, slot = 1)
    int find(byte[] pairs, @DomainFromSlot(parameter = 0, slot = 0) int key) /* was a */;
}
```

The source must be an integral array, the target an integral parameter or return.
For multidimensional tables, specify the innermost `dimension`, e.g.
`@DomainFromSlot(parameter = 0, dimension = 1, slot = 1)` for `int[][]` rows.
Omit `dimension` only on 1D arrays. `slot` selects a `@SlotValue` column from the
table's `@Records`, `@Planes` or `@Slots` contract, falling back to its leaf domain.
Record columns exclude the header. Each call uses its table's meanings while
the helper body stays generic. The shape may come from another mapped use or
return in the same method, without annotating the helper's array parameter.

For a meaning specific to one call, annotate its containing method:

```java
class Decoder /* was d */ {
    @CallDomain(value = ItemKind.class, offset = 17)
    void decode() /* was a */;
}
```

Find the original invoke instruction's byte offset with
`javap -c -p -classpath original.jar OriginalClass`. It is not a source line or
instruction ordinal. The bound value must be compatible with the domain.
Repeat `@CallDomain` for different calls; it overrides the general return binding
for that call only. Add `parameter = n` to bind argument `n` instead, including
constructor arguments. Separate arguments and the result may share an offset.
For methods with scoped contracts, `out/semantic-summary.md` lists invocation
identities and offsets. Recheck offsets if the input JAR changes.

## Arrays, records and planes

Indexes, fixed positions and stored values are separate meanings:

```java
@ValueDomain interface PlayerIndex { int FIRST = 0; int SECOND = 1; }
@SlotDomain interface RecordSlot {
    int OWNER = 0;
    @SlotValue(ItemKind.class) int KIND = 1;
}
class World /* was f */ {
    @IndexDomain(value = PlayerIndex.class, dimension = 0) short[] scores /* was a */;
    @Slots(RecordSlot.class) byte[] record /* was b */;
    @Slots(value = RecordSlot.class, dimension = 1) byte[][] records /* was c */;
    @Domain(ItemKind.class) byte[] itemKinds /* was d */;
    @Flags(InputMask.class) int[][] inputHistory /* was e */;
}
```

Dimensions are zero-based, starting at the outermost array. `@IndexDomain`
requires a dimension and a value domain. `@Slots`, `@Records` and `@Planes`
require a slot domain; omit `dimension` only on 1D arrays. Repeat annotations
for different dimensions. A dimension cannot combine an index domain with
slots/records/planes, or records with planes.

`@Domain` and `@Flags` describe leaf values. `@SlotValue` overrides the meaning
at one position or throughout a selected row; deeper slot bindings can refine it.

Aliases, extracted rows and inline initializers retain established shapes.
Return and parameter contracts reach local allocations and stores through aliases.
A mapped table supplies its contract to stored rows, even through table aliases.
Compatible row stores can establish a local multidimensional array's shape
without an outer index domain. One typed row does not type its siblings, nor
does one typed column type an entire record.

Conflicting layouts and escapes through unannotated calls suppress inferred
shapes. Tracking follows `Object` aliases and nested local array holders, not
arbitrary heap mutation. A primitive array used only as the source of
`System.arraycopy` retains its shape; arbitrary copies do not establish the
destination's layout. An explicit field, return or parameter contract can describe
a local destination and its stores after the copy, without typing the source.
A dynamic index selecting incompatible slots stays ambiguous.

For flat repeated records, declare offsets within one record:

```java
@SlotDomain interface Entry {
    @SlotValue(ItemKind.class) int KIND = 0;
    @SlotValue(InputMask.class) int INPUT = 1;
}
class Table /* was u */ {
    @Records(value = Entry.class, stride = 2) short[] entries /* was a */;
}
```

`entries[i * 2]` carries `ItemKind`; `entries[i * 2 + 1]` carries `InputMask`,
and the `1` can become `Entry.INPUT`. Use positive `stride`, relative slots in
`[0, stride)`, and `offset = h` for a header of `h` elements. Optional `@Slots`
on that dimension must describe positions below `h`.

For non-power-of-two strides, computed indexes also need bounds that prevent
overflow, e.g. `entries[(i & 255) * 3 + 1]`. Indexes that might reach the header
stay ambiguous. Simple constant-step `for` loops can also establish alignment;
variable starts, modified counters and unsafe overflow paths remain numeric.
A mandatory entry access such as `entries[i]` can prove that a wrapped negative
counter cannot complete later accesses. Optional accesses and caught exceptions
do not supply that proof. Array lengths also supply nonnegative range bounds.
Derived local indexes retain proven alignment through copies and agreeing
assignments; unknown or conflicting definitions suppress it.
Combined header/field literals need not acquire slot names.
Do not model an array with a trailing count as uniform records including that count.

For parallel field planes, `stride` is the capacity of each plane, and slot
values are plane numbers:

```java
@SlotDomain interface EntityPlane {
    int X = 0;
    @SlotValue(ItemKind.class) int KIND = 1;
}
class Entities /* was en */ {
    @Planes(value = EntityPlane.class, stride = 100) int[] data /* was a */;
}
```

A base offset can become `EntityPlane.KIND * 100`. The element's domain requires
the full index to stay in that plane; unbounded `100 + i` may cross into another.
Headers work as with `@Records`.

## Conditional meanings

```java
class Decoder /* was dc */ {
    void consume(int kind,
        @DomainWhen(value = ItemKind.class, parameter = 0, equals = 1)
        int payload) /* was c */;
    @DomainWhen(value = ItemKind.class, parameter = 0, equals = 1)
    int read(int kind) /* was r */;
}
```

Repeat `@DomainWhen` for distinct cases of one integral selector; case literals
must fit its type. `notEquals = k` can stand alone or pair with `equals = k`.
Use `otherwise = true` for values outside explicit equality cases; it requires
at least one such case and cannot accompany `notEquals`. Cases must not overlap.
Literal arguments, equality guards, short-circuit/ternary expressions and switch
cases without fallthrough can establish a case. Unknown selectors stay ambiguous.
Mutable receiver state is not tracked: a Gauge constructor's maximum can select
its initial-value meaning, but later `setValue` calls do not inherit that fact.

## Packed fields

```java
@ValueDomain interface Family { int ITEM = 1; int OTHER = 2; }
@PackedDomain
@BitField(name = "kind", value = Family.class, bits = 3)
@BitField(name = "subtype", value = ItemKind.class, shift = 3, bits = 5,
          selectorMask = 7, selectorValue = 1)
@BitField(name = "progress", shift = 8, bits = 7)
interface EncodedType {}
class Reader /* was rd */ {
    @Domain(EncodedType.class) int encoded /* was a */;
}
```

`value` gives the extracted value a domain; `name` identifies the physical field.
Supply either or both: unnamed fields infer value domains, while name-only fields
such as `progress` remain ordinary numbers. `@BitField` also works on value/flag
domains, retaining whole-value names.

`encoded & 7` has the family domain. `(encoded >>> 3) & 31` has the item domain
where `(encoded & 7) == 1` is known, including under a later guard on the same
packed value. Intervening writes do not establish that relationship. The physical
field name can be known before the guard if candidate fields agree on it.
An established producer layout survives other consumer interpretations of the
same word, such as using it directly as an ID in another branch. Conflicting
producer layouts remain ambiguous.

Names become uppercase underscore prefixes. For `subtype`, the generated
`EncodedType` constants are `SUBTYPE_MASK = 248` (stored bits),
`SUBTYPE_VALUE_MASK = 31` (decoded bits) and `SUBTYPE_SHIFT = 3`.
Matching reads, packing into a `@PackedDomain`, and low-field `&=` updates can
use layout constants while preserving operators and integer widths. Masks may
appear on either side of `&`. Unrelated arithmetic and nonmatching masks stay
numeric. Packing alone infers value-domain names only for fields without
selectors; selector-dependent values need other evidence. Complete words are
not automatically reconstructed from fields.

Slices must match in shift, width and signedness. Defaults are `shift = 0` and
`signed = false`; signed slices require sign-extending shifts or byte/short casts.
Use `shift` in 0..63, `bits` in 1..64 and `shift + bits <= 64`; primitive storage
must fit the slice. `selectorMask = 0` means unconditional; `selectorValue` must
not set bits outside the mask. Overlaps require mutually exclusive selectors.
Cyclic definitions and domain constants outside a slice's range are invalid.

One field may repeat its name under different selectors with the same shift,
width and signedness. Names must be Java identifiers whose generated constants
do not collide with other fields or constants.

Extracted locals and agreeing copies can use the field name; word locals use
`packed` followed by the domain's simple name. Different field identities and
unrelated slot reuse remain separate; debug and authored parameter names take
precedence. Signed extractions can name locals
even when their sign-extension shifts cannot use the field's `SHIFT` constant.

## Numeric formats and strings

```java
@NumericDomain(format = "rgb") interface Color {}
@NumericDomain(format = "fixed", fractionBits = 8) interface Fixed8 {}
@StringDomain interface RequestMethod { String GET = "GET"; String POST = "POST"; }
class Values /* was vv */ {
    @Domain(Color.class) int color /* was a */;
    @Domain(Fixed8.class) int position /* was b */;
    @Domain(RequestMethod.class) String method /* was c */;
}
```

Formats `rgb` and `argb` show hexadecimal integers with at least six/eight digits.
Compatible numeric domains can share a format; if both RGB and ARGB apply, the
wider padding wins. Conflicting symbolic domains still suppress constant names.

`fixed` accepts `fractionBits` from 0 to 62: `384` becomes `0x180 /* Q8: 1.5 */`.
`Qn` means `n` fractional bits: divide the stored integer by 2ⁿ (256 for Q8).

For decimal or other scales, use
`@NumericDomain(format = "scaled", divisor = 1000, unit = "px")`.
It renders `1500` as `1500 /* /1000: 1.5 px */`. The divisor must be a positive
integer; repeating decimals use exact fractions. `unit` is an optional display
label on `fixed` or `scaled`, not a unit-conversion rule.
Both formats preserve the stored integer; zero and standard integer extrema keep
their ordinary form. Arbitrary arithmetic does not infer scaled units.

String domains name exact tokens in assignments, arguments, returns and String
comparisons using literals or real `@DomainValue` fields. Do not assign token
domains to unrelated display text.

Class-name strings need a relocation contract, separate from token domains:

```java
class Registry /* was rg */ {
    @ClassName String[] types /* was t */;
    Class lookup(@ClassName String name) /* was a */;
}
```

`@ClassName` accepts String or String[] fields, parameters and returns, without
another value domain on that target. Use it for helper boundaries and tables
of binary class names. Known literals follow renames in the JAR and Java;
direct `Class.forName` calls are recognized automatically. Array descriptors
such as `"[Lold.Type;"` work; computed names and runtime input need separate
handling. Literals shared with ordinary text stay unchanged, with a diagnostic
in `out/semantic-summary.md`.

## Boxed values and containers

```java
class Paths /* was pa */ {
    @Elements(ItemKind.class) java.util.Vector kinds /* was a */;
    @Keys(RequestMethod.class) @Values(ItemKind.class)
    java.util.Hashtable byMethod /* was b */;
}
```

Use `@Elements` on Vector or Enumeration and `@Keys`/`@Values` on Hashtable.
Reads, writes, searches and enumerations carry content meanings; indexes and
sizes do not. Scalar domains also work on Byte, Short, Character, Integer and
Long wrappers through value-preserving boxing/unboxing, and on `Object` values
known to carry compatible boxed numbers or strings.
Returned local Vector/Hashtable allocations propagate content contracts back to
inserted values, including boxed branch-built locals. This does not cover custom
containers or mutation through unknown aliases.

## API bindings

Built-in packs cover CLDC/MIDP and selected optional APIs, activating when the
API classes are available. Check
[builtin-mappings](https://github.com/hourianto/sporeflower/tree/master/toolkit/src/main/resources/j2me/builtin-mappings) before
creating domains; import existing ones:

```java
import javax.microedition.lcdui.GraphicsAnchor;
class TextRenderer /* was t */ {
    void draw(int x, int y, @Flags(GraphicsAnchor.class) int anchor) /* was a */;
}
```

Domain markers need not exist in API JARs, but each must be declared only once,
including empty markers. Built-in bindings take precedence over project maps.
Use `@External` for uncovered API members:

```java
package sample.api;
@FlagDomain interface DeviceButtons {}
@External class Device {
    @DomainValue(DeviceButtons.class) static final int UP;
    void setButtons(@Flags(DeviceButtons.class) int buttons);
}
```

External declarations use bytecode names and do not rename APIs or contribute
to project coverage. Useful built-in domains include graphics anchors/colors,
key actions, text constraints, layouts, HTTP methods/statuses and M3G modes.

## Comments

Record sentinel values, units, bit layouts, array shapes and relationships a
name cannot express. Avoid progress diaries, guessed identities and comments
that merely repeat the names below them.
