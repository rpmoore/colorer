Now I have all the context needed. Let me produce the section content.

---
title: "section-01-cli-scaffold"
milestone: M0
depends_on: []
blocks: [section-02-list-hid]
---

# Section 01: CLI Scaffold (M0)

## Goal

`colorer --help` and `colorer --version` work. No device logic exists yet. This is the "always runs" baseline the rest of the project builds on. This milestone also establishes the `cargo fmt`/`cargo clippy` gates that apply to every subsequent milestone.

## Current State of the Repo

This is a bare `cargo init` scaffold:
- `Cargo.toml` exists with only `[package]` (`name = "colorer"`, `edition = "2024"`) and an empty `[dependencies]` table.
- `src/main.rs` exists with the default `cargo init` placeholder body (`println!("Hello, world!")`).
- No other source files exist yet.

## What to Build

### 1. Add `clap` to `Cargo.toml`

Add the `clap` crate with the derive feature to `[dependencies]` in `/home/rpmoore/code/colorer/Cargo.toml`:

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
```

(Pin to whatever the current `4.x` release resolves to via `cargo add clap --features derive`, or specify the version directly — either is fine as long as the derive feature is enabled.)

### 2. `src/cli.rs` — CLI definitions (parsing only, no device logic)

Create `/home/rpmoore/code/colorer/src/cli.rs`. This module defines the top-level `Cli` struct (derives `clap::Parser`) and a `Commands` enum (derives `clap::Subcommand`). For M0, `Commands` has a single placeholder variant, `List`, whose args are added starting in M1 (section-02) — for M0, `ListArgs` can be an empty/near-empty struct since the `--all` flag doesn't exist until M1.

This module must never touch devices, the filesystem, or any I/O beyond argument parsing — that separation is what keeps `cli.rs` trivially testable and is a hard boundary the rest of the plan depends on (see `commands/*.rs` in later sections, which depend on `Commands` variants but never live in this file).

Signatures:

```rust
/// Top-level CLI definition. Parsed once in `main`.
#[derive(clap::Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available subcommands.
#[derive(clap::Subcommand)]
enum Commands {
    List(ListArgs),
}

/// Args for the `list` subcommand. Empty for M0; `--all` is added in section-02 (M1).
#[derive(clap::Args)]
struct ListArgs {}
```

Use `clap`'s standard derive attributes (`#[command(...)]`, `#[arg(...)]`) as needed for `--help`/`--version` behavior — `clap::Parser`'s derive gives `--help` and `--version` for free when `#[command(version)]` (or equivalent) is set on `Cli`; make sure version output is actually enabled, since a bare derive without `#[command(version)]` will not automatically wire up `--version`.

### 3. `src/main.rs` — entry point

Update `/home/rpmoore/code/colorer/src/main.rs` to parse `Cli` and match on `Commands`. For M0, the `List` arm has no real implementation yet — it may either be a placeholder that prints nothing meaningful (but does not `todo!()`/panic — it must exit 0), or, at the implementer's discretion, stub directly into M1's `list` command if M0 and M1 end up implemented together. Either way, M0 alone must still build and correctly run `--help` and `--version`.

```rust
mod cli;

fn main() {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Commands::List(_args) => {
            // M0: no device logic yet. Real implementation lands in section-02 (M1).
        }
    }
}
```

(Exact placeholder behavior for the `List` arm is an implementation detail — the requirement is only that `cargo run` with no subcommand-specific device logic exits cleanly and `--help`/`--version` work.)

## Tests (write first)

All tests live alongside `cli.rs` (e.g. a `#[cfg(test)] mod tests` block in `src/cli.rs`), using `clap`'s recommended testing patterns — no extra test crates needed, per this project's `cargo test`-only testing approach.

- **Structural check (standing test):** `Cli::command().debug_assert()` passes. This is clap's recommended pattern for catching a structurally invalid derive (e.g. conflicting arg names) at test time rather than only at runtime. This test should remain in place permanently, not just for M0 — every later section that adds new args to `Cli`/`Commands` relies on this same check continuing to pass.
- **Help/version parsing:** `Cli::try_parse_from(["colorer", "--help"])` and `Cli::try_parse_from(["colorer", "--version"])` each behave as clap's built-in handling expects — i.e. they produce the `Err` variant clap uses for early-exit display (`clap::error::ErrorKind::DisplayHelp` / `DisplayVersion`, via `.unwrap_err().kind()`), not a panic and not a plain successful parse.
- **Valid subcommand parsing:** `Cli::try_parse_from(["colorer", "list"])` parses successfully into `Commands::List` with default `ListArgs`.
- **Missing subcommand:** `Cli::try_parse_from(["colorer"])` (no subcommand given) produces a clear parse `Err`, not a panic — `Commands` has no default and no subcommand is optional at this stage, so omitting one must be a clean argument error.

Example test shapes (fill in bodies; exact assertions per the bullets above):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn debug_assert_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_flag_triggers_display_help() {
        // Cli::try_parse_from(["colorer", "--help"]) -> Err with kind() == DisplayHelp
    }

    #[test]
    fn version_flag_triggers_display_version() {
        // Cli::try_parse_from(["colorer", "--version"]) -> Err with kind() == DisplayVersion
    }

    #[test]
    fn list_subcommand_parses() {
        // Cli::try_parse_from(["colorer", "list"]) -> Ok(Cli { command: Commands::List(_) })
    }

    #[test]
    fn missing_subcommand_is_error() {
        // Cli::try_parse_from(["colorer"]) -> Err(_)
    }
}
```

This `try_parse_from`-based testing requirement (valid and invalid argument shapes, no panics) is not unique to M0 — it applies to every subsequent section that adds new CLI args, so establishing the pattern cleanly here matters for later sections.

## Manual Verification

- `cargo run -- --help` prints expected help output and exits 0.
- `cargo run -- --version` prints expected version output and exits 0.
- `cargo build` succeeds.
- `cargo test` passes (all tests above).
- `cargo fmt --check` passes clean.
- `cargo clippy -- -D warnings` passes clean.

Establishing the `fmt`/`clippy` gates here (not deferring them) matches this repo's `AGENTS.md`, which calls for fmt/clippy gates once the crate exists — M0 is that point. These same four gates (`cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings`) apply to every later section as well, plus a manual run of that section's new subcommand.

## Notes for the Implementer

- Do not add any device-related types, traits, or modules in this section — `device/`, `color.rs`, and `commands/` are out of scope here and are introduced starting in section-02 (M1) and later. Keep `cli.rs` strictly about argument parsing.
- `ListArgs` is intentionally minimal in this section (no fields, or only fields already needed for parsing to succeed) — the `--all` flag and the real `list` command implementation are section-02's responsibility (`src/commands/list.rs`, `src/device/*.rs`). Don't build ahead of that section.
- This section blocks section-02-list-hid (M1) and has no dependencies of its own — it is the first section in execution order.

## Implementation Notes (actual, post-review)

Implemented as planned: `src/cli.rs` (`Cli`/`Commands`/`ListArgs`, all 5 required tests), `src/main.rs` dispatch, `clap` added with the derive feature. All gates (`cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings`) pass; `--help`/`--version` verified manually.

**Deviations from plan, added during code review:**
- `RUST.md` and the `docs/knowledge/` OKF bundle (`docs/knowledge/index.md`, `docs/knowledge/cli/index.md`, `docs/knowledge/cli/parsing-boundary.md`) were added — not called for by this plan file, but required by `AGENTS.md`'s own Rust Code Standards and Knowledge Bundle triggers, which the code review flagged as unmet. `docs/knowledge/cli/parsing-boundary.md` documents the parsing-only invariant this section establishes (`cli.rs` never touches devices/I/O), grounded at `src/cli.rs:6-19` and `src/main.rs:1-13`.
- `Cargo.toml` got a `description` field, and `clap`'s version requirement was loosened from an exact-pinned `4.6.6` to `"4"` — both trivial, flagged by review as avoidable friction.
- `///` doc comments on `Cli` and `ListArgs` were changed to `//` (non-doc) comments, because `clap`'s derive macro was rendering them verbatim as `--help` output text (internal implementation notes, not user-facing copy). `Commands::List` got an explicit `/// List discovered RGB-capable devices.` doc comment instead, since that one *is* meant to reach `--help` output. `Cli`'s `about` text now comes from `Cargo.toml`'s new `description` field.
- CI enforcement (GitHub Actions) was raised by code review as a gap but explicitly deferred by the user — out of this section's planned scope; may be added later as its own task.

No deviation was needed to `ListArgs` itself (still empty, `--all` still deferred to section-02) or to the test set (all 5 planned tests implemented as specified, no additions or removals).