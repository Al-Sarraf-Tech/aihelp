# Software Assurance

This document describes the CI/CD gates, security scanning, supply chain protections,
and quality controls that guard the aihelp codebase. Every pull request and push to
`main` must pass these gates before code is merged or released.

---

## Table of Contents

- [CI/CD Pipeline Overview](#cicd-pipeline-overview)
- [Code Quality Gates](#code-quality-gates)
- [Security Scanning](#security-scanning)
- [Supply Chain Security](#supply-chain-security)
- [Concurrency and Runner Controls](#concurrency-and-runner-controls)
- [Running Checks Locally](#running-checks-locally)
- [How This Protects Against Regressions](#how-this-protects-against-regressions)

---

## CI/CD Pipeline Overview

aihelp uses three GitHub Actions workflows that collectively enforce code quality,
security posture, and supply chain integrity:

| Workflow | File | Trigger | Purpose |
|---|---|---|---|
| **CI / Security / Release** | `ci-rust.yml` | push, PR, `v*` tags, weekly schedule | Lint, test, security scanning, Linux release build |
| **Orchestrator Scan** | `orchestrator-scan.yml` | push/PR to `.github/workflows/**` | Haskell Orchestrator governance scan |

All jobs target self-hosted runners (`[self-hosted, unified-all]`).

---

## Code Quality Gates

### Formatting

`cargo fmt --all -- --check` enforces consistent Rust formatting. PRs with formatting
violations are rejected.

### Linting

`cargo clippy --workspace --all-targets -- -D warnings` treats every Clippy warning as
a hard error. This catches common bugs, performance issues, and idiomatic violations
before review.

### Testing

- **Full workspace tests**: `cargo test --workspace` runs the complete test suite.
- **Docker regression**: The CI workflow runs a full regression and security gate inside
  a Docker container for reproducible results.
- **Compile sanity check**: A separate job verifies `cargo check --all-features --all-targets`.

---

## Security Scanning

### cargo-audit

Checks `Cargo.lock` against the RustSec Advisory Database for known vulnerabilities in
dependencies. Runs on every push, PR, and weekly schedule.

### cargo-deny

Enforces policy rules defined in `deny.toml`:

- **Advisories**: denies unmaintained, unsound, and yanked crates.
- **Bans**: warns on duplicate dependency versions.
- **Licenses**: rejects dependencies with incompatible licenses; allows common OSS licenses
  (MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib, MPL-2.0,
  CC0-1.0, OpenSSL, BSL-1.0).
- **Sources**: denies unknown registries and unknown Git sources.

### Gitleaks

Scans the full Git history for accidentally committed secrets (API keys, tokens,
passwords, private keys). Results are uploaded as SARIF to the GitHub Security tab.

---

## Supply Chain Security

### Locked Dependencies

All CI build and test steps use `--locked` to ensure the exact dependency versions from
`Cargo.lock` are used. Any dependency drift between local development and CI is caught
immediately.

### Release Checksums

Every GitHub Release includes `SHA256SUMS` with checksums for all published Linux artifacts.

---

## Concurrency and Runner Controls

### Concurrency Groups

Each workflow uses concurrency groups scoped to `${{ github.ref }}` with
`cancel-in-progress: true`. This ensures that:

- Redundant runs on the same branch are cancelled automatically.
- CI resources are not wasted on superseded commits.
- Results always reflect the latest pushed state.

### Least-Privilege Permissions

The workflow declares explicit `permissions` blocks scoped to the minimum required:

- `ci-rust.yml`: `contents: write`, `id-token: write` (write needed for GitHub Release on tags)

### Runner Selection

All jobs run on self-hosted runners (`[self-hosted, Linux, X64, docker]`) for
consistent, reproducible builds.

---

## Running Checks Locally

Contributors can run the same checks that CI enforces before pushing:

```bash
# Formatting
cargo fmt --all --check

# Linting
cargo clippy --workspace --all-targets -- -D warnings

# Full test suite
cargo test --workspace

# Dependency audit
cargo audit

# License and policy check
cargo deny check --config deny.toml --hide-inclusion-graph bans licenses sources

# Secret scan (requires gitleaks)
gitleaks detect --source . --redact --verbose
```

---

## How This Protects Against Regressions

| Risk | Mitigation |
|---|---|
| Formatting drift | `cargo fmt --check` on every PR |
| Lint regressions | `clippy -D warnings` on every PR |
| Test failures | Full workspace tests + Docker regression on every PR |
| Known CVEs in dependencies | cargo-audit on every PR + weekly |
| License violations | cargo-deny policy enforcement on every PR |
| Leaked secrets | Gitleaks full-history scan on every PR |
| Dependency version drift | `--locked` flag on all cargo commands in CI |
| Wasted CI resources | Concurrency groups with cancel-in-progress |
| Over-privileged workflows | Explicit least-privilege permission blocks |
| Tampered release artifacts | SHA256 checksums published alongside release artifacts |
| Unmaintained/unsound dependencies | cargo-deny advisory policy on every PR |
