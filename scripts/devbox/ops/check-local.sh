#!/usr/bin/env bash
# -------------------------------------------------------------------
# scripts/devbox/ops/check-local.sh: Validate local environment
# @version 3.1.0
# Exits 1 when a required tool is missing (optional ones only warn).
# -------------------------------------------------------------------
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../lib/common.sh"

dbx_detect_os
dbx_header "Local Environment Check"

fails=0

# Git (required)
if dbx_has git; then
  dbx_ok "git" "$(git --version)"
else
  dbx_fail "git" "not found"
  fails=$((fails + 1))
fi

# Node
if dbx_has node; then
  dbx_ok "node" "$(node --version)"
else
  dbx_warn "node" "not found"
fi

# npm
if dbx_has npm; then
  dbx_ok "npm" "$(npm --version)"
else
  dbx_warn "npm" "not found"
fi

echo
if [[ "${fails}" -gt 0 ]]; then
  echo "Local check failed: ${fails} required tool(s) missing."
  exit 1
fi
echo "Local check complete."
