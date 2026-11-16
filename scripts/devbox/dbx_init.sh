#!/usr/bin/env bash
# -------------------------------------------------------------------
# scripts/devbox/dbx_init.sh: DevBox initialization
# @version 3.0.0
# @purpose Detect environment, validate tooling, load OS overrides,
#          display status matrix.
# -------------------------------------------------------------------

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"

# -------------------------------------------------------------------
# Metadata (injected by workspace template or devbox.json env vars)
# -------------------------------------------------------------------
: "${C_DBX_META_TEAM:=NEXUS-Labs}"
: "${C_DBX_META_TEAM_ID:=NEXUS-Labs-DD}"
: "${C_DBX_META_VERSION:=3.0.0}"
: "${C_DBX_META_SAMPLE:=}"
: "${C_DBX_SKIP_API_CHECKS:=true}"

# -------------------------------------------------------------------
# Status registry (bash 3.x compatible: parallel arrays)
# -------------------------------------------------------------------
_STATUS_KEYS=()
_STATUS_VALS=()

_record() { _STATUS_KEYS+=("$1"); _STATUS_VALS+=("${2}|${3}"); }  # key, code, note

# Explicit `return 0` on both paths -- found live (alpha-test, 2026-09-06):
# under `set -euo pipefail` (dbx_init.sh's own top line), a bare `return`
# after the for-loop falls through with the exit status of the loop's last
# failed `[[ ]]` test when the key isn't found, which print_matrix's
# `val="$(_get_status "${key}")"` command-substitution assignment then
# propagates as a hard script abort -- silently, with zero error output,
# before print_matrix or print_help ever run. This stayed dormant as long
# as every key in print_matrix's ordered_keys had a matching _record call,
# but any customization that removes one check function (exactly what
# happened testing this) without also trimming ordered_keys reintroduces
# it. Returning 0 unconditionally makes "key not yet recorded" the normal,
# safe "not found" case it always should have been.
_get_status() {
  local i
  for i in "${!_STATUS_KEYS[@]}"; do
    [[ "${_STATUS_KEYS[$i]}" == "$1" ]] && { echo "${_STATUS_VALS[$i]}"; return 0; }
  done
  return 0
}

# -------------------------------------------------------------------
# Checks
# -------------------------------------------------------------------
check_core_tools() {
  for tool in git node npm jq; do
    if dbx_has "${tool}"; then
      _record "${tool}" ok "$(command -v "${tool}")"
    else
      _record "${tool}" fail "not found"
    fi
  done
}

check_npm_path() {
  if ! dbx_has npm; then
    _record "npm-path" warn "npm missing"
    return 0
  fi
  local npm_bin
  npm_bin="$(npm config get prefix 2>/dev/null)/bin"
  if [[ -d "${npm_bin}" && ":${PATH}:" != *":${npm_bin}:"* ]]; then
    export PATH="${npm_bin}:${PATH}"
  fi
  _record "npm-path" ok "global bin on PATH"
}

check_ai_tools() {
  # Claude CLI
  if dbx_has claude; then
    _record "claude-cli" ok "installed"
  else
    _record "claude-cli" warn "not found"
  fi

  # API keys
  if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
    if dbx_has claude; then
      _record "anthropic-key" warn "present -- may override Claude Max/Pro OAuth login for claude-cli! unset if using a subscription"
    else
      _record "anthropic-key" ok "present"
    fi
  else
    _record "anthropic-key" info "not set (expected with a Claude Max/Pro subscription)"
  fi
  [[ -n "${OPENAI_API_KEY:-}" ]]    && _record "openai-key" ok "present"    || _record "openai-key" info "not set (optional)"
  [[ -n "${GOOGLE_API_KEY:-}" ]]    && _record "gemini-key" ok "present"    || _record "gemini-key" info "not set (optional)"

  # Taskmaster
  if dbx_has task-master; then
    _record "taskmaster" ok "installed"
  else
    _record "taskmaster" info "not installed (optional)"
  fi
}

