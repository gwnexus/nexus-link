# Contributing to Nexus Link

Thank you for your interest in contributing to Nexus Link.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/<your-username>/nexus-link.git`
3. Create a branch: `git checkout -b feature/my-feature`

## Requirements

- Rust >= 1.85 (2024 edition, install via [rustup](https://rustup.rs))
- Docker (for container-related testing)
- `cargo fmt` and `cargo clippy` must pass without warnings

## Development Workflow

```bash
# Build
cargo build

# Run tests
cargo nextest run --all

# Format code
cargo fmt --all

# Lint
cargo clippy --workspace -- -D warnings

# Full pre-commit check
make check
```

## Project Structure

The project uses a Cargo workspace with four crates:

| Crate               | Purpose                                    |
| ------------------- | ------------------------------------------ |
| `nexus-link-core`   | Shared types, config, token auth           |
| `nexus-link-cli`    | CLI binary (`nexus-link` command)           |
| `nexus-link-agent`  | Telemetry push daemon                      |
| `nexus-link-service`| Axum HTTPS server (command receiver)       |

## Pull Request Guidelines

- Keep PRs focused on a single change
- Include tests for new functionality
- Ensure `cargo nextest run --all`, `cargo fmt --check`, and `cargo clippy` pass
- Write clear commit messages following [Conventional Commits](https://www.conventionalcommits.org/)
- Update `CHANGELOG.md` for user-facing changes

## Commit Message Format

```
<type>(<scope>): <description>

[optional body]
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `ci`

## Reporting Bugs

Open an issue on GitHub with:
- Target device OS and architecture
- Nexus Link version (`nexus-link --version`)
- Steps to reproduce
- Expected vs. actual behavior
- Relevant log output

## Security Issues

Please report security vulnerabilities responsibly. See [SECURITY.md](SECURITY.md)
for instructions.

## License

By contributing, you agree that your contributions will be licensed under
the [Apache License 2.0](LICENSE).
