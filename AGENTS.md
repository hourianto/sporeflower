# What is Sporeflower

This project is a fork of the modern Vineflower Java decompiler.

Sporeflower's focus is explicitly on decompiling old J2ME-era .jar programs and games. They have much less advanced Java features, but are often minified and sometimes have weird bytecode shapes.

## Tooling
The code is in Java, the build system is Gradle with ./gradlew.

After a round of fixes, when you're done and have changed something in the decompiler, please run ./gradlew jar to rebuild the jar.

## Privacy
Do not directly use the original J2ME game/program or source names in tests. Do not mention the specific J2ME program/game names in commit messages or test files. It is totally OKAY to mention them in chat/local files (the ones that are ignored/aren't tracked). Only don't mention in commit msgs and in files that are tracked.

## Test suite
Please run the full test suite if you finished modifying some code. You don't need to honor this instruction if you haven't changed any code.

## Comments
For non-trivial patches/fixes, please leave comments in the code (only for the parts that aren't trivial) to understand.

## Commit messages
Please leave detailed commit messages if you're asked to make commits.

## J2ME Fullrun Corpus

The current real J2ME project regression corpus lives at `~/Projects/j2me_decomps`.
It can be run with:

```bash
j2me fullrun
```

But do NOT overuse this - it takes 40+ seconds to run! We also have a lot of decompiler tests, so for most fixes you should first do your normal decompiler work, and only in the very end when you're done, check fullrun to see if anything changes in a bad/good way.

Default behavior:
- writes reports/logs to `~/Projects/j2me_decomps/fullruns/fullrun_HH-mm_dd-mon*`
- decompiles into scratch work dirs, not project `decompiled/`
- tracks normalized decompiled Java in the Git repo:
  `~/Projects/j2me_decomps/fullruns/history/sources/<project>/`
- tracks compact status/diagnostics in:
  `~/Projects/j2me_decomps/fullruns/history/status/`
- uses `--history-mode snapshot` by default, so changes are staged but not committed

Inspect output changes with:

```bash
git -C ~/Projects/j2me_decomps/fullruns/history diff --cached
git -C ~/Projects/j2me_decomps/fullruns/history status --short
```

Record a new accepted baseline only after a good Sporeflower commit:

```bash
j2me fullrun --history-mode commit
```

Discard an exploratory snapshot with:

```bash
git -C ~/Projects/j2me_decomps/fullruns/history reset --hard
```

Useful options:
- `--history-mode off`: run without touching history
- `--keep-work none`: keep only reports/logs, delete scratch work
- `--keep-work all`: keep all scratch work for debugging
- `--in-place`: write to each project’s normal `decompiled/` and `out/`
