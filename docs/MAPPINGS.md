# Mapping reference

Write Java-like declarations in a project's `mappings/*.map` files, normally
one class per file. These files name bytecode entities and attach semantic
meaning to constants; they are not Java source to compile.

For installation and getting started, see [INSTALL.md](../docs/INSTALL.md).
`j2me init` also installs this reference as project guidance.

## Authoring workflow

1. Use `out/coverage.md` to find unnamed classes and map them first.
2. Use `out/usage-priority.md` to choose fields and methods. The TSV version
   contains complete JVM identities and usage details.
3. Edit the authored maps, run `j2me remap`, and resolve reported errors.
4. Inspect the generated Java and refine names or semantic annotations.

Routine mapping work needs only `remap`. Do not edit generated Tiny mappings,
Java output, or reports to repair the authored maps. If a diagnostic contradicts
the input or generated source, investigate the tool rather than inventing a rename.

## Classes, members, and types

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

The declaration supplies the readable name and member type. `/* was ... */`
supplies the original name: a class owner on class declarations, or a simple
field/method name on members. Owners may be simple, slash-separated, or fully
qualified. A package-relative owner is resolved against the input classes.
Original names may be Java keywords, such as `class State /* was do */ {}`.

Name each method parameter. Constructors follow their class rename. Use readable
project class names in types, mapping those classes before members that refer
to them. Primitives, arrays, qualified names, and Java imports are supported.
Common `java.lang` types can be unqualified. Use an import or qualified name
when a short name is ambiguous. External types must exist in the available APIs.

A class whose bytecode names are already final can be excluded from renaming
coverage with an empty declaration:

```java
package sample.game;

@AlreadyMapped
class SettingsScreen {}
```

Its declared package and class name identify the bytecode owner. It must have
an empty body and no `/* was ... */` marker.

## Domains and constants

Domains give a number meaning within an annotated context. Unannotated or
ambiguous values remain numeric. Use them only when the meaning holds for every
use of that field, parameter, return value, or array position.

| Annotation | Purpose |
| --- | --- |
| `@ValueDomain` | Declares a set of named integral values |
| `@FlagDomain` | Declares named bit-mask values |
| `@SlotDomain` | Declares named array record positions |
| `@DomainValue(D.class)` | Uses a real constant field as a value in domain `D` |
| `@Domain(D.class)` | Binds a scalar or array element value to `D` |
| `@Flags(D.class)` | Binds a scalar or array element bit mask to `D` |
| `@DomainFromParameter(index)` | Makes a helper's return inherit one argument's domain |
| `@IndexDomain(value = D.class, dimension = n)` | Gives array indexes a domain |
| `@Slots(value = D.class, dimension = n)` | Gives fixed array positions names and optional value domains |
| `@SlotValue(D.class)` | Gives values stored at one declared slot a domain |

Prefer real fields when the input already contains suitable constants:

```java
package sample.game;

@ValueDomain interface Direction {}

class Entity /* was j */ {
    @DomainValue(Direction.class) static final int RIGHT /* was k */;
    @DomainValue(Direction.class) static final int LEFT /* was o */;

    @Domain(Direction.class) int direction /* was a */;
    void setDirection(@Domain(Direction.class) int value) /* was j */;
    @Domain(Direction.class) int getDirection() /* was q */;
}
```

`@DomainValue` uses the actual bytecode value. The field must be integral,
`static final`, and have a `ConstantValue` attribute.

When suitable fields do not exist, declare source-only constants:

```java
package sample.game;

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

These declarations produce constant interfaces alongside the decompiled source.
Supported constant types are `byte`, `short`, `char`, `int`, and `long`. Flag
rendering combines known bits with OR expressions, retains unknown residual bits
as numbers, and can use complements when the mask is fully known.

## Helper methods

A generic numeric helper may preserve the meaning of its argument:

```java
package sample.game;

class NumericHelper /* was e */ {
    @DomainFromParameter(0)
    static int absolute(int value) /* was b */;
}
```

The index is zero-based and counts declared parameters. The selected parameter
and return must be integral scalars. The annotation cannot be combined with a
fixed `@Domain` or `@Flags` return binding.

At each call site, an unambiguous argument domain can flow to the result.
Do not use this annotation when the result represents a different quantity,
such as a sign, price, length, hash, or comparison result.

## Arrays and record slots

Array indexes, fixed record positions, and stored values are separate meanings:

```java
package sample.game;

@ValueDomain interface PlayerIndex {
    int FIRST = 0;
    int SECOND = 1;
}

@SlotDomain interface RecordSlot {
    int OWNER = 0;
    @SlotValue(ItemKind.class) int KIND = 1;
}

class World /* was f */ {
    @IndexDomain(value = PlayerIndex.class, dimension = 0)
    short[] scores /* was a */;

    @Slots(RecordSlot.class) byte[] record /* was b */;
    @Slots(value = RecordSlot.class, dimension = 1) byte[][] records /* was c */;
    @Domain(ItemKind.class) byte[] itemKinds /* was d */;
    @Flags(InputMask.class) int[][] inputHistory /* was e */;
}
```

`@IndexDomain` always requires a dimension. `@Slots` may omit it only for a
one-dimensional array. `@Domain` and `@Flags` on arrays describe scalar leaf
values; `@SlotValue` applies to the value stored at a particular slot. These
array bindings can also be used on parameters and method returns.

Unambiguous aliases and extracted rows retain array semantics. Computed indexes
are not replaced with fixed slot names.

## API bindings

Built-in semantic packs cover common CLDC/MIDP constants and selected optional
APIs. They activate when the corresponding API classes are available. Import
their domains for application fields or wrappers instead of copying the packs:

```java
package sample.game;
import javax.microedition.lcdui.GraphicsAnchor;

class TextRenderer /* was t */ {
    void draw(int x, int y, @Flags(GraphicsAnchor.class) int anchor) /* was a */;
}
```

Domain marker interfaces belong to the mapping language and need not exist in
API JARs. Built-in declarations are authoritative; project maps can add bindings
for uncovered API members.

Use `@External` to describe other library constants and call sites:

```java
package sample.api;

@FlagDomain interface DeviceButtons {}

@External
class Device {
    @DomainValue(DeviceButtons.class) static final int UP;
    void setButtons(@Flags(DeviceButtons.class) int buttons);
}
```

External declarations use the API's bytecode names and do not rename it or
contribute to project coverage. Semantic annotations also apply to constructor
parameters.

## Comments

Record information a name cannot express: sentinel values, units, bit layouts,
array shapes, or non-obvious relationships. Avoid progress diaries, guessed
identities, and section comments that merely repeat the names below them.
