#!/usr/bin/env bash
# Run the release pipeline without publishing (GitHub workflow
# "Release Dry Run") and wait for the result.
#
# Usage:
#   scripts/ci/release-dryrun.sh [ref]
#
#   ref   Branch to run on (default: the current branch). The ref must be
#         pushed; the workflow builds what is on GitHub, not local changes.
#
# Requires an authenticated GitHub CLI (`gh auth status`).
# Exit code: 0 when the dry run succeeded, non-zero otherwise.

set -euo pipefail

WORKFLOW="release-dryrun.yml"

die() {
  echo "error: $*" >&2
  exit 1
}

command -v gh >/dev/null 2>&1 || die "GitHub CLI (gh) not found"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated (run: gh auth login)"

ref="${1:-$(git rev-parse --abbrev-ref HEAD)}"
[ "$ref" != "HEAD" ] || die "detached HEAD; pass a branch name"

local_sha=$(git rev-parse "$ref" 2>/dev/null || true)
remote_sha=$(git ls-remote --heads origin "$ref" | cut -f1)
[ -n "$remote_sha" ] || die "branch '$ref' does not exist on origin; push it first"
if [ -n "$local_sha" ] && [ "$local_sha" != "$remote_sha" ]; then
  echo "warning: local '$ref' ($local_sha) differs from origin ($remote_sha); the dry run uses origin" >&2
fi

echo "Starting $WORKFLOW on '$ref' ($remote_sha)..."
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run "$WORKFLOW" --ref "$ref"

# gh workflow run does not return the run id; find the run it created.
run_id=""
for _ in $(seq 1 30); do
  run_id=$(gh run list --workflow "$WORKFLOW" --branch "$ref" --event workflow_dispatch \
    --created ">=$started_at" --limit 1 --json databaseId --jq '.[0].databaseId // empty')
  [ -n "$run_id" ] && break
  sleep 2
done
[ -n "$run_id" ] || die "could not find the started run; check: gh run list --workflow $WORKFLOW"

url=$(gh run view "$run_id" --json url --jq .url)
echo "Run: $url"

status=0
gh run watch "$run_id" --exit-status --interval 15 || status=$?

echo
echo "Artifacts:"
gh api "repos/{owner}/{repo}/actions/runs/$run_id/artifacts" \
  --jq '.artifacts[] | "  \(.name) (\(.size_in_bytes) bytes)"' || true

if [ "$status" -eq 0 ]; then
  echo "Release dry run succeeded: $url"
else
  echo "Release dry run FAILED: $url" >&2
fi
exit "$status"
