# Architecture

## Crates

| Crate | Kind | Dependencies | What it is |
| --- | --- | --- | --- |
| `dotmatrix-core` | library | none (`png` for tests only) | The emulator: CPU, bus, PPU, APU, timer, cartridges, plus a PNG writer and test-ROM pass/fail detection |
| `dotmatrix-cli` | binary `dotmatrix` | core | Headless runner: screenshots, serial output, WAV capture, benchmarks, `check` over test ROM folders |
| `dotmatrix-web` | `cdylib` for `wasm32-unknown-unknown` | core | Fifteen `extern "C"` functions; the page reads pixels and samples straight out of linear memory |
| `dotmatrix-mcp` | binary `dotmatrix-mcp` | core, serde_json | stdio MCP server for agents |

The static site in `web/` is plain HTML, CSS and an ES module. There is no
bundler and no wasm-bindgen; the wasm module has zero imports.

## One clock, driven by the CPU

```text
            Cpu::step()
                |
      read / write / internal cycle
                |
                v
           Bus::tick()  ---- one M-cycle (4 T-cycles) ----+
                |                                         |
   +-----------+------------+-------------+---------------+-----------+
   |           |            |             |               |           |
 OAM DMA     Timer        PPU            APU            Serial     Cartridge
 1 byte    counter += 4   4 dots       4 T-cycles     on counter    RTC
            TIMA edge     (fast path     + resampler    bit 8 edge
            detection     if nothing
               |          happens)
               +--- counter bit 12 falling edge --> APU frame sequencer
```

There is no instruction cycle table. Every memory access the CPU makes goes
through `Cpu::read` / `Cpu::write`, which first advance the whole machine by
one M-cycle and then touch the bus; instructions with internal delays call
`Bus::tick` directly. So `PUSH BC` is "tick, write high, write low" and takes
16 T-cycles because that is what its body does, and the *position* of each
access within an instruction is right as well. Blargg's `mem_timing` and
Mooneye's OAM-DMA-stopwatch tests check exactly that.

Interrupts are sampled in the M-cycle that fetches the next opcode. If one
is pending, the fetched byte is discarded and dispatch continues for four
more M-cycles. HALT is modelled as that same fetch-and-discard cycle
repeating, which gives the right wake-up latency without a special case.

## Timer

DIV is the top byte of a 16-bit counter. TIMA increments on the falling edge
of `counter_bit(TAC) AND enable`, computed as a signal before and after
every change (tick, DIV write, TAC write). The DMG's timer glitches (DIV
writes and TAC changes that clock TIMA) fall out of that. Overflow reloads
TMA one M-cycle later, with the documented write behaviour in both cycles.

The APU's 512 Hz frame sequencer and the serial clock hang off the same
counter (bits 12 and 8), so writing DIV disturbs them like on hardware.

## PPU

Timing and pixels are handled separately:

- **Timing** is per dot. Mode 2 is 80 dots; mode 3 is 172 dots plus SCX fine
  scroll, window start (6) and object fetch penalties; mode 0 fills the
  line to 456. LY changes at dot 452, the LY=LYC flag is re-evaluated 4
  dots after LY changes, and line 153 reports LY=0 after 4 dots. VRAM/OAM
  read and write lockouts follow the windows measured by Mooneye's
  `lcdon_timing` / `lcdon_write_timing`. The STAT interrupt is one OR-ed
  line with rising-edge detection, which gives "STAT blocking" for free.
- **Pixels** are produced one scanline at a time when mode 3 ends, from the
  register values at that moment: background, window with its own line
  counter, and up to 10 objects per line chosen during OAM scan, with DMG
  priority (lower X, then lower OAM index; the winning opaque object owns
  the pixel even if BG-over-OBJ then hides it).

Because most M-cycles change nothing in the PPU, `Ppu::tick` first asks
for the next dot at which some state changes and skips ahead when it is
more than 4 dots away. That one change more than doubled overall speed.

The output is a 160x144 buffer of shade indices (0 = lightest). Frontends
choose the colours.

## APU

Two pulse channels (one with sweep), the wave channel and the noise LFSR,
each clocked in T-cycles. Length counters, sweep and envelopes run off the
frame sequencer, including the length-clock quirks Blargg's `dmg_sound`
checks. Every M-cycle the four DAC levels are summed as integers; when an
output sample is due (any sample rate) they are averaged (a box filter),
mixed through NR51/NR50 and passed through a DC-blocking high-pass filter
modelled on the output capacitor.

## Cartridges

`Cartridge` parses the header, pads ROM to a power of two and keeps three
precomputed offsets (bank 0 window, switchable window, RAM window) that are
recomputed on bank switches, so reads are a single index. ROM only, MBC1
(including multicart wiring), MBC2, MBC3 (+RTC counting emulated time,
8-bit bank register for 4 MiB MBC30 ROMs) and MBC5 are supported. Battery
RAM is exposed as a byte slice for the frontends to persist.

## Web frontend

```text
 requestAnimationFrame ──> dm_run_frame() x N ──> RGBA in wasm memory ──> putImageData
        │                        │
        │                        └──> f32 samples in wasm memory ──postMessage──> AudioWorklet ring buffer
        │                                                                              │
        └──────────── speed trimmed by ±0.6% to keep ~70 ms queued <── fill level ─────┘
```

The emulator follows the wall clock while the sound card follows its own
crystal, so the main thread nudges emulation speed by a fraction of a
percent based on how much audio the worklet has queued. Saves are written to
`localStorage` whenever the battery RAM's hash changes (checked twice a
second) and when the page is hidden.
