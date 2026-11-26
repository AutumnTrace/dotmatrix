# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- MBC3 battery saves carry the real-time clock in the 48-byte footer BGB and
  VBA-M use, and the clock catches up on wall-clock time when the save is
  loaded (#10). RAM-only saves and 44-byte footers still load. Clock-only
  carts (MBC3+TIMER+BATTERY) now get a save at all.

### Changed

- `GameBoy::save_data` and `GameBoy::load_save_data` take the current UNIX
  time; `save_data` returns an owned buffer. The wasm exports gain
  `dm_save_build` and `dm_save_ram_len`, and `dm_load_save` takes the time.

## [0.1.0] - 2026-11-23

First release.

### Added

- `dotmatrix-core`: SM83 CPU with all opcodes including the CB page,
  M-cycle-accurate memory access timing, interrupt dispatch sampled during the
  opcode fetch, EI delay, HALT and the HALT bug, and a basic STOP.
- Timer with falling-edge TIMA clocking, DIV/TAC glitches and the delayed
  TMA reload.
- PPU with per-dot mode, LY and STAT timing (variable mode 3 length from SCX,
  window and objects), STAT interrupt blocking, VRAM/OAM lockouts, OAM DMA,
  and a scanline renderer with window, 10 objects per line and DMG priority.
- APU with both pulse channels (sweep on channel 1), wave and noise channels,
  frame sequencer driven by DIV, and output at any sample rate.
- Cartridges: ROM only, MBC1 (including multicarts), MBC2, MBC3 with an RTC
  that counts emulated time, MBC5; battery RAM import/export.
- Serial port output capture, joypad with interrupt, a dependency-free PNG
  writer, and pass/fail detection for Blargg and Mooneye test ROMs.
- `dotmatrix` CLI: `run` (screenshots, serial output, WAV capture, battery
  saves, scripted button presses, `--bench`), `check` and `info`.
- Browser frontend on a 63 KB wasm module with C-ABI exports: canvas
  rendering with four palettes, keyboard, gamepad and touch input, drag and
  drop, localStorage saves, AudioWorklet audio, pause, fast forward and an
  FPS readout. Ships Tobu Tobu Girl as the demo cartridge.
- `dotmatrix-mcp`: stdio MCP server (protocol 2026-07-28 with fallback to the
  `initialize` handshake) with `load_rom`, `press_buttons`, `run_frames`,
  `screenshot` and `read_memory`.
- Test suites: Blargg, Mooneye, dmg-acid2, sprite_priority and MBC3 Tester
  run in CI against the c-sp/game-boy-test-roms v7.0 bundle.

[Unreleased]: https://github.com/AutumnTrace/dotmatrix/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/AutumnTrace/dotmatrix/releases/tag/v0.1.0
