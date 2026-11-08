#!/usr/bin/env bash
# Open or refresh ONE tracking issue when a dependency check fails.
# Called from .github/workflows/deps.yml on `if: failure()`.
# Requires GH_TOKEN (the default GITHUB_TOKEN) and `permissions: issues: write`.
set -uo pipefail

title="deps: dependency drift or advisory"
tmp="${RUNNER_TEMP:-/tmp}"
body="$tmp/deps-issue-body.md"
sha="${GITHUB_SHA:-0000000}"

{
  echo "The dependency check failed on \`${GITHUB_REF_NAME:-unknown}\` at \`${sha:0:7}\`."
  echo
  echo "- run: ${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-unknown}/actions/runs/${GITHUB_RUN_ID:-unknown}"
  echo "- job: ${GITHUB_JOB:-unknown}"
  echo
  echo "Fix by bumping the offending crate(s) in \`Cargo.toml\`/\`Cargo.lock\`, or — only after a"
  echo "deliberate decision — add a license exception in \`deny.toml\`. See \`DEPENDENCIES.md\`."
  echo
  for f in audit.txt deny.txt outdated.txt; do
    [ -s "$tmp/$f" ] || continue
    echo "<details><summary><code>$f</code></summary>"
    echo
    echo '```'
    # Cap each section: GitHub rejects issue bodies over 65 536 characters.
    tail -c 20000 "$tmp/$f"
    echo
    echo '```'
    echo
    echo "</details>"
    echo
  done
} > "$body"

# Exact-title match, so a lookalike issue is never reused.
found="$(gh issue list --state all --limit 100 --search "\"$title\" in:title" \
  --json number,title,state --jq '.[] | "\(.number)\t\(.state)\t\(.title)"' \
  | awk -F'\t' -v t="$title" '$3 == t { print $1 "\t" $2; exit }')"

if [ -z "$found" ]; then
  gh issue create --title "$title" --body-file "$body"
  exit 0
fi

number="${found%%$'\t'*}"
state="${found##*$'\t'}"

gh issue comment "$number" --body-file "$body"
if [ "$state" = "CLOSED" ]; then
  gh issue reopen "$number"
fi
# Keep the body current so the issue always shows the newest failure.
gh issue edit "$number" --body-file "$body"
