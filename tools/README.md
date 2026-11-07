# Corpus tools

These tools require Python 3.11+, a separately obtained J2ME corpus, and a
[release build](../docs/building.md) at `target/release/sporeflower`. Run the
examples from the repository root; each script has `--help` for more options.
The tools do not execute application code.

## Choose a corpus

Keep third-party inputs outside this repository. The corpus contains project
directories with `j2me.toml` and an input JAR (`game.jar` by default). Select its
root with `--root` or `SPOREFLOWER_CORPUS`; the fallback is the current directory.

Name projects explicitly, or use `corpus.py --all` for direct children with
`j2me.toml`, or `--workflow --mapped` for those with authored mappings. Recompile
mode also accepts explicitly named directories containing only `game.jar`.

## Recompile generated sources

```sh
python3 tools/corpus.py --root /path/to/corpus PROJECT
```

This decompiles the original JAR and compiles the output against exported API
stubs. It does not use the project's authored mappings or configured classpath.
Use JDK 17. For Java 1.3 checks, add
`--legacy-javac /path/to/legacy-javac.jar`; see
[compiler setup](../docs/projects.md#compile-for-older-java-targets).

## Check a mapping project

```sh
python3 tools/corpus.py --root /path/to/corpus --workflow PROJECT
```

Workflow mode copies project inputs into a separate work directory, then runs
`remap` and `compile-stubs` with the authored mappings and project classpath.
It checks mapped class names, resources and manifest entry points, and reports
reconstruction and compilation separately. It requires `j2me.toml` and uses its
compiler settings. For javac, `--javac`, `JAVAC`, then `JAVA_HOME/bin/javac` take
precedence over the configured executable. `--legacy-javac` uses the supplied
compiler JAR instead.

Both modes retain sources, classes and logs under timestamped directories in
`work/corpus/`. Its `results.json` describes only the latest invocation and
records the original JAR, rebuilt classes, mapping and effective classpath for
comparison. Use a distinct `--output` directory to retain each run's manifest.
Workflow output must be separate from the corpus root.

## Compare bytecode

```sh
python3 tools/verify_corpus.py work/corpus/results.json --verifier /path/to/j2me-verifier --out work/verification
```

Supply `j2me-verifier` separately; the runner expects its mapping/classpath,
JSON-report and method-budget CLI. `J2ME_VERIFIER` or `PATH` can also select it.
The output directory must be new and receives reports, logs and `summary.json`.

Inspect the method reports for non-`MATCH` results: `REVIEW` flags a possible
difference, while `LIMIT` and `UNRESOLVED` identify incomplete checks.

## Compare speed and memory

```sh
python3 tools/benchmark.py --root /path/to/corpus --baseline-jar /path/to/decompiler.jar PROJECT
```

Supply an executable Java decompiler JAR with the Sporeflower/Vineflower CLI and
a compatible Java runtime. Projects need `j2me.toml`. Measurement uses macOS
`/usr/bin/time` or GNU `/usr/bin/time`; Linux systems may need the `time` package.

Each measurement starts a fresh process, so JVM startup is included. Results and
logs go to a timestamped directory under `work/benchmark/`. Retain the baseline
JAR version and environment alongside results when comparing runs.
