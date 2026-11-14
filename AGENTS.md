# Repository Guidelines

## Overview

jl1his-terminal-unit is an RTTY terminal unit built on the RP2350 (Raspberry
Pi Pico 2), with firmware in Rust on embassy. The PC sees a composite USB
device: a USB Audio microphone that carries the rig's receive AF, and a CDC
ACM control port that takes text commands to key FSK and PTT and to change
settings.

Read `docs/design.md` (the design and what is implemented so far) and
`docs/control-port.md` (the protocol of the control port) before starting
work.

## Architecture

- `firmware/` is a Cargo workspace. `firmware/core/` (`terminal-unit-core`)
  is a `no_std` library without platform dependencies, tested on the host.
  `firmware/app/` (`terminal-unit-firmware`) is the RP2350 binary.
- `hardware/circuit/` holds the KiCad 10 project of the rig interface.
  `terminal-unit.kicad_pcb` is a parts layout for a 2.54 mm universal board,
  not a board to be fabricated.
- `hardware/enclosure/` holds the FreeCAD model of the case. Its dimensions
  live in the `Params` spreadsheet of the file.
- embassy changes its APIs between releases. Check each API against the
  source or docs.rs of the version locked in `Cargo.lock` rather than writing
  it from memory.
- Nothing may leave PTT or FSK keyed after power-up, during reset, on panic or
  after a watchdog reset.
- Keep the RP2350-E9 workarounds in section 5 of `docs/design.md`.
- Change the USB PID whenever the descriptor configuration changes, because
  Windows caches the old one. See "USB VID / PID" in `docs/design.md`.

## Code Style

- Use LF line endings and UTF-8, and follow `.editorconfig`. There is no
  `.gitattributes`, so every edit has to write LF itself. On Windows,
  text-mode writes turn LF into CRLF without reporting it: Python's
  `pathlib.write_text` and `open(path, "w")`, PowerShell's `>` and
  `Set-Content`. If `git diff --stat` reports a whole file as changed after a
  small edit, rewrite the file with LF instead of committing it.
- Combine imports from the same crate into a single `use` statement within
  each module scope, except where `cfg` attributes differ.
- Avoid comments by default. Add comments only when explicitly requested by
  the user.
- Prefer the smallest correct implementation and avoid speculative
  abstractions.
- Model ownership and state transitions explicitly; avoid global mutable
  state.
- Keep platform-specific types and dependencies out of `core`.
- Prefer `modname.rs` and `modname/` style over `mod.rs` style.
- Use `rstest` for parameterized tests and fixtures where it improves
  coverage or reduces repetition.

## Documentation

- Write documentation under `docs/` in English, and update it in the same
  change that alters what it describes.
- Items marked "unverified" in `docs/design.md` come from memory. Check them
  against primary sources (crate source, datasheets, rig manuals) before
  building on them, then remove the mark and cite the source.
- When an item under "To Check Before Implementing" is checked, tick it and
  write the result and its source under it.
- Do not settle items under "Undecided" on your own. Propose an option with
  its reasons and ask the user.

## Commits, Issues and PRs

- Use imperative mood. Do not use conventional commit messages.
- Name branches in lowercase, hyphenated, under `feature/`, `fix/`,
  `refactor/`, `docs/` or `test/`.

## Build and Test

The firmware does not build for the host, so the workspace root, `firmware/`,
defaults to `core` alone. The target and the runner are set in
`firmware/app/.cargo/config.toml`, which Cargo reads only when it is run
inside `firmware/app/`.

In `firmware/`:

- `cargo test` runs the host tests of `core`.
- `cargo clippy --all-targets -- -D warnings` checks `core`.
- `cargo fmt --all --check` checks the formatting of both crates.

In `firmware/app/`:

- `cargo build` builds the firmware. `--features tiny2350` builds it for the
  Pimoroni Tiny 2350.
- `cargo clippy -- -D warnings` checks the firmware.
- `cargo run` flashes the board through a debug probe with `probe-rs` and
  prints the defmt log over RTT.

Run all of these except `cargo run` before completing a code change.
