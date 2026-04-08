# CLAUDE.md — aihelp

Rust CLI for LM Studio. Cargo-based single binary.

## Quick Reference
```bash
cargo build --release
cargo test
cargo fmt --all --check
cargo clippy -- -D warnings
cargo install --path .
aihelp --setup                  # interactive config wizard
aihelp --list-models
aihelp "your question"
cat file.sh | aihelp "explain this"
aihelp --mcp "query with MCP tools"
```

Config: `~/.config/aihelp/config.toml`. Default endpoint: `http://192.168.50.2:1234`.

MCP policy options: `read_only` (default), `allow_list`, `all`. CI via GitHub Actions (`ci.yml`, `security.yml`, `release.yml`). Self-hosted runner setup: `ops/runner/`.

## Build

```bash
cargo build --release
cargo install --path .
```

## Test

```bash
cargo test --workspace
```

## Lint

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Rust CI Gate
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Release profile: `codegen-units = 1`, `lto = true`, `strip = true`.

## CI/CD
- Org repo (`Al-Sarraf-Tech/aihelp`) runs full CI pipeline. Personal repo (`jalsarraf0/aihelp`) runs no CI on GitHub.
- Org CI must pass before pushing to personal. Release artifacts on org repo.
- Runners: `linux-mega-1` (amarillo, 70°C limit), `wsl2-runner` (no limit), `dominus-runner` (no limit).
