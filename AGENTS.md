# Sporeflower

Sporeflower is a Java decompiler focused on old J2ME programs: simpler language
features, but often minified names and unusual bytecode. It retains most of
Vineflower's architecture. Prefer general fixes within that architecture;
a small reproduction may require changes across several processing stages.
The `j2me` CLI lives in `toolkit/` and calls the decompiler API directly.

## Development and validation

- Use Java and Gradle through `./gradlew`.
- Check that regression fixtures faithfully represent the original bytecode.
  Compilable output can still be wrong; verify behavior where practical.
- Run focused tests while iterating, then the full suite with `./gradlew test`
  after code changes. After decompiler changes, also run `./gradlew jar`.
  Documentation-only changes need neither.
- Refresh `./gradlew :toolkit:installDist` before checking the CLI.
- Run builds, tests, and corpus jobs sequentially. Do not build or test native
  executables unless explicitly requested.
- Comment non-obvious logic, assumptions, and limitations.
- Commit messages should explain the problem and resulting behavior. Leave test
  counts and corpus statistics in the task report, not the commit message.

## Privacy

Keep original corpus program/class names, personal absolute paths, and private
artifacts out of tracked files and commit messages. Use neutral fixture names.
Investigation notes and temporary tools may stay locally, but must not be committed.
Local API stubs and compilers stay in gitignored `toolkit/vendor/` and must not enter release archives.

## J2ME regression corpus

Run `j2me fullrun --root /path/to/corpus` near the end of a decompiler fix;
it is expensive, so avoid using it for each iteration. Add `--project NAME`
for targeted rechecks.

Reports and scratch outputs live under the corpus's `fullruns/`. Fullrun stages
source and status snapshots in `fullruns/history/`; review with `git diff --cached`
there. Preserve pre-existing staged history; use `--history-dir` for isolated runs.
