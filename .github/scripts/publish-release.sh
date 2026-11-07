#!/usr/bin/env bash
# Run from the repository root with packaged binaries in dist/.
set -euo pipefail

release_date=$(date -u +%Y.%m.%d)
tag="v${release_date}.${GITHUB_RUN_NUMBER}.${GITHUB_RUN_ATTEMPT}"
subject=$(git show -s --format=%s "$GITHUB_SHA")
title_subject="$subject"
if (( ${#title_subject} > 80 )); then
  title_subject="${title_subject:0:77}..."
fi
previous=$(gh release list --exclude-drafts --limit 1 --json tagName --jq '.[0].tagName // empty')
repository_url="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}"
notes="${RUNNER_TEMP}/release-notes.md"
# Markdown backticks in printf formats are literal.
# shellcheck disable=SC2016
{
  if [[ -n "$previous" ]] && git merge-base --is-ancestor "refs/tags/$previous" "$GITHUB_SHA"; then
    changes=$(git log --no-merges --max-count=5 --format='- %s' "refs/tags/$previous..$GITHUB_SHA")
    if [[ -n "$changes" ]]; then
      printf '%s\n' "$changes"
      previous_commit=$(git rev-parse "refs/tags/$previous^{commit}")
      printf '\n[All changes](%s/compare/%s...%s)\n' "$repository_url" "$previous_commit" "$GITHUB_SHA"
    else
      printf 'Rebuild of `%s`.\n' "${GITHUB_SHA:0:8}"
    fi
  else
    printf '%s\n' "$subject"
  fi
  printf '\nEach archive contains the standalone `sporeflower` binary (Windows: `sporeflower.exe`), including J2ME API declarations. Extract and run; no Java runtime is needed for decompilation.\n'
  printf '\nWindows: x86_64. macOS and Linux: x86_64 or ARM64. Linux binaries are statically linked with musl.\n'
  printf '\nBuilt from [`%s`](%s/commit/%s).\n' "${GITHUB_SHA:0:8}" "$repository_url" "$GITHUB_SHA"
} > "$notes"

gh release create "$tag" dist/* \
  --target "$GITHUB_SHA" \
  --title "${release_date//./-}: ${title_subject}" \
  --notes-file "$notes" \
  --latest
