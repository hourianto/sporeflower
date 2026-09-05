# Sporeflower

Sporeflower is a Java decompiler focused on old J2ME programs: simpler language
features, but often minified names and unusual bytecode. It retains most of
Vineflower's architecture. Prefer general fixes within that architecture;
a small reproduction may require changes across several processing stages.

## Development and validation

- Use Java and Gradle through `./gradlew`.
- Check that regression fixtures faithfully represent the original bytecode.
  Compilable output can still be wrong; verify behavior where practical.
- Run focused tests while iterating, then the full suite with `./gradlew test`
  after code changes. After decompiler changes, also run `./gradlew jar`.
  Documentation-only changes need neither.
- Comment non-obvious logic, assumptions, and limitations.
- When asked to commit, explain the problem and resulting behavior. Leave test
  counts and corpus statistics in the task report, not the commit message.

## Privacy

Keep original corpus program/class names, personal absolute paths, and private
artifacts out of tracked files and commit messages. Use neutral fixture names.
Original identities and investigation evidence belong in chat or ignored local
notes such as `bugs.md`. Treat those notes as leads to verify, not authoritative
current test results.

## J2ME regression corpus

The corpus lives at `~/Projects/j2me_decomps`. Run `j2me fullrun` near the end of
a decompiler fix; it is expensive, so avoid using it for each iteration.
Use `j2me fullrun --project NAME` for targeted rechecks.

By default, fullrun uses scratch workspaces and stages an uncommitted history
snapshot. Reports and logs go under `fullruns/`; normalized sources and compact
status/diagnostics live in `fullruns/history/{sources,status}/`.

Review changes with:

```sh
git -C ~/Projects/j2me_decomps/fullruns/history diff --cached
```

Preserve any pre-existing staged history when reviewing or discarding a run.
`--history-mode commit` records an accepted baseline; `--keep-work all` retains
scratch output for debugging. Use `--history-mode off` or `--keep-work none`
only when deliberately foregoing that history or debugging evidence.
`--in-place` writes to the projects' normal output directories.
