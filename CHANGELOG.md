# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning

Since 0.21.0 the `MAJOR.MINOR` of nexus-link follows the Nexus Hub release line
it is built against (ADR-0006): nexus-link `0.21.x` targets Nexus Hub `0.21.x`.
`PATCH` numbers are independent of the hub. The jump from 0.11.0 to 0.21.0 is
this alignment only; versions 0.12.0 to 0.20.x do not exist. Releases up to
0.11.0 used independent numbering.

## [Unreleased]

### Security

- HTTP command path: the nonce is recorded only after the Ed25519 signature
  verified, as on the command queue (ADR-0007). Before, a request with a
  forged signature could burn a nonce for the genuine one.

### Fixed

- **Queued `get_logs_snapshot` failed on hardened hosts** with
  `open <compose dir>/.env: permission denied` (Dispatch 82170dd4).
  `docker compose logs` loads the whole project, including `.env` for
  interpolation, which the service user cannot read. The snapshot now finds
  the stack's containers by the compose label
  `com.docker.compose.project.working_dir` and reads them with `docker logs`;
  nothing in the compose directory is opened. Lines are merged across
  containers in timestamp order (`<service> | <timestamp> <line>`), the newest
  500 are kept, and the result gains a `containers` count.
- **Failed log snapshots were reported as `completed`** with the error as a
  log line. A failing docker call now finishes the command as `failed` with
  the message in `error`.
- Command output truncation could panic on multi-byte UTF-8 at the cut
  position; it now cuts at a character boundary.

## [0.21.2] - 2026-11-17

Signed command queue, step 1 of 2 (Dispatch 747d083f from the hub). Safe with
the current hub: nothing changes for nodes with `require_signatures = false`
unless the hub starts sending signed commands. Enforcing signatures on the
queue (`require_signatures = true`) requires a Nexus Hub that signs queued
commands; the default changes to `true` for fresh installs in the first
release after that hub ships.

### Security

- **Ed25519 verification on the command queue.** `put_file`, `activate` and
  all other queued commands were executed without any signature check;
  `require_signatures` only protected the local HTTP routes. Queued commands
  are now verified before execution (`nexus_link_core::signing`):
  - Message `nexus-cmd-v1`, node id, command id, type, timestamp, nonce and
    the SHA-256 of the `payload` string (the exact signed JSON arguments), so
    no cross-language JSON canonicalization is needed.
  - ±5-minute window; nonce cache shared with the HTTP path; the nonce is only
    recorded after the signature verified.
  - With `require_signatures = true` every command type needs a valid
    signature; rejected commands are not executed and reported as `failed`
    with an error starting with `signature:`.
  - Signed commands are always verified, also with enforcement off.

### Added

- **Heartbeat field `command_signatures`**: `enforced`, `off` or
  `key_missing`, so the hub can gate runtime mutation on enforcement.
- `ComposeCommandItem` fields `payload`, `signature`, `timestamp`, `nonce`
  (optional, older hubs keep working).
- 26 tests: 20 for verification rules in `tests/src/signing_tests.rs`, 6
  end-to-end in the poller (unsigned or tampered `put_file` not written,
  signed `put_file` written once, replay rejected).

### Changed

- Verifying key loading moved from the service into
  `nexus_link_core::signing::load_verifying_key`, shared with the agent.
- Poller executes through `CommandContext` instead of `AppState`.

### Added (CI and release tooling)

- **Release dry run.** `release-dryrun.yml` runs the real release build
  without publishing (no image push, no GitHub Release, no tag), on changes
  to release workflows, `docker/`, `rust-toolchain.toml`, `Cargo.toml`,
  `Cargo.lock` or `CHANGELOG.md`, and on demand via
  `scripts/ci/release-dryrun.sh`.
- **Release checks.** Tag must equal the `Cargo.toml` version and have a
  CHANGELOG section; tarballs are checked for all three binaries and the CLI
  `--version`.
- **Dependabot** for GitHub Actions, Cargo and the Rust toolchain (weekly,
  grouped).
