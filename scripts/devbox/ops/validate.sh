#!/usr/bin/env bash
# -------------------------------------------------------------------
# scripts/devbox/ops/validate.sh: Validate repo structure + syntax
# @version 3.1.0
# Checks the JSON files and the shell syntax of the devbox scripts;
# exits 1 on any failure.
# -------------------------------------------------------------------
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../lib/common.sh"

dbx_header "Repo Validation"

fails=0

# Required files
for f in package.json devbox.json; do
  if [[ -f "${DBX_REPO_ROOT}/${f}" ]]; then
    dbx_ok "${f}" "found"
  else
    dbx_fail "${f}" "missing"
    fails=$((fails + 1))
  fi
done
if [[ -f "${DBX_REPO_ROOT}/tsconfig.json" ]]; then
  dbx_ok "tsconfig.json" "found"
fi

# JSON syntax (jq, else node)
for f in package.json devbox.json; do
  path="${DBX_REPO_ROOT}/${f}"
  [[ -f "${path}" ]] || continue
  if dbx_has jq; then
    valid_cmd=(jq empty "${path}")
  elif dbx_has node; then
    valid_cmd=(node -e 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))' "${path}")
  else
    dbx_warn "${f} syntax" "no jq or node, skipped"
    continue
  fi
  if "${valid_cmd[@]}" >/dev/null 2>&1; then
    dbx_ok "${f} syntax" "valid JSON"
  else
    dbx_fail "${f} syntax" "invalid JSON"
    fails=$((fails + 1))
  fi
done

# Shell syntax of the devbox scripts
scripts_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
while IFS= read -r -d '' script; do
  if bash -n "${script}" 2>/dev/null; then
    :
  else
    dbx_fail "bash -n" "${script#${DBX_REPO_ROOT}/}"
    fails=$((fails + 1))
  fi
done < <(find "${scripts_dir}" -name '*.sh' -type f -print0)

echo
if [[ "${fails}" -gt 0 ]]; then
  echo "Validation failed: ${fails} problem(s)."
  exit 1
fi
echo "Validation complete."
