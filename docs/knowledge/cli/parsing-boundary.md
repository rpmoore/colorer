---
type: concept
title: CLI Parsing Boundary
description: cli.rs defines argument parsing only and never performs device/filesystem I/O; commands/*.rs depends on it but lives separately.
resource: colorer/cli
tags: [cli, testability]
---

# CLI Parsing Boundary

`src/cli.rs:6-19` defines `Cli` (derives `clap::Parser`) and `Commands` (derives `clap::Subcommand`) with no device, filesystem, or network access — parsing only.

`src/main.rs:1-13` is the only place `Cli::parse()` is called; it matches on `Commands` and dispatches to command implementations.

This boundary is intentional and load-bearing for later sections: `commands/*.rs` (introduced starting in `section-02-list-hid`) depends on `Commands` variants but never lives in `cli.rs`, and never needs `clap` types to reach device code. This is what keeps `cli.rs` trivially unit-testable via `Cli::try_parse_from([...])` (`src/cli.rs:24-51`) without any device fakes, and keeps device-layer tests independent of argument-parsing concerns.

**Invariant to preserve:** if a future change adds I/O directly inside `cli.rs` or a `Commands` variant's fields, that breaks this boundary — device/filesystem access belongs in `commands/*.rs` and `device/*.rs`, reached only after parsing completes in `main.rs`.
