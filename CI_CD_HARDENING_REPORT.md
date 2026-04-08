# CI/CD Hardening Report -- aihelp

**Repository:** `Al-Sarraf-Tech/aihelp`
**Date:** 2026-03-14
**Branch:** `ci/assurance-hardening`

---

## Pre-Existing CI/CD Infrastructure

aihelp had these workflows in place before hardening:

| Workflow | Status | Notes |
|---|---|---|
| `ci-rust.yml` | Operational | Lint, test, security, Linux release build, GitHub Release |

### What Was Missing

- No `deny.toml` or cargo-deny policy enforcement
- No Gitleaks secret scanning
- No concurrency controls on any workflow
- Security workflow only triggered on schedule + manual (not on push/PR)
- No ASSURANCE.md or hardening documentation

---

## What Was Added

| Item | Type | Description |
|---|---|---|
| `deny.toml` | New file | cargo-deny policy: deny unmaintained/unsound/yanked, license allowlist, deny unknown registries/git |
| cargo-deny job in `ci-rust.yml` | New CI job | Runs `cargo deny check` on every push, PR, and weekly schedule |
| Gitleaks job in `ci-rust.yml` | New CI job | Full-history secret scan via gitleaks |
| Concurrency controls in `ci-rust.yml` | Workflow enhancement | `aihelp-ci-${{ github.ref }}` group with cancel-in-progress |
| Push/PR triggers for security | Workflow enhancement | Security jobs now run on push to main and on PRs |
| Least-privilege permissions in `ci-rust.yml` | Workflow enhancement | `contents: write`, `id-token: write` |
| `ASSURANCE.md` | New file | Comprehensive software assurance document |
| `CI_CD_HARDENING_REPORT.md` | New file | This report |

---

## What Was NOT Changed

- Release job inside `ci-rust.yml` was not modified (already has proper permissions)
- No source code was modified
- No existing CI jobs were removed or altered in behavior
- README.md badges already covered CI, Security, and Release workflows

---

## Verification

All changes are structural (YAML workflow definitions, TOML policy, Markdown docs).
Syntax validation:

- `deny.toml`: valid TOML, matches cargo-deny schema
- `ci-rust.yml`: valid GitHub Actions YAML (all jobs: lint, test, security, release)

---

## Remaining Recommendations

| Item | Priority | Notes |
|---|---|---|
| CodeQL workflow | Medium | Add `codeql.yml` for Rust semantic analysis (SSH-Hunt model) |
| SBOM generation | Medium | Add CycloneDX SBOM workflow for source-level bill of materials |
| Trivy filesystem scan | Low | Additional vulnerability scanning layer |
| OSV-Scanner | Low | Google OSV database cross-reference |
