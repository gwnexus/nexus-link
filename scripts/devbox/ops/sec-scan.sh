#!/usr/bin/env bash
# -------------------------------------------------------------------
# scripts/devbox/ops/sec-scan.sh: Security & secrets scan
# @version 3.1.0
# gitleaks over the git history (the version CI pins); without
# gitleaks, a pattern scan of tracked files. Exits 1 on findings.
# -------------------------------------------------------------------
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../lib/common.sh"

dbx_header "Security Scan"

if dbx_has gitleaks; then
  echo "Scanning the git history with gitleaks (values redacted)..."
  if gitleaks git --no-banner --redact --verbose "${DBX_REPO_ROOT}"; then
    dbx_ok "gitleaks" "no leaks found"
  else
    dbx_fail "gitleaks" "possible secrets found (see above)"
    exit 1
  fi
elif dbx_has rg && dbx_has git; then
  echo "gitleaks not available; pattern scan of tracked files..."
  listing="$(mktemp)"
  trap 'rm -f "${listing}"' EXIT
  if ! git -C "${DBX_REPO_ROOT}" ls-files -z > "${listing}"; then
    dbx_fail "secrets scan" "git ls-files failed"
    exit 1
  fi
  files=()
  while IFS= read -r -d '' f; do files+=("${f}"); done < "${listing}"
  hits=""
  if (( ${#files[@]} > 0 )); then
    # rg exit codes: 0 = matches, 1 = no matches, 2 = error (e.g. unreadable file)
    rc=0
    hits=$(cd "${DBX_REPO_ROOT}" && rg -il \
        -e '-----BEGIN [A-Z ]*PRIVATE KEY-----' \
        -e '(password|secret|api_key)\s*=\s*["\x27][^"\x27]{8,}["\x27]' \
        -- "${files[@]}") || rc=$?
    if (( rc > 1 )); then
      dbx_fail "secrets scan" "ripgrep failed (exit ${rc}), scan incomplete"
      exit 1
    fi
  fi
  if [[ -n "${hits}" ]]; then
    dbx_fail "potential secrets" "in tracked files:"
    sed 's/^/    /' <<< "${hits}"
    exit 1
  fi
  dbx_ok "secrets scan" "no obvious patterns in tracked files"
else
  dbx_warn "scanner" "neither gitleaks nor ripgrep available, scan skipped"
fi

echo
echo "Security scan complete."
