# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
