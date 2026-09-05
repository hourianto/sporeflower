# Mapping reference

Write Java-like declarations in a project's `mappings/*.map` files, normally
one class per file. They name bytecode entities and attach semantic meanings;
they are not Java source to compile. See [INSTALL.md](INSTALL.md) for setup.
`j2me init` installs this reference as project guidance.

## Workflow

1. Use `out/coverage.md` to find unnamed classes and map them first.
2. Use `out/usage-priority.md` to choose members; its TSV has full JVM identities.
3. Edit the maps, run `j2me remap`, and resolve reported errors.
4. Inspect generated Java to verify names and semantic meanings.

The generated `out/semantic-summary.md` lists declared contracts and domain
references; it does not measure successful substitutions.

Routine mapping work needs only `remap`. Do not repair authored maps by editing
generated Tiny mappings, Java output, or reports. If a diagnostic contradicts
the input, investigate the tool rather than inventing a rename.

## Names and types

```java
package sample.game;
import java.util.Hashtable;
import javax.microedition.lcdui.Graphics;

class GameEngine /* was af */ {
    int frameCounter /* was a */;
    Hashtable cache /* was b */;
    void draw(Graphics graphics, int x, int y) /* was a */;
    int tick(int frame, long delta) /* was c */;
}
```

The declaration gives the readable name and type. `/* was ... */` gives the
original class owner or simple member name. Owners may be simple,
slash-separated, or fully qualified; package-relative owners resolve against
the input classes. Original names may be Java keywords: `class State /* was do */ {}`.

Name every parameter. Constructors follow their class rename. Use readable
project class names in types, mapping those classes first. Primitives, arrays,
qualified names and Java imports work; common `java.lang` types need no import.
Qualify ambiguous names. External types must exist in the available APIs.

To exclude a class whose bytecode names are already final from renaming coverage:

```java
package sample.game;
@AlreadyMapped class SettingsScreen {}
```

The package and name identify its bytecode owner; the body must be empty with
no `/* was ... */` marker.

## Choosing semantic annotations

Bind a meaning only where it holds for every use of the annotated value.
Do not guess from a matching number or a readable variable name alone.

| Annotation | Use |
| --- | --- |
| `@ValueDomain` | Named integral values |
| `@FlagDomain` | Bit masks |
| `@SlotDomain` | Array positions or record fields |
| `@PackedDomain`, `@BitField(...)` | Fields within an encoded integer |
| `@NumericDomain(...)` | RGB, ARGB or fixed-point formatting |
| `@StringDomain` | String tokens |
| `@DomainValue(D.class)` | A real constant belonging to `D` |
| `@Domain(D.class)`, `@Flags(D.class)` | Scalar values or array leaf values |
| `@DomainFromParameter(n)` | A helper return that preserves an argument's meaning |
| `@DomainFromSlot(parameter = n, slot = k)` | A parameter/return using a supplied table column's meaning |
| `@CallDomain(...)` | One specific call result |
| `@DomainWhen(...)` | A parameter/return whose meaning depends on another parameter |
| `@IndexDomain(...)` | Array index values |
| `@Slots(...)`, `@SlotValue(D.class)` | Fixed positions and their stored values |
| `@Records(...)`, `@Planes(...)` | Repeated records or parallel field planes |
| `@Elements(D.class)`, `@Keys(D.class)`, `@Values(D.class)` | J2ME container contents |

Value, array and container bindings apply to fields, parameters and returns,
including constructor parameters. Parameter indexes are zero-based and count
declared parameters, excluding `this`.

Constructors use the mapped class name and an empty body, e.g.
`World(@Domain(ItemKind.class) int kind) {}`. Omit the member `was` comment.
Their parameter names and bindings apply to `new`, `this` and `super` calls.

Meanings propagate through copies, compatible casts, unambiguous branches and
supported consumers. Conflicting meanings or unknown dynamic values suppress
names; literal defaults can accompany a known domain. Arithmetic deltas, shift
distances, sign checks and wrap bounds may remain numeric: `direction -= 2`
does not make `2` a direction. Bit-mask updates can name their operands.
Always inspect the output; a missing name is preferable to a false meaning.

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

