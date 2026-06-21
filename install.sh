#!/usr/bin/env bash
# Nexus Link installer
#
# Usage:
#   curl -fsSL https://nexus.gatewarden.eu/install-link.sh | bash
#
# Options (via env vars):
#   NEXUS_LINK_VERSION   Pin a specific version tag, e.g. "v0.1.0" (default: latest)
#   NEXUS_LINK_BIN_DIR   Custom install directory (default: ~/.local/bin or ~/.cargo/bin)
#   GITHUB_TOKEN         GitHub PAT (optional -- only needed for higher rate limits)
#
# The installer downloads a pre-built binary from GitHub Releases.
# If no binary exists for the current platform it falls back to `cargo install`.

set -euo pipefail

REPO="gwnexus/nexus-link"
BINARY_NAME="nexus-link"

# -- colours (disabled when piped) --
if [ -t 1 ]; then
  BOLD="\033[1m"  GREEN="\033[32m"  YELLOW="\033[33m"
  RED="\033[31m"   CYAN="\033[36m"   RESET="\033[0m"
else
  BOLD="" GREEN="" YELLOW="" RED="" CYAN="" RESET=""
fi

info()  { printf "${BOLD}${CYAN}info${RESET}  %s\n" "$1"; }
ok()    { printf "${BOLD}${GREEN}  ok${RESET}  %s\n" "$1"; }
warn()  { printf "${BOLD}${YELLOW}warn${RESET}  %s\n" "$1"; }
err()   { printf "${BOLD}${RED} err${RESET}  %s\n" "$1" >&2; }
die()   { err "$1"; exit 1; }

# -- detect platform --
detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux*)  OS="linux"  ;;
    Darwin*) OS="darwin" ;;
    *)       die "Unsupported OS: $os (nexus-link targets Linux nodes)" ;;
  esac

  case "$arch" in
    x86_64|amd64)  ARCH="x86_64"  ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)             die "Unsupported architecture: $arch" ;;
  esac

  case "${OS}-${ARCH}" in
    linux-aarch64) TARGET="aarch64-unknown-linux-gnu"
                   ASSET_SLUG="aarch64-linux" ;;
    linux-x86_64)  TARGET="x86_64-unknown-linux-gnu"
                   ASSET_SLUG="x86_64-linux" ;;
    darwin-*)      TARGET="${ARCH}-apple-darwin"
                   ASSET_SLUG="${ARCH}-darwin"
                   warn "nexus-link is designed for Linux nodes. macOS install is for development only." ;;
    *)             die "No target triple for ${OS}-${ARCH}" ;;
  esac

  info "Platform: ${OS}/${ARCH} -> ${ASSET_SLUG}"
}

# -- resolve install dir --
resolve_bin_dir() {
  if [ -n "${NEXUS_LINK_BIN_DIR:-}" ]; then
    BIN_DIR="$NEXUS_LINK_BIN_DIR"
  elif [ -d "$HOME/.local/bin" ]; then
    BIN_DIR="$HOME/.local/bin"
  elif [ -d "$HOME/.cargo/bin" ]; then
    BIN_DIR="$HOME/.cargo/bin"
  else
    BIN_DIR="$HOME/.local/bin"
  fi
  mkdir -p "$BIN_DIR"
}

# -- authenticated curl helper --
auth_curl() {
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    curl -H "Authorization: token ${GITHUB_TOKEN}" "$@"
  else
    curl "$@"
  fi
}

# -- check GitHub auth --
check_github_auth() {
  if [ -z "${GITHUB_TOKEN:-}" ]; then
    if command -v gh &>/dev/null && gh auth status &>/dev/null 2>&1; then
      GITHUB_TOKEN="$(gh auth token 2>/dev/null || true)"
    fi
  fi

  if [ -n "${GITHUB_TOKEN:-}" ]; then
    info "GitHub auth: authenticated"
  else
    info "GitHub auth: anonymous (public repo)"
  fi
}