- actionlint and zizmor configuration in `.github/`.

### Changed (CI)

- **Rust toolchain pinned** to 1.100.0 in `rust-toolchain.toml`; CI, release
  and local builds use exactly this version (replaces
  `dtolnay/rust-toolchain@stable`).
- **CI hardening:** all actions pinned to commit SHAs and moved to their
  Node.js 24 releases (checkout v7, upload-artifact v7, download-artifact v8,
  docker/login-action v4, rust-cache v2.9.3, Blacksmith actions);
  least-privilege `permissions:` in every workflow; `persist-credentials:
  false` on checkouts; `cross` pinned to a commit; workflow inputs passed via
  `env` instead of inline expressions.
- Release build logic moved into the reusable `release-build.yml`, shared by
  release and dry run. `useblacksmith/cache` (still Node.js 20) replaced by
  `Swatinem/rust-cache`; `softprops/action-gh-release` replaced by
  `gh release create`.

## [0.21.1] - 2026-11-17

### Added

- **Backing model per heartbeat model entry (`backing_model`).** Object
  entries in the heartbeat `models` field may now carry the physical model
  behind an alias, so the hub can group LiteLLM aliases that share one runtime
  (Dispatch 2ac05e75). Sources:
  - LiteLLM: `GET <models_url>/model/info`, `litellm_params.model` of each
    alias with the provider prefix (`hosted_vllm/`, `openai/`, ...) stripped.
    Only queried when `/v1/models` carries no `root`. If an alias has several
    deployments with different models, the field is omitted.
  - vLLM: `root` from `/v1/models`; absolute local paths are not reported.
  - Same id rules as `id`; omitted when unknown. `api_base`, keys and all other
    `litellm_params` are never read into the payload.
  - A missing or rejected `/model/info` never drops the inventory; the models
    are reported without `backing_model` (rejection is logged as a warning).
- `nexus_link_core::heartbeat`: `ModelEntry::new`, `model_info_endpoint`,
  `parse_model_info`, `strip_provider_prefix`, `apply_backing_models`, with 10
  new tests.

### Changed

- `ModelEntry::Detailed` now has optional `max_model_len` and `backing_model`
  fields (both skipped when `None`); use `ModelEntry::new` to build entries.

## [0.21.0] - 2026-11-16

Requires Nexus Hub >= 0.21.0 for the model inventory; older hubs ignore the new
`models` field.

### Added

- **Periodic online heartbeat.** `nexus-link-agent` now sends
  `POST /api/nodes/{id}/heartbeat` with `status: online` every
  `agent.heartbeat_sec` seconds (default 60), independent of the telemetry
  push. The dashboard marks nodes offline after five minutes without a
  heartbeat; until now only `nexus-link refresh` sent one.
- **Served-model inventory in the heartbeat (`models`).** When
  `agent.models_url` points to the node's local OpenAI-compatible runtime
  (LiteLLM or vLLM), the agent reads `GET /v1/models` on the node and reports
  each model as an id string or `{ id, max_model_len }`. At most 50 entries;
  invalid and duplicate ids are dropped. If the runtime cannot be queried the
  field is omitted, so the hub keeps the last known inventory. The response's
  `models_recorded` count is logged. Requested by the hub for BYOHW gateway
  suggestions (Dispatch 06d00498).
- **Config keys** `agent.heartbeat_sec` (1 to 120), `agent.models_url` and
  `agent.models_api_key` (optional local runtime key, never sent to the
  backend) in `config show/set/get`.
- `nexus_link_core::heartbeat`: heartbeat payload, model list parser and id
  validation, with 13 new tests plus config default and round-trip tests.

### Changed