`@DomainValue` reads the bytecode value; the field must be `static final` with
a `ConstantValue` attribute. Use integral fields for value/flag/slot domains and
String fields for string domains. Otherwise, declare source-only constants:

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

These generate constant interfaces beside the decompiled source. Integral
constants support `byte`, `short`, `char`, `int` and `long`; slot constants cannot
be `long`. Each domain must have unique constant names and values.

Flags combine known bits with `|`, retaining unknown bits numerically. For a mask
containing mutually exclusive choices, mark their bit ranges:

```java
@FlagDomain(exclusiveMasks = {0x0f}) interface Options {
    int TEXT = 1;
    int NUMBER = 2;
    int PASSWORD = 0x100;
}
```

`0x102` can become `Options.NUMBER | Options.PASSWORD`; the unknown choice in
`0x103` stays numeric instead of becoming `TEXT | NUMBER`. Exclusive masks must
be nonzero and disjoint. To give extracted bits a separate meaning, use `@BitField`.
Signed masks may use byte/short casts to preserve sign extension compactly.

## Helpers and individual reads

Use `@DomainFromParameter` when a generic integral helper returns the same kind
of quantity as one argument:

```java
class NumericHelper /* was e */ {
    @DomainFromParameter(0) static int absolute(int value) /* was b */;
}
```

Both parameter and return must be integral scalars. Do not combine this with a
fixed return binding or use it for results with different meanings, such as
lengths, hashes, signs or comparison results.

For a generic table helper, derive each call's meanings from the supplied array:

```java
class Lookup /* was lu */ {
    @DomainFromSlot(parameter = 0, slot = 1)
    int find(byte[] pairs, @DomainFromSlot(parameter = 0, slot = 0) int key) /* was a */;
}
```

The source must be an integral array; the target must be an integral parameter
or return. For multidimensional tables, specify the innermost `dimension`, e.g.
`@DomainFromSlot(parameter = 0, dimension = 1, slot = 1)` for `int[][]` rows.
Omit `dimension` only on 1D arrays. `slot` selects a `@SlotValue` column from its `@Records`, `@Planes`
or `@Slots` contract, falling back to its leaf domain. Record columns are relative
to a record, excluding any header. Each call uses its own table's meanings;
unknown/conflicting tables stay ambiguous and the helper body stays generic.
Do not combine this with another binding on the same target.

If only one call to a generic reader has a known meaning, annotate its containing
method instead of assigning that meaning to every reader result:

```java
class Decoder /* was d */ {
    @CallDomain(value = ItemKind.class, offset = 17)
    void decode() /* was a */;
}
```

Find the original invoke instruction's byte offset with
`javap -c -p -classpath original.jar OriginalClass`. It is not a source line or
instruction ordinal. The result must be compatible with the domain: integral,
boxed integral or String. Repeat `@CallDomain` for different calls. It overrides
a general return binding for that call only; overriding methods do not inherit
it. Recheck offsets if the input JAR changes.

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

`@IndexDomain` requires a dimension and a value domain. `@Slots`, `@Records` and
`@Planes` require a slot domain; they may omit `dimension` only on a 1D array.
Repeat annotations for different dimensions. `@Domain` and `@Flags` describe
scalar leaf values; `@SlotValue` overrides the value meaning at one position,
or throughout a selected row. Deeper slot bindings can refine that row.

Aliases, extracted rows and inline initializers retain established array
meanings. A dynamic index that can select incompatible slots stays ambiguous.
Do not annotate an entire array with a meaning that holds for only some entries.

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
on that dimension must describe positions below `h`. A dimension cannot combine
an index domain with slots/records/planes, or combine records with planes.