# -- try binary download --
try_binary_download() {
  local version="${NEXUS_LINK_VERSION:-latest}"
  local api_base="https://api.github.com/repos/${REPO}"
  local api_url

  if [ "$version" = "latest" ]; then
    api_url="${api_base}/releases/latest"
  else
    api_url="${api_base}/releases/tags/${version}"
  fi

  info "Checking for pre-built binary (${version})..."

  local http_code
  http_code="$(auth_curl -sL -w "%{http_code}" -o /tmp/nexus_link_release.json \
    -H "Accept: application/vnd.github+json" \
    "$api_url" 2>/dev/null || echo "000")"

  if [ "$http_code" != "200" ]; then
    warn "No release found (HTTP ${http_code}) -- falling back to source build"
    return 1
  fi

  local release_json
  release_json="$(cat /tmp/nexus_link_release.json)"
  rm -f /tmp/nexus_link_release.json

  # Expected asset: nexus-link-<slug>.tar.gz (e.g. nexus-link-aarch64-linux.tar.gz)
  local asset_name="nexus-link-${ASSET_SLUG}.tar.gz"
  local asset_id
  asset_id="$(echo "$release_json" \
    | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a['name'] == '${asset_name}':
        print(a['id'])
        break
" 2>/dev/null || echo "")"

  if [ -z "$asset_id" ]; then
    warn "No asset '${asset_name}' in release -- falling back to source build"
    return 1
  fi

  info "Downloading ${asset_name}..."

  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap "rm -rf '$tmp_dir'" EXIT

  auth_curl -fsSL \
    -H "Accept: application/octet-stream" \
    -o "${tmp_dir}/${asset_name}" \
    "${api_base}/releases/assets/${asset_id}"

  # Verify checksum if available
  local sha_id
  sha_id="$(echo "$release_json" \
    | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a['name'] == '${asset_name}.sha256':
        print(a['id'])
        break
" 2>/dev/null || echo "")"

  if [ -n "$sha_id" ]; then
    auth_curl -fsSL \
      -H "Accept: application/octet-stream" \
      -o "${tmp_dir}/${asset_name}.sha256" \
      "${api_base}/releases/assets/${sha_id}"

    info "Verifying SHA256 checksum..."
    cd "$tmp_dir"
    if command -v sha256sum &>/dev/null; then
      sha256sum -c "${asset_name}.sha256"
    elif command -v shasum &>/dev/null; then
      shasum -a 256 -c "${asset_name}.sha256"
    fi
    cd - >/dev/null
  fi

  tar -xzf "${tmp_dir}/${asset_name}" -C "$tmp_dir"

  # Install all binaries from the archive
  for bin in nexus-link nexus-link-agent nexus-link-service; do
    local found=""
    for candidate in "${tmp_dir}/${bin}" "${tmp_dir}/nexus-link-${ASSET_SLUG}/${bin}" "${tmp_dir}/dist/${bin}"; do
      if [ -f "$candidate" ]; then
        found="$candidate"
        break
      fi
    done
    if [ -n "$found" ]; then
      mv "$found" "${BIN_DIR}/${bin}"
      chmod +x "${BIN_DIR}/${bin}"
      ok "Installed ${bin} -> ${BIN_DIR}/${bin}"
    fi
  done

  local release_tag
  release_tag="$(echo "$release_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tag_name','?'))" 2>/dev/null || echo "?")"

  ok "Nexus Link ${release_tag} installed successfully"
  return 0
}

# -- build from source (fallback) --
build_from_source() {
  if ! command -v cargo &>/dev/null; then
    die "No pre-built binary for ${TARGET} and Rust toolchain not found.
  Install Rust first: https://rustup.rs
  Then re-run this installer."
  fi

  local rust_ver major minor
  rust_ver="$(rustc --version | awk '{print $2}')"
  major="$(echo "$rust_ver" | cut -d. -f1)"
  minor="$(echo "$rust_ver" | cut -d. -f2)"
  if [ "$major" -lt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -lt 85 ]; }; then
    die "Rust >= 1.85 required (found ${rust_ver}). Run: rustup update stable"
  fi

  info "Building from source with cargo (Rust ${rust_ver})..."

  local repo_url="https://github.com/${REPO}.git"
  local version="${NEXUS_LINK_VERSION:-}"
  local version_args=()
  if [ -n "$version" ] && [ "$version" != "latest" ]; then
    version_args=(--tag "$version")
  fi

  RUSTFLAGS="" cargo install \
    --git "$repo_url" \
    "${version_args[@]}" \
    nexus-link-cli \
    --locked 2>&1 || {
      warn "Retrying without --locked..."
      RUSTFLAGS="" cargo install \
        --git "$repo_url" \
        "${version_args[@]}" \
        nexus-link-cli 2>&1
    }

  ok "Built and installed from source"
}

# -- verify --
verify_install() {
  if command -v "$BINARY_NAME" &>/dev/null; then
    local ver
    ver="$("$BINARY_NAME" --version 2>/dev/null || echo "unknown")"
    ok "${ver}"
  elif [ -x "${BIN_DIR}/${BINARY_NAME}" ]; then
    local ver
    ver="$("${BIN_DIR}/${BINARY_NAME}" --version 2>/dev/null || echo "unknown")"
    ok "${ver}"
    warn "${BIN_DIR} is not in your PATH. Add it:"
    echo ""
    echo "  export PATH=\"${BIN_DIR}:\$PATH\""
    echo ""
  else
    die "Installation failed -- '${BINARY_NAME}' binary not found"
  fi
}

# -- main --
main() {
  echo ""
  printf "${BOLD}Nexus Link Installer${RESET}\n"
  echo "============================="
  echo ""

  detect_platform
  resolve_bin_dir
  check_github_auth

  if ! try_binary_download; then
    build_from_source
  fi

  verify_install

  echo ""
  ok "Done! Run '${BINARY_NAME} --help' to get started."
  echo ""
  echo "  Quick start:"
  echo "    ${BINARY_NAME} register --token <nxs_node_*>   # register this node"
  echo "    ${BINARY_NAME} agent start                      # start telemetry push"
  echo "    ${BINARY_NAME} status                           # check node status"
  echo ""
}

main "$@"
