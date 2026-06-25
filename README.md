# Nexus Link

[![Release](https://github.com/gwnexus/nexus-link/actions/workflows/release.yml/badge.svg)](https://github.com/gwnexus/nexus-link/releases)
[![CI](https://github.com/gwnexus/nexus-link/actions/workflows/ci.yml/badge.svg)](https://github.com/gwnexus/nexus-link/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org)

Hardware agent for the [Gatewarden Nexus](https://nexus.gatewarden.eu) platform.
Connects on-premise AI hardware nodes (NVIDIA DGX Spark, A100, H100) to the
Nexus backend: live telemetry push and remote compose management — without
requiring inbound network access into your infrastructure.

## Install

### One-liner (recommended)

```bash
curl -fsSL https://nexus.gatewarden.eu/install-link.sh | bash
```

The installer detects your platform (Linux aarch64 / x86\_64), downloads the
pre-built binary from GitHub Releases, and verifies its SHA-256 checksum.
Falls back to `cargo install --git` if no binary exists for your platform.

### From source

```bash
cargo install --git https://github.com/gwnexus/nexus-link.git nexus-link-cli
```

Requires Rust >= 1.85 (2024 edition).

### Pin a specific version

```bash
NEXUS_LINK_VERSION=v0.8.2 curl -fsSL https://nexus.gatewarden.eu/install-link.sh | bash
```

---

## Quick Start

```bash
# 1. Preflight check (optional — register runs this automatically)
nexus-link preflight

# 2. Register the node with both tokens from the Nexus dashboard
nexus-link register \
  --token     <nxs_node_*> \
  --cmd-token <nxs_cmd_*>

# 3. Start the telemetry agent
nexus-link agent start

# 4. Start the command service (receives compose management commands)
nexus-link-service

# 5. Check status
nexus-link status
```

Both tokens are generated and displayed **once** in the Nexus dashboard
registration wizard. Copy them before closing the dialog.

---

## Two-Token Security Model

nexus-link uses two independent credentials with different trust levels:

```
┌─────────────────────────────────────────────────────────────────┐
│                      Two-Token Architecture                     │
├───────────────────┬─────────────────────────────────────────────┤
│  Token            │  nxs_node_*  (Node Token)                   │
│  Used by          │  nexus-link-agent (telemetry daemon)        │
│  Operations       │  POST /telemetry, heartbeat                 │
│  Trust level      │  Read-only — cannot modify anything         │
│  Rotation         │  24-hour grace period                       │
├───────────────────┼─────────────────────────────────────────────┤
│  Token            │  nxs_cmd_*  (Command Token)                 │
│  Used by          │  nexus-link-service (compose management)    │
│  Operations       │  File read/write, docker compose up, logs   │
│  Trust level      │  Write-capable                              │
│  Rotation         │  Immediate — no grace period                │
│  Backend storage  │  AES-256-GCM encrypted — never in browser   │
└───────────────────┴─────────────────────────────────────────────┘
```

This separation ensures a compromised telemetry process cannot trigger service
restarts or modify compose configuration. Both tokens are generated together
during node registration and stored in `~/.nexus-link/config.toml`.

---

## Compose Management API

The `nexus-link-service` (port 8443) exposes a compose management API used by
the Nexus dashboard **Compose tab**:

| Endpoint                    | Method | Description                                              |
|-----------------------------|--------|----------------------------------------------------------|
| `/api/health`               | GET    | Health check — no auth required                          |
| `/api/compose/file`         | GET    | Read `docker-compose.yaml` + companion config files      |
| `/api/compose/file`         | PUT    | Write compose file (YAML validation, atomic write)       |
| `/api/compose/activate`     | POST   | Run `docker compose up -d` (120s timeout)                |
| `/api/compose/logs`         | GET    | Live log stream (SSE, `docker compose logs --follow`)    |
| `/api/commands`             | POST   | Legacy command endpoint (node token auth)                |

All `/api/compose/*` endpoints require `Authorization: Bearer <nxs_cmd_*>`.
The node token (`nxs_node_*`) is **not accepted** on compose routes.

### From the Nexus dashboard

The AI Hardware node detail page has a **Compose** tab where you can:

- **View and edit** `docker-compose.yaml` and companion files (`.env`, `.conf`, `.toml`)
- **Save & Commit** — atomically writes the file with YAML validation; commits to git if the compose directory is a repository
- **Activate Config** — runs `docker compose up -d` with stdout/stderr displayed in the UI
- **Stream Logs** — live `docker compose logs --follow` as Server-Sent Events

The dashboard proxy routes all compose API calls server-side — the command
token never reaches the browser.

---

## Ed25519 Request Signing (v2)

Write operations (`PUT /api/compose/file`, `POST /api/compose/activate`) support
Ed25519 request signing. The Nexus backend signs each write request with its
private key; nexus-link-service verifies the signature before processing.

**Canonical message format:**

```
METHOD\n
PATH\n
ISO8601_UTC_TIMESTAMP\n
NONCE_HEX8\n
SHA256_HEX(body_bytes)
```

**Headers sent by the backend:**

```
X-Nexus-Signature: <hex-encoded 64-byte Ed25519 signature>
X-Nexus-Timestamp: 2026-06-25T10:00:00Z
X-Nexus-Nonce:     a3f5c21b9e4d7f08
```

**Protection:**
- ±5-minute timestamp window prevents replay attacks
- SHA-256 body hash in the canonical message detects payload tampering
- The signing private key never leaves the Nexus backend

**Device setup:** The matching Ed25519 public key (32 bytes, base64url) is
delivered in the `RegisterResponse` at registration time and written to
`~/.nexus-link/signing_key.pub`. It is also stored in `config.toml` as fallback.

Signature enforcement is **opt-in** (default: off for backward compatibility):

```bash
nexus-link config set compose.require_signatures true
```

Read-only routes (GET) never require a signature.

---

## Configuration

After registration, the node config is stored at `~/.nexus-link/config.toml`:

```toml
[node]
node_id = "eb287dc7-f51f-47b8-a665-0241b1cc2010"
name    = "spark-ccd9"
token   = "nxs_node_..."        # telemetry credential

[api]
base_url         = "https://nexus.gatewarden.eu"
push_interval_secs = 10

[service]
listen_addr = "0.0.0.0"
port        = 8443

[compose]
dir                = "/opt/dgx-llm"   # compose root (docker-compose.yaml location)
extra_extensions   = [".env", ".conf", ".toml"]
cmd_token          = "nxs_cmd_..."    # compose management credential
signing_public_key = "<base64url>"    # Ed25519 verifying key (written at registration)
require_signatures = false            # set to true to enforce signed writes
```

### Config management

```bash
nexus-link config show                                    # display full config
nexus-link config set compose.dir /opt/my-stack          # change compose root
nexus-link config set compose.require_signatures true     # enable Ed25519
nexus-link config get node.node_id                        # read a single value
nexus-link config path                                    # show config file path
```

---

## CLI Reference

### Core commands

```
nexus-link register --token <nxs_node_*> --cmd-token <nxs_cmd_*>
  Register this node. Runs preflight check. Writes both tokens and signing key
  to config. --force skips non-compatible device warning.

nexus-link preflight
  Run device compatibility check only (no registration).

nexus-link status
  Show node registration and agent status.

nexus-link reset [--force]
  Hard-reset: stop and disable all nexus-link services, remove ~/.nexus-link/
  entirely. Use after deleting the node in the Nexus dashboard.
  Docker containers and compose files are not affected.

nexus-link unregister [--force]
  Send an offline heartbeat to the backend and remove local config.
```

### Token rotation

```
nexus-link refresh --token <nxs_node_*>
  Rotate the node token. Verifies the new token with a heartbeat before saving.
  Restarts the agent service automatically. 24-hour grace period on the backend.

nexus-link refresh-cmd --cmd-token <nxs_cmd_*>
  Apply a rotated command token from the Nexus dashboard.
  Takes effect immediately — no service restart required.
  (Rotation invalidates the old token instantly with no grace period.)
```

### Agent and config

```
nexus-link agent start          Start the telemetry agent daemon
nexus-link agent stop           Stop the running agent
nexus-link agent logs [-t N]    Show last N agent log lines (default: 50)

nexus-link config show          Display full configuration
nexus-link config set <k> <v>   Set a configuration value
nexus-link config get <k>       Get a configuration value
nexus-link config path          Print config file path

nexus-link upgrade [--force]    Upgrade nexus-link to the latest release
```

---

## Architecture

A hybrid data channel decouples telemetry from compose management:

```
┌────────────────────────────────────────────────────────────────────────┐
│  On-Premise Node (DGX Spark / A100 / H100)                             │
│                                                                        │
│  nexus-link-agent ──────────── nxs_node_* ────────────► Nexus API     │
│  (telemetry push, 10s)          POST /api/nodes/:id/telemetry          │
│                                                                        │
│  nexus-link-service ◄─── nxs_cmd_* + Ed25519 sig ─── Nexus Dashboard │
│  (axum HTTP :8443)              /api/compose/*                         │
└────────────────────────────────────────────────────────────────────────┘
```

### Components

```
nexus-link/
├── crates/
│   ├── nexus-link-cli/     # CLI binary: register, reset, refresh-cmd, config, …
│   ├── nexus-link-agent/   # Telemetry collector + push daemon
│   ├── nexus-link-service/ # Axum HTTP server — compose management (:8443)
│   └── nexus-link-core/    # Shared types, token auth, config, Ed25519
├── docker/
│   ├── Dockerfile.release  # Minimal runtime image (binary-only, no compiler)
│   ├── Dockerfile.native   # Native ARM64 full build (for local dev / Spark)
│   └── Dockerfile.service  # Cross-compiled aarch64 service image
└── .github/workflows/
    ├── ci.yml              # Clippy + fmt + tests (ubuntu + macos)
    └── release.yml         # Cross-compile → multi-arch container → GitHub Release
```

---

## Device Compatibility

Registration includes an automatic preflight check:

| Check        | What it verifies                      |
|--------------|---------------------------------------|
| Architecture | Linux aarch64 or x86\_64              |
| GPU          | NVIDIA GPU present (via `nvidia-smi`) |
| Docker       | Docker daemon accessible              |
| Network      | Nexus API reachable                   |
| Disk         | >= 10 GB available                    |
| Device Match | Known device in compatibility registry|

| Verdict             | Meaning                                                              |
|---------------------|----------------------------------------------------------------------|
| **COMPATIBLE**      | Device is in the tested registry. Registration proceeds.             |
| **NOT RECOMMENDED** | GPU detected but device is unknown. Use `--force` to proceed anyway. |
| **INCOMPATIBLE**    | No GPU detected. Registration blocked unless `--force` is passed.    |

### Known devices (fully tested)

| Device                  | Identifier  | Notes                                  |
|-------------------------|-------------|----------------------------------------|
| NVIDIA DGX Spark        | `gb10`      | GB10 Grace Blackwell, 128 GB, aarch64  |
| NVIDIA DGX Station A100 | `a100`      | 4× A100 80 GB, AMD EPYC, x86\_64      |
| NVIDIA DGX A100         | `dgx-a100`  | 8× A100 80 GB, AMD EPYC, x86\_64      |
| NVIDIA DGX H100         | `dgx-h100`  | 8× H100 80 GB, x86\_64                |

---

## CI/CD

- **CI** (`ci.yml`): Runs on every push/PR to `main`. Checks on
  `ubuntu-latest` and `macos-latest`: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo nextest run`.

- **Release** (`release.yml`): Triggered by `v*` tags. Jobs run in order:
  1. **Build binaries** (matrix: aarch64 + x86\_64) via `cross`, with Cargo
     registry cache
  2. **Container Image** (matrix: linux/arm64 + linux/amd64) — copies the
     pre-built binary into `debian:bookworm-slim` via `Dockerfile.release`
     (no compiler, no QEMU)
  3. **Publish Multi-Arch Manifest** — assembles the two platform images into
     a single manifest list via `docker buildx imagetools create`, tagged
     `<version>`, `<major.minor>`, and `latest`
  4. **GitHub Release** — attaches both tarballs; blocked until the manifest
     is published

### Container image

```bash
# Automatically selects the correct platform (arm64 or amd64)
docker pull ghcr.io/gwnexus/nexus-link/nexus-link-service:latest

# Pin to a specific version
docker pull ghcr.io/gwnexus/nexus-link/nexus-link-service:0.8.2
```

---

## Build

```bash
cargo build --release
```

### Cross-compile for ARM64 (DGX Spark)

```bash
cross build --release --target aarch64-unknown-linux-gnu
```

### Development quality gate

```bash
cargo fmt --all                          # format
cargo clippy --all -- -D warnings        # lint
cargo nextest run --all                  # test
make check                               # all three
```

Pre-commit hooks run `cargo fmt`, `cargo check`, and `cargo clippy` automatically
before each commit (installed via `pre-commit install --install-hooks`).

---

## Related

- [Gatewarden Nexus](https://nexus.gatewarden.eu) — Platform (Next.js / Supabase)
- [nexus-cli](https://github.com/gwnexus/nexus-cli) — Workspace CLI
- [nexus-mcp](https://github.com/gwnexus/nexus-mcp) — MCP server

---

## License

[Apache-2.0](LICENSE) — Copyright (c) 2026 RelicFrog Holding UG (haftungsbeschraenkt)
