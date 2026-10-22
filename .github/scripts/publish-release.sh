#!/usr/bin/env bash
# Run from the repository root after building the distributions.
set -euo pipefail

cp toolkit/build/distributions/j2me-*-SNAPSHOT.zip sporeflower.zip
cp build/libs/sporeflower-*-SNAPSHOT.jar sporeflower.jar

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
  printf '\nJava 8+. `sporeflower.jar`: standalone decompiler; `sporeflower.zip`: full j2me toolkit. Both include J2ME API declarations.\n'
  printf '\nBuilt from [`%s`](%s/commit/%s).\n' "${GITHUB_SHA:0:8}" "$repository_url" "$GITHUB_SHA"
} > "$notes"

gh release create "$tag" sporeflower.zip sporeflower.jar \
  --target "$GITHUB_SHA" \
  --title "${release_date//./-}: ${title_subject}" \
  --notes-file "$notes" \
  --latest

# Retire the old moving tag only after replacement downloads exist.
if git show-ref --verify --quiet refs/tags/continuous; then
  gh release delete continuous --cleanup-tag --yes
fi
