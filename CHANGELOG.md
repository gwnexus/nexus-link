# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.4] - 2026-06-25

### Changed

- **Config restructure: `[api.tokens]` + `[agent]`** — the two token
  credentials and the two timing intervals now live in dedicated sections:

  ```toml
  [api]
  base_url = "https://nexus.gatewarden.eu"

  [api.tokens]
  telemetry = { token = "nxs_node_...", scope = "read" }
  command   = { token = "nxs_cmd_...",  scope = "read_write" }

  [agent]
  push_sec = 6    # telemetry push interval (nexus-link-agent)
  poll_sec = 2    # command queue poll interval (nexus-link-service)
  ```

  **Backward compatibility:** old `config.toml` files are migrated
  transparently on first load — `api.push_interval_secs` → `agent.push_sec`,
  `node.token` → `api.tokens.telemetry`, `compose.cmd_token` →
  `api.tokens.command`. No manual action required on existing nodes.

- Default telemetry push interval corrected: **6 seconds** (was hardcoded
  to 10 at registration time despite the Serde default being 6).
- Default command poll interval: **2 seconds** (unchanged).
- `nexus-link config show` now displays `[api.tokens]` and `[agent]` sections.
- `nexus-link config set/get` supports new keys:
  `agent.push_sec`, `agent.poll_sec`, `api.tokens.telemetry`,
  `api.tokens.command`. Old aliases (`push_interval`, `interval`) still work.
- `nexus-link status` shows `<push>s push / <poll>s poll` instead of a
  single interval value.
- `nexus-link refresh --token` keeps `api.tokens.telemetry.token` in sync
  with `node.token` on rotation.
- New convenience methods on `Config`: `node_token()` and `cmd_token()` —
  resolve the effective token from the new location with fallback to the
  legacy fields.



### Fixed
- `nexus-link config set compose.signing_public_key` was rejected with
  "Unknown config key" — the key was missing from the `set` and `get`
  dispatch tables in `config.rs`.
  Now writes `compose.signing_public_key` to `config.toml` **and**
  simultaneously writes `~/.nexus-link/signing_key.pub` so
  `nexus-link-service` picks up the key at its next start without
  requiring a re-registration.

### Added
- `nexus-link config set compose.signing_public_key <base64url>` — sets
  the Ed25519 verifying key used for signed compose commands (ADR-0051 v2).
  Passing an empty string clears the key and removes `signing_key.pub`.
- `nexus-link config get compose.signing_public_key` — reads the configured
  verifying key.

## [0.8.2] - 2026-06-25

### Added
- Multi-architecture container image (`linux/arm64` + `linux/amd64`).
  Both platforms are built in the release pipeline and assembled into a
  single multi-arch manifest via `docker buildx imagetools create`.
  `docker pull ghcr.io/gwnexus/nexus-link/nexus-link-service` now
  automatically selects the correct platform on ARM and x86\_64 nodes.
- New `docker/Dockerfile.release`: minimal runtime image that copies the
  pre-built cross-compiled binary into `debian:bookworm-slim`. No compiler,
  no QEMU emulation — container build time reduced from ~12 min to ~60-90s.
- Cargo registry cache (`useblacksmith/cache@v5`) in the release pipeline
  saves 60-90s on warm runs by skipping dependency recompilation.
- Manifest tags `<version>`, `<major.minor>`, and `latest` published after
  every release.

### Fixed
- Release job now correctly waits for the multi-arch manifest before
  creating the GitHub Release (`needs: [build-binaries, publish-manifest]`).
  Previously the container build was not a release gate.
