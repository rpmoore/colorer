# RUST.md

Rust-specific standards for `colorer`. `AGENTS.md` is the source of truth for process/workflow; this file is the source of truth for toolchain- and language-level rules.

## Gates

Every change must pass, before being reported done:

```
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

## Testing approach

`cargo test` only — no extra test crates (no `assert_cmd`/`predicates`, no `mockall`). Unit tests for parsing/formatting logic. Device I/O sits behind a trait (`DeviceBackend`, and later `HidTransport`) so it can be faked in tests without touching real hardware or requiring elevated privileges. Filesystem-backed backends (e.g. the sysfs backend) take their root path as a constructor parameter so tests can point them at a temp directory instead of the real filesystem.

## Error handling

Domain errors go through a shared `DeviceError` enum (see `src/device/mod.rs` once it exists) rather than `anyhow`/`eyre`-style dynamic errors — command-layer code needs to branch on error kind (not found vs. permission denied vs. unsupported, etc.), which a typed enum makes possible.

## Dependency policy

Prefer small, focused crates already anticipated by the implementation plan (`clap`, `hidapi`) over pulling in new dependencies for one-off convenience. Justify any new dependency against what it replaces (hand-rolled code, or a heavier alternative).
