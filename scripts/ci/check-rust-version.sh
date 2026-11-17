#!/usr/bin/env bash
# Single source of truth for the Rust version: rust-toolchain.toml.
#
# CI, the release build and devbox (rustup) read rust-toolchain.toml
# directly. The Docker builder images cannot, so they carry the same version
# as `ARG RUST_VERSION=<x.y.z>` default; this script keeps them in sync.
#
# It also checks that no image in docker/ floats: no `:latest`, and the
# Debian runtime base is pinned by digest.
#
# Usage:
#   scripts/ci/check-rust-version.sh          # check, exit 1 on drift
#   scripts/ci/check-rust-version.sh --fix    # rewrite the ARG defaults
set -euo pipefail

cd "$(dirname "$0")/../.."

fix=false
[ "${1:-}" = "--fix" ] && fix=true

channel=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
if ! [[ "$channel" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "::error file=rust-toolchain.toml::channel '$channel' is not an exact x.y.z version"
  exit 1
fi
echo "rust-toolchain.toml: $channel"

status=0
for f in docker/Dockerfile*; do
  if grep -q '^FROM rust:' "$f" || grep -q 'RUST_VERSION' "$f"; then
    current=$(sed -n 's/^ARG RUST_VERSION=\(.*\)$/\1/p' "$f")
    if [ -z "$current" ]; then
      echo "::error file=$f::builder needs 'ARG RUST_VERSION=$channel' before FROM"
      status=1
    elif [ "$current" != "$channel" ]; then
      if $fix; then
        sed -i.bak "s/^ARG RUST_VERSION=.*/ARG RUST_VERSION=$channel/" "$f" && rm -f "$f.bak"
        echo "$f: RUST_VERSION $current -> $channel (fixed)"
      else
        echo "::error file=$f::RUST_VERSION $current differs from rust-toolchain.toml $channel (run $0 --fix)"
        status=1
      fi
    else
      echo "$f: RUST_VERSION $current"
    fi
  fi

  if grep -nE '^FROM [^ ]+:latest( |$)' "$f"; then
    echo "::error file=$f::floating ':latest' base image"
    status=1
  fi
  if grep -nE '^FROM debian:[^@ ]+( |$)' "$f"; then
    echo "::error file=$f::debian base image must be pinned by digest (debian:<tag>@sha256:...)"
    status=1
  fi
done

exit $status
