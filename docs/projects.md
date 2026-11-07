# Project configuration and compilation

After creating a project with the [README workflow](../README.md), keep its
library and compiler settings in `j2me.toml`. The commands below run from that
project directory. From elsewhere, add `--project /path/to/game`.

`init` copies the full mapping guide to `mappings/MAPPINGS.md`.
`init --force` refreshes that copy.

## Add libraries

Bundled API declarations cover common J2ME libraries. If the application uses
other APIs, supply their JARs or directories of class files through `classpath`:

```toml
jar = "game.jar"
classpath = ["lib/vendor-api.jar", "lib/classes"]
```

These declarations help resolve types, inheritance and calls. Later classpath
entries override earlier entries and bundled declarations. For a one-time
addition, use `sporeflower remap --classpath extra-api.jar`; command-line
entries take precedence over the configured list.

Paths in `j2me.toml` are relative to the project directory; absolute paths also
work. Paths passed on the command line are relative to the current directory.
Configured `java` and `javac` commands are the exception: a bare executable name
is looked up on `PATH`, while a path such as `tools/jdk/bin/java` is relative to
the project.

Run `sporeflower remap` after changing the classpath. It regenerates sources and
`out/api-stubs.jar`, and records the library inputs used for the next compilation
check. See [API declarations](../api/README.md) for coverage and missing classes.
The exported API JAR contains compile-only stubs, not a J2ME runtime.

## Check generated Java

Compilation is optional; decompilation and remapping need no Java installation.
To check the current generated sources, install a JDK and run:

```sh
sporeflower compile-stubs
```

The command uses the API stubs and libraries selected during source generation.
It writes compiler logs and results under `out/compile-stubs/`, replacing that
directory's `classes/` on each run. Run `remap` first if you have changed mappings
or libraries. A successful compilation checks Java source validity, not
behavioral equivalence to the original program.

By default, compilation uses `javac` from `PATH` with a Java 8 target. To save a
different executable or release, add a compiler section:

```toml
[compiler]
backend = "javac"
javac = "tools/jdk/bin/javac"
java_release = 8
```

The selected `javac` must be Java 8 or newer and support the requested release.
For a one-time override, use `--javac-bin /path/to/javac` or `--java-release 8`.

## Compile for older Java targets

The `legacy` backend uses **JDK 1.4.2_30 javac**, with its package renamed to
avoid conflicts with the host JDK's compiler. It runs on a modern JVM and
supports Java 1.3 source and Java 1.1 bytecode targets. Supply the compiler JAR
separately.

To prepare `legacy-javac.jar`:

1. Download `j2sdk-1_4_2_30-solaris-i586.tar.Z` from
   [Oracle's Java 1.4 archive](https://www.oracle.com/java/technologies/java-archive-javase-v14-downloads.html).
   This direct download currently works without an Oracle sign-in, using the
   license-acceptance cookie; the URL or cookie requirement may change:

   ```sh
   curl --fail --location --cookie 'oraclelicense=accept-securebackup-cookie' --output j2sdk-1_4_2_30-solaris-i586.tar.Z 'https://download.oracle.com/otn-pub/java/j2sdk/1.4.2_30-b04/j2sdk-1_4_2_30-solaris-i586.tar.Z'
   ```

   Unpack it and take `SUNWj3dev/reloc/j2se/lib/tools.jar`. The compiler is Java
   bytecode; no Solaris installation is needed.
2. Keep only `com/sun/tools/javac/**`, including its resource classes. Relocate
   `com.sun.tools.javac` to `j2me.thirdparty.legacyjavac`: update JAR entry paths
   and class-file UTF-8 constant-pool entries, in both slash-separated and dotted
   forms. This includes descriptors and strings used for reflection and resource
   lookups. No compiler logic changes are needed.
3. Pack those entries as `legacy-javac.jar`. Its entry point is
   `j2me.thirdparty.legacyjavac.Main`.

For example, keep the compiler in the project's `tools/` directory and add:

```toml
[compiler]
backend = "legacy"
jar = "tools/legacy-javac.jar"
java = "java"
```

Then run `sporeflower compile-stubs`. Alternatively, use `backend = "ecj"` with
an Eclipse compiler JAR supporting the required targets; its entry point is
`org.eclipse.jdt.internal.compiler.batch.Main`. Both backends use the exported
J2ME API declarations as their boot classpath.

`init` records the original class-file version in `[bytecode].major` and sets
`javac_source` and `javac_target`. Legacy and ECJ compilation use those levels
by default. To override them, add these keys to the same `[compiler]` section:

```toml
source_level = "1.3"
target_level = "1.1"
```

Choose levels supported by your compiler. Command-line compiler options override
the corresponding project settings; `sporeflower compile-stubs --help` lists
them, along with options for custom source and output directories.

## Generate sources without remapping a JAR

To use a project's JAR, libraries and mappings for source generation only:

```sh
sporeflower --project . -o source --mapping-report reports
```

Or supply mappings directly, without a project:

```sh
sporeflower game.jar --mappings mappings -o source --mapping-report reports
```

These commands generate Java and a Tiny name map without writing a remapped
JAR. The source directory is cleared before generation. Add `--stubs api.jar`
to export the selected API declarations for compilation. See the
[mapping reference](mappings.md) for authoring and interpreting reports.

## Refresh extracted resources

`init` extracts the original JAR's resources for inspection. To extract them
again, run `sporeflower extract-resources`; this replaces `resources/` with a
fresh copy. Remapping retains the original JAR's resources, so edits to the
extracted files are not packaged into the mapped JAR.