- Contact emails updated to `post+*@gatewarden.eu` across `Cargo.toml`,
  `AUTHORS.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, and GitHub templates.

### Changed
- Release pipeline restructured into four sequential jobs:
  `build-binaries` → `build-container` (matrix) → `publish-manifest` → `release`.
- `pre-commit` hooks now active: `cargo fmt`, `cargo check`, `cargo clippy`,
  plus file-hygiene hooks (trailing whitespace, EOF, YAML/TOML syntax).
- `rustfmt.toml` added (`edition = "2024"`) to prevent formatter divergence
  between local and CI toolchain versions.

## [0.8.1] - 2026-06-25

### Added
- `nexus-link reset [--force]`: hard-reset command for post-dashboard-delete
  cleanup. Stops and disables `nexus-link-agent` and `nexus-link-service`
  via systemd (user mode → system mode → pkill fallback), then removes the
  entire `~/.nexus-link/` directory. Docker containers and compose files are
  not affected. Interactive confirmation unless `--force` is passed.

### Changed
- Integration test workflow temporarily set to `workflow_dispatch` only
  (re-enabled after DGX Spark E2E test — tracked in Nexus task
  `90d82c80-838f-43aa-8279-2bc43c075864`).

## [0.8.0] - 2026-06-25

### Added
- **Compose Management API** (`nexus-link-service`, port 8443):
  - `GET /api/compose/file` — reads `docker-compose.yaml` and companion
    config files (`.env`, `.conf`, `.toml`) from the configured compose root
    (default `/opt/dgx-llm`).
  - `PUT /api/compose/file` — writes compose file with YAML validation and
    atomic write (tmp → rename). Commits to git if the directory is a repo.
  - `POST /api/compose/activate` — runs `docker compose up -d`
    (120-second timeout, returns stdout + stderr).
  - `GET /api/compose/logs` — SSE live log stream via
    `docker compose logs --follow`.
- **Two-token security model** (ADR-0051 v1 + v2):
  - Dedicated `nxs_cmd_*` command token for all `/api/compose/*` routes,
    separate from the telemetry `nxs_node_*` token.
  - New `require_cmd_auth` middleware: `403 Forbidden` when `cmd_token`
    is not configured; `401 Unauthorized` on wrong token.
  - `cmd_auth` middleware stores the token hash on every request (no
    in-memory cache) so `nexus-link config set compose.cmd_token` takes
    effect immediately without a service restart.
  - Ed25519 signed command verification for write operations when
    `compose.require_signatures = true` (opt-in, default false).
    Canonical message: `METHOD\nPATH\nTIMESTAMP\nNONCE\nSHA256(body)`.
    ±5-minute replay window. Body is buffered and re-injected after
    verification.
  - `AppState.signing_pubkey` loaded from `~/.nexus-link/signing_key.pub`
    (primary) or `config.compose.signing_public_key` (fallback) at startup.
  - `ed25519-dalek v2` + `hex` added to workspace dependencies.
- `nexus-link register --cmd-token <nxs_cmd_*>`: registers both tokens in a
  single step. Writes `signing_public_key` from the register response to
  `config.toml` and `~/.nexus-link/signing_key.pub`.
- `nexus-link refresh-cmd --cmd-token <nxs_cmd_*>`: applies a rotated
  command token. Takes effect immediately — no service restart required.
  (Backend invalidates old token atomically, no grace period.)
- `nexus-link config set/get` support for `compose.cmd_token` and
  `compose.require_signatures`.
- `ComposeConfig` extended with `cmd_token`, `signing_public_key`, and
  `require_signatures` fields in `nexus-link-core`.

### Fixed
- `require_auth` middleware now performs real SHA-256 hash comparison against
  the stored node token hash. Previously any well-formed `nxs_node_*` token
  was accepted without verification.
- `AppState` no longer suppresses `dead_code` warnings on `config` and
  `docker` fields — both are now actively used.

### Changed
- Router split into `public_routes()` (health, no auth) and
  `protected_routes()` (commands, node auth) + `compose_routes()` (cmd auth).
  Auth middleware applied via `from_fn_with_state` in `main.rs` with full
  access to `SharedState` for token verification.
- `ServiceConfig.compose_root` superseded by `ComposeConfig.dir` (existing
  deployments unaffected — field is `#[serde(default)]`).

## [0.7.5] - 2026-06-21

### Fixed
- Private IP detection now uses `ip route get 1.1.1.1` with `hostname -I`
  fallback. Reports detected endpoint during registration.
- DGX Spark unified memory workaround: when `nvidia-smi` reports `[N/A]`
  for `memory.total`/`memory.used` (Grace Blackwell unified architecture),
  falls back to `/proc/meminfo` for total and per-process VRAM sum for used.
  Hardcoded fallback: 128 GB total for DGX Spark.
- Telemetry push logging: structured `info!` log after each successful push
  with CPU%, memory GB, container count, GPU summary, and private IP.
- Default push interval changed from 30s to 10s.

## [0.7.4] - 2026-06-21

### Added
- `nexus-link config show|set|get|path` commands for runtime configuration
  management (api_url, push_interval, listen_addr, port, name, tags)
- Installer creates symlinks in `/usr/local/bin` for system-wide access
  (attempts direct write, falls back to sudo)

### Fixed
- **Critical:** systemd system units now include `User=` and `Environment=HOME=`
  directives when installed by a non-root user. Previously, the agent ran as root
  and looked for config in `/root/.nexus-link/` instead of the registering user's
  home directory.
- Binary search now prioritizes `~/.local/bin` (where the installer writes)

### Changed
- 49 tests passing

## [0.7.3] - 2026-06-21

### Added
- `nexus-link refresh --token <nxs_node_*>`: token rotation with rollback safety.
  Validates new token, sends heartbeat to verify acceptance, reverts to old token
  on failure. Restarts agent after successful rotation.
- Non-root systemd support: `nexus-link agent start` detects whether the user
  is root (or has sudo) and chooses between system-wide units
  (`/etc/systemd/system/`) and per-user units (`~/.config/systemd/user/`).
  Warns about `loginctl enable-linger` for user services.
- Sudo capability check: `sudo -n true` probe before attempting privileged ops.

### Changed
- Installer writes to `~/.local/bin` by default (no root required on target nodes)
- Agent `start/stop/logs` commands respect systemd user mode for non-root users
- `which_binary` now prioritizes `~/.local/bin` over `/usr/local/bin`
- 49 tests passing

## [0.7.2] - 2026-06-21

### Added
- GPU telemetry collection: parses nvidia-smi CSV output for temperature,
  utilization, memory usage, and power draw per device
- Per-container stats: CPU percentage, memory usage/limit, network RX/TX
  via bollard stats API (one-shot, non-streaming)
- Systemd integration: `nexus-link agent start` generates security-hardened
  unit files, enables and starts both agent and service via systemctl
- `nexus-link agent stop`: systemctl stop with pkill fallback
- `nexus-link agent logs`: journalctl output with unit filter
- `nexus-link unregister [--force]`: device-side cleanup (offline heartbeat,
  agent stop, config removal)
- Integration test scenario for unregister command

### Changed
- 49 tests passing (5 new GPU parser tests, 1 new integration scenario)
- Container metrics no longer return zeros -- real stats from Docker API

## [0.7.1] - 2026-06-21

### Added
- `nexus-link upgrade` command: checks GitHub Releases for newer versions,
  downloads the matching platform binary, verifies integrity, and replaces
  the current binary in-place (supports `--force` for re-download)
- Dedicated test crate (`tests/`) with 44 unit tests covering token, config,
  types, telemetry serialization, error handling, and preflight checks
- Integration test workflow (`.github/workflows/integration.yml`) with
  scenario-based testing: CLI help, preflight, register validation, service health
- GitHub issue templates (bug report, feature request, security)
- Pull request template with device testing checklist
- CODE_OF_CONDUCT.md (Contributor Covenant 2.1)
- Pre-commit hook (`hooks/pre-commit`) for fmt + clippy
- `.cargo/config.toml` for macOS linker compatibility

### Fixed
- Release pipeline: switched from `native-tls` (openssl-sys) to `rustls-tls`
  to eliminate cross-compilation dependency on system OpenSSL headers
- Docker images: use `rust:latest` base instead of pinned 1.87
  (ensures edition 2024 support)
- Docker runtime: removed unnecessary `libssl3` from runtime images
  (rustls is statically linked)

### Changed
- 44 tests passing
- Version aligned with nexus-cli v0.7.x

## [0.7.0] - 2026-06-21

### Added
- Initial release of nexus-link hardware telemetry agent
- `nexus-link register` command with device preflight checks (GPU detection,
  Docker availability, architecture validation, known device matching)
- `nexus-link status` command for node registration and agent health
- `nexus-link agent start|stop|logs` for daemon management
- `nexus-link-agent` telemetry push daemon (GPU/CPU/memory/containers, 30s interval)
- `nexus-link-service` axum HTTPS server for command reception (:8443)
- Supported commands: `compose_restart`, `compose_logs`, `config_exchange`
- Node token authentication (`nxs_node_*`)
- Docker container metrics via bollard
- GPU metrics via nvidia-smi
- Cross-compilation support for `aarch64-unknown-linux-gnu`
- Docker images (native ARM64 + cross-compiled)
- CI/CD pipeline (GitHub Actions: CI + Release)
- One-liner installer script with SHA-256 verification
- Device compatibility matrix (DGX Spark fully supported)
