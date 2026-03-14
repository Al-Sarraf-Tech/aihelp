# CI/CD Hardening Report -- aihelp

**Repository:** `jalsarraf0/aihelp`
**Date:** 2026-03-14
**Branch:** `ci/assurance-hardening`

---

## Pre-Existing CI/CD Infrastructure

aihelp had three workflows in place before hardening:

| Workflow | Status | Notes |
|---|---|---|
| CI (`ci.yml`) | Operational | Docker regression, compile sanity check |
| Security (`security.yml`) | Minimal | cargo-audit only, weekly schedule, no PR trigger |
| Release (`release.yml`) | Operational | Multi-platform build, attestation, GitHub Release |

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
| cargo-deny job in `security.yml` | New CI job | Runs `cargo deny check` on every push, PR, and weekly schedule |
| Gitleaks job in `security.yml` | New CI job | Full-history secret scan with SARIF upload to GitHub Security tab |
| Concurrency controls in `security.yml` | Workflow enhancement | `security-${{ github.ref }}` group with cancel-in-progress |
| Push/PR triggers in `security.yml` | Workflow enhancement | Security now runs on push to main and on PRs (previously schedule-only) |
| Permissions block in `security.yml` | Workflow enhancement | Least-privilege: `contents: read`, `security-events: write` |
| Concurrency controls in `ci.yml` | Workflow enhancement | `ci-${{ github.ref }}` group with cancel-in-progress |
| `ASSURANCE.md` | New file | Comprehensive software assurance document |
| `CI_CD_HARDENING_REPORT.md` | New file | This report |

---

## What Was NOT Changed

- `release.yml` was not modified (already has attestation and proper permissions)
- No source code was modified
- No existing CI jobs were removed or altered in behavior
- README.md badges already covered CI, Security, and Release workflows

---

## Verification

All changes are structural (YAML workflow definitions, TOML policy, Markdown docs).
Syntax validation:

- `deny.toml`: valid TOML, matches cargo-deny schema
- `security.yml`: valid GitHub Actions YAML
- `ci.yml`: valid GitHub Actions YAML (concurrency block added)

---

## Remaining Recommendations

| Item | Priority | Notes |
|---|---|---|
| CodeQL workflow | Medium | Add `codeql.yml` for Rust semantic analysis (SSH-Hunt model) |
| SBOM generation | Medium | Add CycloneDX SBOM workflow for source-level bill of materials |
| Trivy filesystem scan | Low | Additional vulnerability scanning layer |
| OSV-Scanner | Low | Google OSV database cross-reference |
