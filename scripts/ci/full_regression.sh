#!/usr/bin/env bash
set -euo pipefail

if command -v rustup >/dev/null 2>&1; then
  rustup component add rustfmt clippy
fi

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features

if ! cargo audit --version >/dev/null 2>&1; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
      AUDIT_ASSET="cargo-audit-x86_64-unknown-linux-gnu-v0.22.1.tgz"
      AUDIT_DIR="cargo-audit-x86_64-unknown-linux-gnu-v0.22.1"
      ;;
    *)
      echo "cargo-audit is missing and no bootstrap binary is configured for this platform" >&2
      exit 1
      ;;
  esac

  TMPDIR_AUDIT="$(mktemp -d)"
  trap 'rm -rf "$TMPDIR_AUDIT"' EXIT
  curl -fsSL \
    "https://github.com/rustsec/rustsec/releases/download/cargo-audit/v0.22.1/${AUDIT_ASSET}" \
    -o "${TMPDIR_AUDIT}/${AUDIT_ASSET}"
  tar -xzf "${TMPDIR_AUDIT}/${AUDIT_ASSET}" -C "${TMPDIR_AUDIT}"
  install -m 0755 "${TMPDIR_AUDIT}/${AUDIT_DIR}/cargo-audit" "${HOME}/.cargo/bin/cargo-audit"
fi

cargo audit

cargo build --release
