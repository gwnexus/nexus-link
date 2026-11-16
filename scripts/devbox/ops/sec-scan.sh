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
  hits=$(git -C "${DBX_REPO_ROOT}" ls-files -z \
    | (cd "${DBX_REPO_ROOT}" && xargs -0 rg -il --no-messages \
        -e '-----BEGIN [A-Z ]*PRIVATE KEY-----' \
        -e '(password|secret|api_key)\s*=\s*["\x27][^"\x27]{8,}["\x27]' -- 2>/dev/null) || true)
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
