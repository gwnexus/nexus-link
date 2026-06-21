# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