For non-power-of-two strides, computed indexes also need bounds that prevent
overflow, e.g. `entries[(i & 255) * 3 + 1]`. Indexes that might reach the header
stay ambiguous. Simple constant-step `for` loops can also establish alignment;
variable starts, modified counters and unsafe overflow paths remain numeric.
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

A base offset can become `EntityPlane.KIND * 100`. Assigning a domain to the
selected element additionally requires the full index to stay in that plane;
unbounded `100 + i` may cross into another plane. Headers work as with `@Records`.

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

Repeat `@DomainWhen` for distinct cases of the same integral selector. Use
`notEquals = k` for the complementary case, or `otherwise = true` for values
outside all explicit `equals` cases. Cases must not overlap: a negative case
may only accompany its matching equality case; a default requires equality cases.
These cannot combine with another binding on the same target.
Literal arguments, equality guards, short-circuit/ternary expressions and switch
cases without fallthrough can establish a case. Unknown selectors stay ambiguous.
Mutable receiver state is not tracked: a Gauge constructor's maximum can select
its initial-value meaning, but later `setValue` calls do not inherit that fact.

## Packed fields

```java
@ValueDomain interface Family { int ITEM = 1; int OTHER = 2; }
@PackedDomain
@BitField(value = Family.class, shift = 0, bits = 3)
@BitField(value = ItemKind.class, shift = 3, bits = 5,
          selectorMask = 7, selectorValue = 1)
interface EncodedType {}
class Reader /* was rd */ {
    @Domain(EncodedType.class) int encoded /* was a */;
}
```

`encoded & 7` has the family domain. `(encoded >>> 3) & 31` has the item domain
only where `(encoded & 7) == 1` is known. Use `signed = true` for signed slices;
shift pairs and byte/short casts can establish sign extension. Masks, widths,
shifts and signedness must match the declaration.
An extracted local can acquire its meaning under a later guard on the same
packed value. Unknown selectors and intervening writes do not establish a case.

`@BitField` also works on value/flag domains, retaining whole-value names.
Overlapping slices require mutually exclusive selectors; cyclic definitions and
constants outside a slice's range are invalid. Matching packing expressions can
name field operands; complete words are not automatically reconstructed from fields.

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
`fixed` accepts `fractionBits` from 0 to 62: `384` becomes `0x180 /* Q8: 1.5 */`.
Here, `Qn` means `n` fractional bits: divide the stored integer by 2ⁿ to decode
the value. For Q8, the divisor is 256.
It preserves the integer; zero and standard integer extrema keep their ordinary
form. Arbitrary arithmetic does not infer fixed-point units.

String domains name exact tokens in assignments, arguments, returns and String
comparisons. Use source-only String literals or real `@DomainValue` fields.
Unrelated display text is not a reason to assign a token domain.

## Boxed values and containers

```java
class Paths /* was pa */ {
    @Elements(ItemKind.class) java.util.Vector kinds /* was a */;
    @Keys(RequestMethod.class) @Values(ItemKind.class)
    java.util.Hashtable byMethod /* was b */;
}
```

Use `@Elements` on Vector or Enumeration and `@Keys`/`@Values` on Hashtable.
Reads, writes, searches and enumerations carry the corresponding content meaning;
indexes and sizes do not. Byte, Short, Character, Integer and Long wrappers can
also carry scalar domains through boxing/unboxing that preserves values.
These contracts do not describe arbitrary custom containers.

## API bindings

Built-in packs cover CLDC/MIDP and selected optional APIs, activating when the
API classes are available. Look up existing domains in
[builtin-mappings](../toolkit/src/main/resources/j2me/builtin-mappings) before
creating one; import their names in project maps:

```java
import javax.microedition.lcdui.GraphicsAnchor;
class TextRenderer /* was t */ {
    void draw(int x, int y, @Flags(GraphicsAnchor.class) int anchor) /* was a */;
}
```

Domain markers need not exist in API JARs. Declarations must be unique, including
empty markers: reference built-in domains directly. Built-in bindings are
authoritative; project maps can describe uncovered API members with `@External`:

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