check_dev_tools() {
  # GitHub CLI
  if dbx_has gh; then
    local gh_login gh_via="global login"
    [[ -n "${GH_CONFIG_DIR:-}" && -f "${DBX_REPO_ROOT}/.nexus/gh.env" ]] && gh_via="project profile"
    # Active account from the account list (no API call); "as <login>" is
    # the format of gh releases before multi-account support. gh exits 1
    # when no account is logged in (and awk may close the pipe early); under
    # set -euo pipefail that would end dbx_init.sh without a word, so the
    # exit status is ignored and the parsed login decides.
    gh_login="$( { gh auth status --hostname "${GH_HOST:-github.com}" 2>&1 || true; } | awk '
      /Logged in to/ { for (i = 1; i < NF; i++) if ($i == "account" || $i == "as") acct = $(i + 1); if (!first) first = acct }
      /Active account: true/ { print acct; found = 1; exit }
      END { if (!found && first) print first }')"
    if [[ -n "${gh_login}" ]]; then
      _record "gh" ok "${gh_login} (${gh_via})"
    elif [[ "${gh_via}" == "project profile" ]]; then
      _record "gh" warn "project profile not logged in: nexus git login"
    else
      _record "gh" warn "unauthenticated"
    fi
  else
    _record "gh" warn "not found"
  fi

  # Ansible
  dbx_has ansible && _record "ansible" ok "on PATH" || _record "ansible" info "not installed (optional)"

  # Cursor
  if dbx_has cursor; then
    _record "cursor" ok "on PATH"
  elif [[ "${DBX_OS}" == "darwin" && -d "/Applications/Cursor.app" ]]; then
    _record "cursor" info "use: open -a Cursor ."
  else
    _record "cursor" info "not installed (optional)"
  fi

  # Docker
  dbx_has docker && _record "docker" ok "available" || _record "docker" info "not installed (optional)"
}

# Self-healing gitleaks pre-commit wiring, resolved against the repo root so
# it also works when invoked from a subdirectory.
check_git_hooks() {
  if dbx_has gitleaks; then
    _record "gitleaks" ok "$(gitleaks version 2>/dev/null | head -1)"
  else
    _record "gitleaks" fail "not found -- secret-scan pre-commit hook cannot run"
  fi

  local repo_root
  repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  if [[ -n "${repo_root}" ]]; then
    local current_hooks_path
    current_hooks_path="$(git -C "${repo_root}" config --get core.hooksPath 2>/dev/null || true)"
    if [[ -f "${repo_root}/.githooks/pre-commit" ]]; then
      if [[ "${current_hooks_path}" != ".githooks" ]]; then
        git -C "${repo_root}" config core.hooksPath .githooks
      fi
      _record "git-hooks" ok "core.hooksPath=.githooks (gitleaks wired)"
    elif [[ -f "${repo_root}/.pre-commit-config.yaml" ]] && dbx_has pre-commit; then
      if ! grep -q "File generated by pre-commit" "${repo_root}/.git/hooks/pre-commit" 2>/dev/null; then
        (cd "${repo_root}" && pre-commit install --install-hooks &>/dev/null) || true
      fi
      if grep -q "File generated by pre-commit" "${repo_root}/.git/hooks/pre-commit" 2>/dev/null; then
        _record "git-hooks" ok "pre-commit framework installed (gitleaks wired)"
      else
        _record "git-hooks" warn "pre-commit framework not installed -- run 'pre-commit install --install-hooks'"
      fi
    else
      _record "git-hooks" warn "no .githooks/ or .pre-commit-config.yaml found"
    fi
  fi
}

