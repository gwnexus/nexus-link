# Nexus Link

[![Release](https://github.com/gwnexus/nexus-link/actions/workflows/release.yml/badge.svg)](https://github.com/gwnexus/nexus-link/releases)
[![CI](https://github.com/gwnexus/nexus-link/actions/workflows/ci.yml/badge.svg)](https://github.com/gwnexus/nexus-link/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org)

Hardware telemetry agent for the [Gatewarden Nexus](https://nexus.gatewarden.eu)
platform. Connects on-premise AI hardware nodes to the Nexus backend with live
GPU/CPU/memory metrics and remote command execution -- without requiring inbound
network access into your infrastructure.

## Install

### One-liner (recommended)

```bash
curl -fsSL https://nexus.gatewarden.eu/install-link.sh | bash
```

The installer detects your platform (Linux aarch64/x86_64), downloads the
pre-built binary from GitHub Releases, and verifies its SHA-256 checksum.
Falls back to `cargo install --git` if no binary exists for your platform.

### From source

```bash
cargo install --git https://github.com/gwnexus/nexus-link.git nexus-link-cli
```

Requires Rust >= 1.85 (2024 edition).

### Pin a version

```bash
NEXUS_LINK_VERSION=v0.7.0 curl -fsSL https://nexus.gatewarden.eu/install-link.sh | bash
```

## Quick Start

```bash
# 1. Run preflight check (optional -- register does this automatically)
nexus-link preflight

# 2. Register this node with the Nexus platform
nexus-link register --token <nxs_node_*>

# 3. Start the telemetry agent (push metrics every 30s)
nexus-link agent start

# 4. Check status
nexus-link status
```

The node token (`nxs_node_*`) is generated in the Nexus dashboard when
registering a hardware node. It is stored locally in `~/.nexus-link/config.toml`
and never transmitted again after initial registration.

## Device Compatibility

Registration includes an automatic **preflight check** that validates device
compatibility before linking with the Nexus backend.

### Preflight Checks

| Check        | What it verifies                       |
|--------------|----------------------------------------|
| Architecture | Linux aarch64 or x86_64                |
| GPU          | NVIDIA GPU present (via `nvidia-smi`)  |
| Docker       | Docker daemon accessible               |
| Network      | Nexus API reachable                    |
| Disk         | >= 10 GB available                     |
| Device Match | Known device in compatibility registry |

### Compatibility Verdicts

| Verdict             | Meaning                                                             |
|---------------------|---------------------------------------------------------------------|
| **COMPATIBLE**      | Device is in the tested registry. Registration proceeds.            |
| **NOT RECOMMENDED** | GPU detected but device is not a known/tested model. Use `--force`. |
| **INCOMPATIBLE**    | No GPU detected. Registration blocked.                              |

### Known Devices (fully tested)

| Device                  | Identifier | Notes                                 |
|-------------------------|------------|---------------------------------------|
| NVIDIA DGX Spark        | `gb10`     | GB10 Grace Blackwell, 128 GB, aarch64 |
| NVIDIA DGX Station A100 | `a100`     | 4x A100 80GB, AMD EPYC, x86_64        |
| NVIDIA DGX A100         | `dgx-a100` | 8x A100 80GB, AMD EPYC, x86_64        |
| NVIDIA DGX H100         | `dgx-h100` | 8x H100 80GB, x86_64                  |

### Register Flags

```bash
nexus-link register --token <token>                  # Normal (preflight enforced)
nexus-link register --token <token> --force          # Force on non-recommended devices
nexus-link register --token <token> --skip-preflight # Skip all checks
```

## Architecture

A **hybrid data channel** decouples telemetry push from command inbound:

```
┌──────────────────────────────────────────────────────────────────┐
│  On-Premise Node (e.g. DGX Spark)                                │
│                                                                  │
│  nexus-link-agent ──────────────► Nexus API                      │
│  (telemetry push, 30s interval)    POST /api/nodes/:id/telemetry │
│                                                                  │
│  nexus-link-service ◄──────────── Nexus App                      │
│  (axum HTTPS :8443)               signed command payloads        │
└──────────────────────────────────────────────────────────────────┘
```

- **Telemetry Push** -- the agent streams GPU/CPU/memory metrics and Docker
  container health to the Nexus backend every 30 seconds
- **Command Inbound** -- the axum HTTPS service accepts signed command payloads
  (`compose_restart`, `compose_logs`, `config_exchange`) from the Nexus App

## Commands

```
nexus-link register --token <nxs_node_*>   Register this node with Nexus (runs preflight)
nexus-link preflight                        Run device compatibility check only
nexus-link status                           Show node registration and agent status
nexus-link agent start                      Start the telemetry agent daemon
nexus-link agent stop                       Stop the running agent
nexus-link agent logs [-t 50]               Show agent logs
```

### Supported Commands (via Service)

| Command           | Description                             |
|-------------------|-----------------------------------------|
| `compose_restart` | Restart a Docker Compose service        |
| `compose_logs`    | Fetch recent logs from a service        |
| `config_exchange` | Exchange configuration with the backend |

## Project Structure

```
nexus-link/
├── Cargo.toml              # Workspace root
├── install.sh              # Platform-aware binary installer
├── .github/workflows/
│   ├── ci.yml              # CI: clippy, fmt, test (ubuntu + macos)
│   └── release.yml         # Release: cross-compile, container, GH release
├── crates/
│   ├── nexus-link-cli/     # CLI binary (register, start, stop, status)
│   ├── nexus-link-agent/   # Telemetry collector + push daemon
│   ├── nexus-link-service/ # Axum HTTPS server (command receiver, :8443)
│   └── nexus-link-core/    # Shared types, token auth, config
├── docker/
│   ├── Dockerfile.native   # Native ARM64 build (for Spark or ARM CI)
│   └── Dockerfile.service  # Cross-compiled aarch64 service image
└── scripts/                # DevBox scripts and ops utilities
```

## Build

```bash
cargo build --release
```

### Cross-compile for ARM64 (DGX Spark)

```bash
cross build --release --target aarch64-unknown-linux-gnu
```

## Test

```bash
cargo nextest run --all
```

## Development

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --all -- -D warnings

# Full check (fmt + clippy + test)
make check
```

## Deployment

### Direct binary deployment

```bash
make deploy-cli      # Deploy CLI to Spark
make deploy-agent    # Deploy agent to Spark
make deploy-all      # Deploy CLI + agent
```

### Container deployment (service)

```bash
make docker-build    # Build aarch64 container image
```

The service runs as a Docker container on the target node via Docker Compose.

## CI/CD

- **CI** (`ci.yml`): Runs on every push/PR to `main`. Tests on ubuntu-latest
  and macos-latest, clippy with `-D warnings`, and rustfmt check.
- **Release** (`release.yml`): Triggered by `v*` tags. Cross-compiles for
  aarch64 and x86_64 Linux, builds a container image, and creates a GitHub
  Release with all assets attached.

## Configuration

After registration, the node config is stored at `~/.nexus-link/config.toml`:

```toml
[node]
id = "uuid"
name = "dgx-spark"
api_url = "https://nexus.gatewarden.eu"

[agent]
interval_secs = 30
push_endpoint = "/api/nodes/{id}/telemetry"

[service]
bind_addr = "0.0.0.0:8443"
tls_cert = "/etc/nexus-link/cert.pem"
tls_key = "/etc/nexus-link/key.pem"
```

## Related

- [Gatewarden Nexus](https://nexus.gatewarden.eu) -- Platform (Next.js/Supabase)
- [nexus-cli](https://github.com/gwnexus/nexus-cli) -- Workspace CLI (scaffolding, sync, preflight)
- [nexus-mcp](https://github.com/gwnexus/nexus-mcp) -- MCP server (agent coordination)

## License

[Apache-2.0](LICENSE) -- Copyright (c) 2026 RelicFrog Holding UG (haftungsbeschraenkt)