- **Version alignment with Nexus Hub (ADR-0006).** Version jumps from 0.11.0
  to 0.21.0; see [Versioning](#versioning).
- **Security tooling:** gitleaks secret scan added to the pre-commit framework;
  `devbox` init verifies that gitleaks and the pre-commit hooks are installed.
- README: versioning and hub compatibility table, `[agent]` config section
  (replaces the outdated `api.push_interval_secs` example), heartbeat and model
  inventory documentation.
- CHANGELOG: restored the missing 0.8.3 and 0.8.4 headings (their entries had
  been merged into 0.8.5) and added the missing 0.8.9 entry.

## [0.11.0] - 2026-09-01

### Added

- **Multi-profile support (ADR-0005).** A single physical device can now register
  against multiple independent Nexus App backends simultaneously (e.g. production
  + staging). Each profile runs as an isolated systemd instance with its own
  config, tokens, node identity, and state directory.

- **`--profile <name>` flag** on all CLI commands, `nexus-link-agent`, and
  `nexus-link-service`. Sets `NEXUS_LINK_PROFILE` env var before config loading.
  Omitting `--profile` uses the `default` profile (backward compatible).

- **Profile-aware config resolution.** Config files follow the pattern
  `config.<profile>.toml` (e.g. `config.staging.toml`). The `default` profile
  resolves to `config.toml` for backward compatibility. Full resolution order:
  1. `NEXUS_LINK_CONFIG` env var (absolute precedence)
  2. `/etc/nexus-link/config.<profile>.toml`
  3. Legacy paths (default profile only)
  4. `~/.nexus-link/config.toml` (default profile only)

- **Profile-aware systemd service names.** Non-default profiles generate
  template-style service names: `nexus-link-agent@staging.service`. The default
  profile keeps the original names for backward compatibility.

- **Profile-aware state directories.** Non-default profiles use
  `/var/lib/nexus-link/<profile>/` for signing keys and transient state.

- **10 new tests** for profile config resolution, service naming, state directory
  isolation, and env var precedence. Total: 68 tests.

### Usage

```bash
# Register against staging backend
nexus-link --profile staging register \
  --api-url https://staging.nexus.gatewarden.eu \
  --token <nxs_node_*> --cmd-token <nxs_cmd_*>

# Start staging agent and service
nexus-link --profile staging agent start

# View staging logs
nexus-link --profile staging agent logs

# Production continues to work without --profile
nexus-link agent start
```

## [0.10.6] - 2026-09-01

### Security

- **SEC-504 (MEDIUM): Migrated config file permissions hardened from 644 to 600.**
  `preflight --setup` migration set `/etc/nexus-link/config.toml` to world-readable
  (mode 644), exposing bearer tokens and database credentials to all local users.
  Now uses mode 600 (owner-only read/write).
- **SEC-405 (HIGH): Filesystem paths no longer leaked to remote backend.**
  The `get_file` and `put_file` poller results included full absolute paths
  (e.g. `/opt/dgx-llm/docker-compose.yaml`) in the JSON payload sent to the
  Nexus API. Error messages also exposed the compose directory. Now returns
  only filenames.
- **SEC-502 (LOW): Doc comment example corrected.** The `Config` struct doc
  comment showed `listen_addr = "0.0.0.0"` as example, potentially misleading
  operators into insecure copy-paste configuration. Changed to `127.0.0.1`.

### Fixed

- **Dependency vulnerabilities resolved.** Updated `crossbeam-epoch` (RUSTSEC-2026-0204,
  invalid pointer dereference), `anyhow` (RUSTSEC-2026-0190, unsound `downcast_mut`),
  and `chacha20` (yanked version). All advisories cleared via `cargo audit`.

## [0.10.5] - 2026-09-01

### Fixed

- **Config resolution: stale legacy config after migration.** The `preflight
  --setup` migration step copied configs from `/var/lib/nexus-link/config.toml`
  to `/etc/nexus-link/config.toml` but never deleted the source file. On
  re-registration (hostname rename, token rotation), the stale legacy config
  persisted and could shadow the new config during resolution. The migration
  now removes the source file after a successful copy.
- **`unregister` targeted wrong config path.** The unregister command used
  `default_config_path()` (pointing to `~/.nexus-link/`) instead of
  `effective_config_path()`, so it never cleaned up the config actually in use
  when running in system mode (`/etc/nexus-link/`).

### Added

- **Config path logged at startup.** `Config::load()` now emits an info-level
  log line showing the resolved config file path (e.g.
  `Config loaded from /etc/nexus-link/config.toml`). Visible in
  `journalctl -u nexus-link-agent` for immediate debugging.
- **`--config <path>` flag for agent and service binaries.**
  `nexus-link-agent --config /path/to/config.toml` and
  `nexus-link-service --config /path/to/config.toml` now set the
  `NEXUS_LINK_CONFIG` env var before loading, matching the existing
  CLI `--config` flag behavior.
- **4 new tests:** Config path env var override with nonexistent paths, empty
  env var edge case, fallback resolution without env var, and
  `Config::load()` integration test via env var override.

## [0.10.4] - 2026-08-10

### Fixed

- **Compose logs timeout:** `docker compose logs --tail N` applies per-container,
  resulting in N × num_containers lines and slow execution when no service is
  specified. Now adds `--since 5m` to bound the time range and reduce output.
- **Output cap:** Log snapshot results are capped at 500 lines max to prevent
  oversized payloads that approach the 30s frontend timeout.
- **Default tail reduced:** Changed from 200 to 100 lines per container.
- **Improved debug logging:** PG LISTEN errors now use Debug formatting to show
  full error chain instead of generic "db error".

## [0.10.3] - 2026-08-10

### Fixed

- **`preflight --setup` binary install after upgrade:** When the running binary
  is already in `/usr/local/bin/` (from a previous setup), the install step now
  searches `~/.local/bin/` (where `nexus-link upgrade` places new versions) as
  an alternative source. Prevents self-referential copy that deleted the binary
  then failed to find the source.
- **Same-file copy guard:** Skip binary copy if source and target resolve to the
  same inode (already up to date).

## [0.10.2] - 2026-08-10

### Changed

- **Config path moved to `/etc/nexus-link/config.toml`:** FHS-compliant location
  for service configuration. Root-owned (mode 644), readable by the `nexus-link`
  service user. Operators edit with `sudo`.
- **Runtime state directory stays at `/var/lib/nexus-link/`:** Signing keys and
  transient state remain service-owned (mode 700). Config no longer stored here.
- **Config resolution order updated:** env var → `/etc/nexus-link/config.toml` →
  `/var/lib/nexus-link/config.toml` (legacy) → `~/.nexus-link/config.toml`.
- **`preflight --setup` creates both directories:** `/etc/nexus-link/` (root:root,
  755) for config and `/var/lib/nexus-link/` (nexus-link:nexus-link, 700) for state.
- **Config migration priority:** Migrates from `/var/lib/nexus-link/config.toml`
  (v0.10.0/v0.10.1) first, then `~/.nexus-link/config.toml` (original legacy).
- **Systemd units use `/etc/nexus-link/config.toml`** via `NEXUS_LINK_CONFIG` env.
- **`reset` command cleans all paths:** `/etc/nexus-link/`, `/var/lib/nexus-link/`,
  `~/.nexus-link/`, binaries, unit files, and system user.

### Fixed

- **PG LISTEN/NOTIFY TLS handshake failure:** Supabase Pooler uses a private CA
  (Supabase Root 2021 CA) not present in public root stores. The TLS connector
  now accepts any server certificate (equivalent to PostgreSQL `sslmode=require` —
  encrypted transport without server identity verification).

### Migration

For nodes already running v0.10.0 or v0.10.1:
```bash
nexus-link upgrade
sudo nexus-link preflight --setup
```
This migrates the config from `/var/lib/nexus-link/` to `/etc/nexus-link/`
automatically and restarts services.

## [0.10.1] - 2026-08-10

### Added

- **`preflight --setup` full provisioning:** Installs binaries to `/usr/local/bin/`
  (replaces symlinks to user-local paths), writes hardened systemd unit files,
  runs `daemon-reload`, and enables+starts both services. Single command setup.
- **Comprehensive `reset` command:** Full teardown — stops and disables services,
  removes systemd unit files, deletes `/var/lib/nexus-link/` and `~/.nexus-link/`,
  uninstalls binaries from `/usr/local/bin/`, removes the `nexus-link` system user.
  Clean slate for re-registration.
- **`setup_tests.rs`:** Unit tests for setup constants and struct validation.

### Fixed

- **Binary permission denied:** `preflight --setup` now copies binaries to
  `/usr/local/bin/` instead of relying on symlinks into user home directories
  (which the `nexus-link` system user cannot access).

## [0.10.0] - 2026-08-09

### Added

- **Dedicated system user `nexus-link`:** Services now run as a minimal-privilege
  system user with no login shell, no home directory, and `docker` group
  membership only. Config stored at `/var/lib/nexus-link/config.toml` (mode 600).
- **`preflight --setup` command:** Full automated provisioning: creates the system
  user, config directory, migrates legacy configs, installs binaries to
  `/usr/local/bin/` (replacing symlinks), writes systemd unit files, runs
  `daemon-reload`, and enables+starts both services. Single command, idempotent.
- **Systemd unit installation:** `preflight --setup` writes hardened unit files
  for `nexus-link-agent.service` and `nexus-link-service.service` with
  `ProtectHome`, `ProtectSystem=strict`, `RestrictSUIDSGID`, `NoNewPrivileges`.
- **Automatic setup during `register`:** The registration flow now runs system
  setup (user creation, directory provisioning) before saving the config. All
  privileged operations use `sudo` internally — no need to run as root.
- **`NEXUS_LINK_CONFIG` env var:** Override the config path for any nexus-link
  binary. Priority: env var → `/var/lib/nexus-link/config.toml` → `~/.nexus-link/config.toml`.
- **Global `--config` flag:** All CLI commands respect `--config <path>` to
  specify an explicit config location.
- **Enhanced `reset` command:** Now performs full teardown — stops services,
  removes systemd unit files, deletes both legacy (`~/.nexus-link/`) and system
  (`/var/lib/nexus-link/`) directories, uninstalls binaries from `/usr/local/bin/`,
  and removes the `nexus-link` system user. Clean slate for re-registration.

### Changed

- **Config resolution order:** `Config::load()` now checks env var, system path,
  then user path (previously only checked `~/.nexus-link/config.toml`).
- **Systemd units use `User=nexus-link`** when the system config directory
  exists, instead of running as root or the invoking user.
- **Signing key storage:** Written alongside config (system path preferred).

### Migration

For existing nodes, run:
```bash
sudo nexus-link preflight --setup
nexus-link agent restart
```
This creates the `nexus-link` user, migrates the config, and regenerates
systemd units. No re-registration required.

## [0.9.1] - 2026-08-09

### Fixed

- **Runtime panic on aarch64:** Rustls CryptoProvider was not installed before
  PG TLS handshake. Now explicitly installs the `ring` provider at pg_listener
  startup. Affects only devices with `database_url` configured.

### Changed

- **Docs:** Updated README with PG LISTEN/NOTIFY architecture diagram, command
  queue latency table, and PG listener credential security documentation.
  Added `database_url` to config example.

## [0.9.0] - 2026-08-08

### Added

- **PG LISTEN/NOTIFY wake-up channel (ADR-0071):** New `pg_listener` module in
  `nexus-link-service` maintains a persistent PostgreSQL connection subscribed to
  channel `node_cmd_<nodeId>`. On notification the command poller runs immediately
  instead of waiting for the next interval tick. Includes automatic reconnection
  with exponential backoff (1 s → 30 s max). Fully opt-in — requires
  `database_url` in `[api]` config section.
- **Adaptive polling via X-Poll-Interval header:** The command poller now reads
  the `X-Poll-Interval` response header from the `/pending` endpoint and adjusts
  its fallback HTTP polling interval dynamically (30 s idle, 2 s active).
- **TLS support for PG connections:** Uses `tokio-postgres-rustls` with
  `webpki-roots` for secure connections to Supabase session-mode pooler
  (`sslmode=require`).
- **Config: `[api].database_url`** — optional PostgreSQL connection string for
  the LISTEN/NOTIFY channel. When absent, behavior is unchanged (HTTP-only).
- **Register response: `database_url` field** — the CLI now writes the
  `database_url` returned by `POST /api/nodes/register` into the local config.

### Changed

- Command poll loop refactored from fixed `tokio::time::interval` to
  `tokio::select!` over sleep + wake signal, enabling instant response to
  PG NOTIFY events.

## [0.8.9] - 2026-07-29

### Fixed

- **Security:** findings SEC-001 to SEC-009 from an OWASP-based audit:
  - 0600/0700 permissions on `config.toml` and its directory.
  - Startup warning when listening on a non-localhost address without TLS.
  - Constant-time token hash comparison (`subtle::ConstantTimeEq`).
  - Token hashes pre-computed at startup; raw tokens no longer kept in the
    service state.
  - Nonce replay cache rejects duplicate signed requests.
  - Startup warning when `require_signatures = false` with the command
    channel active.
  - 30 s timeout on the telemetry push HTTP client.
  - `docker compose` output truncated to 8 KiB before it is sent to the backend.
  - Git commit messages sanitized (200 characters, control characters stripped).

## [0.8.8] - 2026-06-30

### Added

- **Command queue:** `validate_yaml` command type — dry-run YAML validation
  using the same `serde_yaml` parser as `put_file`. Returns `valid: true/false`
  without writing any file. Requested by nexus-app for Compose editor UX.

### Fixed

- **Security (HIGH):** Added 2 MiB `DefaultBodyLimit` layer to the axum router
  to prevent memory exhaustion via oversized request payloads.
- **Security (HIGH):** Default `listen_addr` changed from `0.0.0.0` to
  `127.0.0.1`. Existing configs with explicit `listen_addr = "0.0.0.0"` are
  unaffected. New deployments require explicit opt-in for public binding.
- **Security (MEDIUM):** Removed filesystem paths from client-facing error
  responses in compose handlers. Full errors are still logged server-side.
- **Security (MEDIUM):** Clamped `tail` query parameter in the SSE log stream
  to a maximum of 10,000 lines to prevent OOM from unbounded log reads.

### Changed

- **CI:** Re-enabled integration test workflow on push/PR to `main` (was
  disabled during v0.8.0 E2E verification cycle).

## [0.8.7] - 2026-06-27

### Changed

- CLI header updated to match Nexus CLI style:
  `Nexus Link v<version> — Nexus hardware node agent`
  `(C) 2025-present RelicFrog Holding UG | Nexus Product Group`

## [0.8.6] - 2026-06-26

### Fixed

- **Security (MEDIUM):** Added service name validation to `stream_compose_logs`
  SSE handler — rejects names starting with `-` or containing special characters
  (`/`, `;`, `&`, `|`, ` `). Mirrors existing validation in `poller.rs`.
- **Security (MEDIUM):** Removed misleading "constant-time" claim from
  `verify_token` doc comment in `token.rs`. The actual security property
  (SHA-256 pre-image resistance) is now documented correctly.
- **Security (LOW):** Removed internal filesystem path from 404 error
  response in `get_compose_file` handler — now returns a generic message.

## [0.8.5] - 2026-06-25

### Added
- `nexus-link agent restart` — restarts both `nexus-link-agent` and
  `nexus-link-service` via `systemctl restart`. Falls back to `start`
  if the service is not yet registered as a systemd unit.
  Respects user vs. system systemd mode (same detection logic as
  `agent start` / `agent stop`).

## [0.8.4] - 2026-06-25

### Added

- **Command queue poll loop (ADR-0049 reverse agent):** `nexus-link-service`
  polls the backend for pending commands and executes them locally
  (`get_file`, `put_file`, `activate`, `logs_snapshot`), so the backend never
  needs inbound access to the node.

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

## [0.8.3] - 2026-06-25

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