# -------------------------------------------------------------------
# Status output: one summary line by default, with only warnings and
# failures listed; DBX_VERBOSE=1 prints the full matrix.
# -------------------------------------------------------------------
# Shorten tool paths: devbox profile -> "devbox", nix store -> "nix",
# home directory -> "~".
_short_note() {
  local note="$1"
  case "${note}" in
    "${DBX_REPO_ROOT}"/.devbox/*) note="devbox" ;;
    /nix/store/*) note="nix" ;;
    "${HOME}"/*) note="~${note#"${HOME}"}" ;;
  esac
  printf "%s" "${note}"
}

_matrix_row() {
  local key="$1" code="$2" note
  note="$(_short_note "$3")"
  case "${code}" in
    ok)   printf "  %-16s ${C_GRN}%-4s${RST} %s\n" "${key}" "$(dbx_icon ok)" "${note}" ;;
    warn) printf "  %-16s ${C_YLW}%-4s${RST} %s\n" "${key}" "$(dbx_icon warn)" "${note}" ;;
    fail) printf "  %-16s ${C_RED}%-4s${RST} %s\n" "${key}" "$(dbx_icon fail)" "${note}" ;;
    info) printf "  %-16s ${C_GRY}%-4s${RST} ${DIM}%s${RST}\n" "${key}" "$(dbx_icon info)" "${note}" ;;
  esac
}

print_matrix() {
  # Deterministic display order
  local ordered_keys=(
    git node npm jq npm-path
    claude-cli anthropic-key openai-key gemini-key taskmaster
    gh ansible cursor docker
    gitleaks git-hooks
    nexus-env env.nexus.local plugin-env
  )
  local n_ok=0 n_warn=0 n_fail=0 key val code note
  # Rows from init/checks.d or an OS override follow, in recording order.
  local extra known
  for extra in ${_STATUS_KEYS[@]+"${_STATUS_KEYS[@]}"}; do
    known=0
    for key in "${ordered_keys[@]}"; do
      [[ "${key}" == "${extra}" ]] && { known=1; break; }
    done
    (( known == 1 )) || ordered_keys+=("${extra}")
  done
  for key in "${ordered_keys[@]}"; do
    val="$(_get_status "${key}")"
    [[ -z "${val}" ]] && continue
    IFS='|' read -r code note <<< "${val}"
    case "${code}" in ok) n_ok=$((n_ok + 1)) ;; warn) n_warn=$((n_warn + 1)) ;; fail) n_fail=$((n_fail + 1)) ;; esac
  done

  echo
  local status_color="${C_GRN}"
  (( n_warn > 0 )) && status_color="${C_YLW}"
  (( n_fail > 0 )) && status_color="${C_RED}"
  printf "${BLD}DevBox${RST}  %s/%s v%s ${DIM}·${RST} %s ${DIM}·${RST} ${status_color}%d ok, %d warn, %d fail${RST}\n" \
    "${C_DBX_META_TEAM}" "${C_DBX_META_TEAM_ID}" "${C_DBX_META_VERSION}" "${DBX_OS}" "${n_ok}" "${n_warn}" "${n_fail}"
  # Sample workspaces (meta var set by the sample customer) say so up front
  if [[ -n "${C_DBX_META_SAMPLE}" ]]; then
    printf "  ${C_YLW}Sample workspace${RST} ${DIM}·${RST} %s: an example environment, not a real project\n" "${C_DBX_META_SAMPLE}"
  fi
  for key in "${ordered_keys[@]}"; do
    val="$(_get_status "${key}")"
    [[ -z "${val}" ]] && continue
    IFS='|' read -r code note <<< "${val}"
    if [[ "${DBX_VERBOSE:-0}" == "1" || "${code}" == "warn" || "${code}" == "fail" ]]; then
      _matrix_row "${key}" "${code}" "${note}"
    fi
  done
  [[ "${DBX_VERBOSE:-0}" == "1" ]] || printf "  ${DIM}all checks: DBX_VERBOSE=1 devbox shell${RST}\n"
}

# -------------------------------------------------------------------
# Commands: read from devbox.json, so the list never drifts from the
# scripts that actually exist.
# -------------------------------------------------------------------
print_help() {
  local cfg="${DBX_REPO_ROOT}/devbox.json" rows name cmd width=0
  dbx_has jq && [[ -f "${cfg}" ]] || return 0
  # name<TAB>command (multi-step scripts joined with &&), sorted by name
  rows="$(jq -r '.shell.scripts // {} | to_entries | sort_by(.key)[]
    | "\(.key)\t\(.value | if type == "array" then join(" && ") else tostring end)"' "${cfg}" 2>/dev/null || true)"
  [[ -n "${rows}" ]] || return 0
  while IFS=$'\t' read -r name cmd; do
    (( ${#name} > width )) && width=${#name}
  done <<< "${rows}"
  printf "\n${BLD}Commands${RST} ${DIM}devbox run <name>${RST}\n"
  while IFS=$'\t' read -r name cmd; do
    printf "  ${C_CYN}%-*s${RST}  ${DIM}%s${RST}\n" "${width}" "${name}" "${cmd}"
  done <<< "${rows}"
  echo
}

# -------------------------------------------------------------------
# .nexus/plugin.env: plugin variables managed by the Nexus platform
# (written by nexus pull from the project settings, e.g. HEADROOM_*). The
# first init_hook entry loads it into the devbox environment.
# -------------------------------------------------------------------
check_plugin_env() {
  if [[ -f "${DBX_REPO_ROOT}/.nexus/plugin.env" ]]; then
    _record "plugin-env" ok "loaded from .nexus/plugin.env"
  else
    _record "plugin-env" info "none (no plugin vars, or run nexus pull)"
  fi
}

# -------------------------------------------------------------------
# .env.nexus.local: Nexus environment file check
# -------------------------------------------------------------------
check_nexus_env() {
  local env_file="${DBX_REPO_ROOT}/.env.nexus.local"
  if [[ -f "${env_file}" ]]; then
    # shellcheck source=/dev/null
    source "${env_file}"
    _record "env.nexus.local" ok "loaded"
  else
    _record "env.nexus.local" info "not found (optional)"
  fi
}

# -------------------------------------------------------------------
# Main
# -------------------------------------------------------------------
dbx_detect_os
check_npm_path
check_core_tools
check_ai_tools
check_dev_tools
check_git_hooks
check_plugin_env
check_nexus_env
dbx_load_os_override
dbx_load_project_checks
dbx_load_overlay

print_matrix
print_help
