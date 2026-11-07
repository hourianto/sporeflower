# Mapping reference

Write Java-like declarations in `mappings/*.map` to rename classes and members
and describe their values, then run `sporeflower remap`. See the
[project workflow](https://github.com/hourianto/sporeflower#work-with-mappings) for setup.

Maps are declarations, not compilable Java. Each file may contain several classes
and domains; filenames do not matter. Only `.map` files directly inside
`mappings/` are loaded. Both direct decompilation and `remap` accept repeated
`--mappings` file or directory arguments. Edit the maps, not generated Java or reports.

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

`package` sets the destination; omitting it uses `defpackage`, including for
generated domains. Package moves must preserve access and overrides. Unmapped
classes follow their original package's mapped classes when those share one
destination. Relative resource lookups retain the original package directory.

Name every parameter. Use mapped project class names in types. Primitives,
arrays, qualified names and imports work; common `java.lang` types need no
import. Qualify ambiguous names. External types must exist in the available APIs.
Match the original member's full parameter and return types. Access flags and
hierarchy come from the bytecode; modifiers and class headers do not change them.
Unlisted members are still decompiled. Each listed member needs a `was` marker,
even if unchanged, except constructors and [external API bindings](#api-bindings).

Constructors use the mapped class name, an empty body and no member `was`
comment. Their parameter names and semantic bindings apply to `new`, `this`
and `super` calls.

Exclude classes whose bytecode names are already final from renaming coverage:

```java
package sample.game;
@AlreadyMapped class SettingsScreen {}
```

Use the bytecode package and class name, an empty body and no `was` marker.

Authored names override automatic and debug names. Method overrides share a name;
external API methods keep theirs. Conflicting explicit names are errors, while
collisions with automatic names may be repaired with suffixes.

## Choosing semantic annotations

A *domain* groups values with one meaning, such as directions or item kinds.
A *contract* attaches it to a field, parameter, return or array position.
For example, `@Domain(Direction.class) int direction` allows matching literals
to become `Direction.LEFT`. Bind meanings supported by the program, not merely
numbers that happen to match.

Use `@Domain` for value, packed, numeric or string domains; `@Flags` requires a
flag domain. Scalar, array and container bindings apply to fields, parameters
and returns. Parameter indexes count declarations from zero, excluding `this`;
`long` and `double` each count once, unlike JVM slots.
Place a return annotation before the method declaration, and a parameter
annotation immediately before that parameter's type.

`@DomainFromParameter`, `@DomainFromSlot` and `@DomainWhen` cannot share a
target with each other or a fixed contract.

Unannotated overrides inherit method contracts. An explicit binding replaces
the whole parameter or return contract, so repeat inherited annotations you still
need. Conflicting inherited contracts stay ambiguous. A field hiding an ancestor's
field does not inherit its contract. `@CallDomain` bindings are not inherited.

Semantic annotations change source presentation, not stored values.
`@ClassName` is the exception: it relocates strings in both Java and the JAR.

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
Decimal, hexadecimal, binary and octal forms are accepted, as are underscores,
character literals and the `L` suffix for long values.

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

A mask declared as `byte HIGH = -128` remains negative when promoted to `int`.
Use `int HIGH = 128` for a mask containing only bit 7.

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
the helper body stays generic.

For a meaning specific to one call, annotate its containing method:

```java
class Decoder /* was d */ {
    @CallDomain(value = ItemKind.class, offset = 17)
    void decode() /* was a */;
}
```

Find the original invoke instruction's byte offset with
`javap -c -p -classpath original.jar OriginalClass`. `@CallDomain` overrides
the return binding at that call; add `parameter = n` to bind an argument instead,
including constructor arguments. Repeat it for other calls or arguments.
`out/semantic-summary.md` lists offsets for methods with scoped contracts.
Recheck offsets if the input JAR changes.

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

For example, a count followed by pairs uses separate header and record domains:

```java
@SlotDomain interface Header { int COUNT = 0; }
class CountedTable /* was ct */ {
    @Slots(Header.class)
    @Records(value = Entry.class, stride = 2, offset = 1)
    short[] entries /* was a */;
}
```

`entries[0]` is the count. Records start at `entries[1]`, and their `INPUT`
column is at `1 + i * 2 + 1`. `Entry.INPUT` remains a column offset of 1;
it does not include the header.

Column names require an unambiguous index; overflow or header accesses can prevent this.
Combined header/column offsets may stay numeric: the `2` in `entries[i * 2 + 2]`
includes both the header and `Entry.INPUT`. Keep trailing counts outside the
record layout.

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

One method can use an integer as different kinds of value depending on another
argument. Bind that relationship instead of assigning one domain to every call:

```java
@ValueDomain interface PayloadKind { int ITEM = 1; int STATUS = 2; }
@ValueDomain interface Status { int READY = 2; int FINISHED = 3; }
class Decoder /* was dc */ {
    void consume(@Domain(PayloadKind.class) int kind,
        @DomainWhen(value = ItemKind.class, parameter = 0, equals = 1)
        @DomainWhen(value = Status.class, parameter = 0, equals = 2)
        int payload) /* was c */;
    @DomainWhen(value = ItemKind.class, parameter = 0, equals = 1)
    int read(int kind) /* was r */;
}
```

Here `consume(1, 2)` can become
`consume(PayloadKind.ITEM, ItemKind.UNIT)`, while `consume(2, 2)` can become
`consume(PayloadKind.STATUS, Status.READY)`. An unknown selector leaves the
payload numeric.

A condition can supply the missing selector value, so generated Java can use
the domain constant even when the argument is a variable:

```java
if (kind == PayloadKind.ITEM) {
    decoder.consume(kind, ItemKind.UNIT);
}
```

The return annotation on `read` similarly lets `read(1) == 2` use `ItemKind.UNIT`.

Repeat `@DomainWhen` for distinct cases of one integral selector; case literals
must fit its type. `notEquals = k` can stand alone or pair with `equals = k`.
Use `otherwise = true` for values outside explicit equality cases; it requires
at least one such case and cannot accompany `notEquals`. Cases must not overlap.
The selector must be an argument; bindings cannot depend on mutable object state.

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
when `(encoded & 7) == 1` is known.

Names become uppercase underscore prefixes. For `subtype`, the generated
`EncodedType` constants are `SUBTYPE_MASK = 248` (stored bits),
`SUBTYPE_VALUE_MASK = 31` (decoded bits) and `SUBTYPE_SHIFT = 3`.
Matching reads and packing operations can use these constants. Packing alone
infers value-domain names only for fields without selectors.

Slices must match in shift, width and signedness. Defaults are `shift = 0` and
`signed = false`; signed slices require sign-extending shifts or byte/short casts.
Use `shift` in 0..63, `bits` in 1..64 and `shift + bits <= 64`; primitive storage
must fit the slice. `selectorMask = 0` means unconditional; `selectorValue` must
not set bits outside the mask. Overlaps require mutually exclusive selectors.
Cyclic definitions and domain constants outside a slice's range are invalid.

One field may repeat its name under different selectors with the same shift,
width and signedness. Names must be Java identifiers whose generated constants
do not collide with other fields or constants.

## Numeric formats and strings

```java
@NumericDomain(format = "rgb") interface Color {}
@NumericDomain(format = "fixed", fractionBits = 8) interface Fixed8 {}
@NumericDomain(format = "scaled", divisor = 1000, unit = "px") interface Millipixels {}
@StringDomain interface RequestMethod { String GET = "GET"; String POST = "POST"; }
class Values /* was vv */ {
    @Domain(Color.class) int color /* was a */;
    @Domain(Fixed8.class) int position /* was b */;
    @Domain(RequestMethod.class) String method /* was c */;
    @Domain(Millipixels.class) int distance /* was d */;
}
```

`rgb` and `argb` print hexadecimal integers with at least six/eight digits.
`fixed` accepts `fractionBits` from 0 to 62: `384` becomes `0x180 /* Q8: 1.5 */`,
where Q8 means division by 2⁸. `scaled` requires a positive integer divisor:
`1500` becomes `1500 /* /1000: 1.5 px */` in the example. `unit` is an optional
label for `fixed` and `scaled`. These formats preserve the stored integer;
zero and standard integer extrema keep their ordinary form.

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
direct `Class.forName` calls are recognized automatically. Use dotted binary
names such as `"old.Type"` or `"[Lold.Type;"`, not `"old/Type"`. Shared display
text stays unchanged. For computed names or runtime input, inspect the generated
lookup code and diagnostics.

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

Custom containers and mutation through unknown aliases are not covered.

## API bindings

Built-in packs cover CLDC/MIDP and selected optional APIs, activating when the
API classes are available. Check
[the bundled maps](https://github.com/hourianto/sporeflower/tree/master/src/mappings/builtin) before
creating domains; import existing ones:

```java
import javax.microedition.lcdui.GraphicsAnchor;
class TextRenderer /* was t */ {
    void draw(int x, int y, @Flags(GraphicsAnchor.class) int anchor) /* was a */;
}
```

Each domain must be declared only once, including empty markers. Built-in
bindings take precedence; `--no-builtin-mappings` disables them. Use `--classpath`
for additional API declarations and `@External` to annotate uncovered members:

```java
package sample.api;
@FlagDomain interface DeviceButtons {}
@External class Device {
    @DomainValue(DeviceButtons.class) static final int UP;
    void setButtons(@Flags(DeviceButtons.class) int buttons);
}
```

External declarations use bytecode names and do not rename APIs or contribute
to project coverage. Domains themselves need no class in the API JAR.

## Inference and limits

Bindings follow values through assignments, calls and compatible casts. Annotate
fields, parameters and returns; conflicting or unknown meanings can leave
literals unnamed. Array positions and their contents have separate meanings:
one typed row or column does not type the whole table.

Arithmetic offsets retain their numeric meaning: `direction -= 2` does not make
`2` a direction constant. Sign checks such as `state < 0` keep numeric zero;
equality checks and nonzero range endpoints can use domain constants.

## Reports and diagnostics

Use `out/coverage.md` to find unnamed classes and `out/usage-priority.tsv` to
choose members by their bytecode reference counts. The latter includes original
owners, names and descriptors. After remapping, inspect both named substitutions
and remaining literals in the generated Java. `out/semantic-summary.md` lists
contracts and domain references; it does not count substitutions.

`out/mapping.tiny` exports the resolved names in Tiny v2 format. Semantic
annotations remain in the `.map` files.

Invalid mappings report the file, line, column and source text with a caret.
Missing members list matching overload candidates; check the original owner,
member name and full parameter/return descriptor before changing a declaration.
Conflicting names, packages or contracts identify the relevant declarations.

Diagnostics use JVM descriptors: `(I)I` means one `int` parameter and an `int`
return. Parameters go inside parentheses; the return type follows them.
Primitive codes are `B` byte, `C` char, `D` double, `F` float, `I` int, `J` long,
`S` short and `Z` boolean; `V` means a void return. `Lpkg/Name;` is a class type,
and each `[` adds an array dimension: `[I` is `int[]`.
