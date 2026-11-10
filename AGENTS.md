# Repository Guidelines

## Overview

jl1his-terminal-unit is an RTTY terminal unit built on the RP2350 (Pimoroni
Tiny 2350), with firmware written in Rust with embassy. The PC sees it as a
composite USB device: a USB Audio microphone that carries the rig's receive
AF, and two CDC ACM ports, one keying FSK and PTT for MMTTY and similar
programs, the other taking control commands.

The design is recorded in `docs/design.md`. Read it before starting work.

Implementation has not started. It begins with stage 1, the two CDC ports and
PIO-based FSK / PTT. The UAC microphone and on-board demodulation follow once
stage 1 works on hardware.

## Architecture

- The firmware targets the Cortex-M33 cores of the RP2350
  (`thumbv8m.main-none-eabihf`) and uses `embassy-rp`, `embassy-usb` and
  `embassy-sync`.
- embassy changes its APIs between releases. Check each API against the
  source or docs.rs of the version locked in `Cargo.lock` rather than writing
  it from memory.
- Nothing may leave PTT or FSK keyed after power-up, during reset, on panic or
  after a watchdog reset. The hardware also turns the optocouplers off while
  their drive pins are undefined.
- PTT is released only after the buffer has been sent in full, even when the
  PC has already dropped RTS / DTR.
- The chip is stepping A2 and affected by erratum RP2350-E9. Do not rely on
  internal pull-downs: inputs use pull-ups and are pulled to GND, and a pin
  that must stay Low uses an external resistor of 8.2 kΩ or less. Disable the
  digital input on pins used by the ADC.
- Change the USB PID whenever the descriptor configuration changes. Windows
  caches the old configuration.

## Code Style

- Use Rust edition 2024.
- Follow standard Rust naming and formatting conventions.
- Use LF line endings and UTF-8 for all text files, and follow
  `.editorconfig`. There is no `.gitattributes` to correct this after the
  fact, so every edit has to write LF itself. On Windows a tool that opens a
  file in the platform's text mode converts LF to CRLF without reporting it:
  Python's `pathlib.write_text` and `open(path, "w")`, PowerShell's `>`
  redirection, and `Set-Content` all do. Write bytes instead, or edit in place
  with a tool that preserves what is already there.
- Check `git diff --stat` before committing. A small edit that reports the
  whole file as changed has had its line endings rewritten, and the fix is to
  rewrite the file with LF rather than to commit it.
- Combine imports from the same crate into a single `use` statement within each
  module scope, except when different `cfg` attributes require separate imports.
- Avoid comments by default. Add comments only when explicitly requested by the
  user.
- Prefer the smallest correct implementation and avoid speculative abstractions.
- Model ownership and state transitions explicitly; avoid global mutable state.
- Keep platform-specific types and dependencies out of reusable core APIs.
- Prefer `modname.rs` and `modname/` style over `mod.rs` style.
- Use `rstest` features for parameterized tests, fixtures, and test cases where
  they improve coverage or reduce repetition.

## Documentation

- Write documentation under `docs/` in English.
- When a change alters behavior, APIs, architecture or anything else
  documented, update the relevant documentation in the same change.
- Items marked "unverified" in `docs/design.md` come from memory. Check them
  against primary sources (crate source, datasheets, rig manuals) before
  building on them, then remove the mark and cite the source.
- When an item under "To Check Before Implementing" in `docs/design.md` is
  checked, tick it and write the result and its source under it.
- Do not settle items under "Undecided" in `docs/design.md` on your own, such
  as the pin assignment, the VID / PID or the control command set. Propose an
  option with its reasons and ask the user.

## Commits, Issues and PRs

- Use imperative mood. Do not use conventional commit messages.
- Name branches in lowercase, hyphenated, under `feature/`, `fix/`,
  `refactor/`, `docs/` or `test/`.

## Build and Test

Not set up yet. When the firmware crate is created, list here the commands to
build, flash and debug it, and the checks to run before completing a code
change, including at least `cargo fmt --check` and Clippy.
