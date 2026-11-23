# Devlog

Notes from building dotmatrix: the bugs that took more than one try, and
what the test ROMs taught me. Roughly in the order they happened.

## The harness thought Blargg had failed

The first `dotmatrix check` run over Blargg's ROMs reported `06-ld r,r` and
`instr_timing` as failures with nonsense register dumps. Both were fine. My
auto-detecting harness treated any `LD B,B` (opcode `0x40`) as Mooneye's
"test finished" marker, and those two ROMs execute `LD B,B` as part of the
test itself: one exercises every `LD r,r`, the other times every opcode.

Fix: `LD B,B` only ends a run when B/C/D/E/H/L hold Mooneye's Fibonacci pass
signature (3/5/8/13/21/34) or its all-`0x42` failure signature.

## One off-by-one cycle, thirteen failing tests

After the first pass, 13 Mooneye timing tests failed together:
`call_timing`, `call_timing2`, `call_cc_timing`, `call_cc_timing2`,
`jp_timing`, `jp_cc_timing`, `ret_timing`, `ret_cc_timing`, `reti_timing`,
`rst_timing`, `push_timing`, `add_sp_e_timing`, `ld_hl_sp_e_timing`, plus
`oam_dma_timing` and `oam_dma_restart`. Seeing them fail as a group was the
clue: they all use OAM DMA as a stopwatch. A DMA is started, and the
instruction under test reads or writes OAM at the exact cycle the DMA lets
go of it.

`oam_dma_timing.s` spells out the expectation: a read 161 M-cycles after the
`FF46` write must still see `0xFF`, one cycle later it must see the data. My
DMA copied byte 159 and unlocked OAM in the same M-cycle, so the CPU access
at the end of that cycle already got through. OAM now stays locked through
the cycle that copies the last byte. All fifteen went green at once.

## rapid_toggle, and where interrupts are sampled

`timer/rapid_toggle` turns the timer on and off in a tight loop and counts
iterations until the timer interrupt fires. Expected `BC=$FFD9`, I got
`$FFD8`: one more trip around the loop. Tracing showed TIMA overflowing on
the "timer off" write (the falling-edge glitch, emulated correctly), the
reload and IF bit landing in the next M-cycle, and my CPU then happily
executing the following `DEC BC` before noticing the interrupt.

The SM83 samples interrupts in the M-cycle that fetches the next opcode, and
an interrupt raised during that cycle still wins. I had been checking
*before* the fetch. Moving the check after the fetch tick fixed
`rapid_toggle` and broke five tests that had been passing (`di_timing`,
`halt_ime1_timing2`, `intr_1_2_timing`, `intr_2_0_timing`,
`boot_sclk_align`).

What got everything consistent: treat HALT as the CPU repeatedly doing that
same fetch-and-discard cycle. When an interrupt arrives during HALT, the
idle cycle it arrived in *is* the fetch cycle, so wake-up costs nothing
extra. My first attempt added a wake-up cycle instead (Pan Docs mentions 4
extra T-cycles), which did not help; the extra cost is already the discarded
fetch. The HALT change brought the interrupt tests back and also fixed
`intr_2_mode0_timing`, `intr_2_mode3_timing` and `intr_2_oam_ok_timing`,
which had failed from the start. It broke `hblank_ly_scx_timing`, which is
further down.

## The DIV value at boot

There is no boot ROM in dotmatrix; the machine starts in post-boot state.
`boot_div-dmgABCmgb` checks both the value and the phase of DIV at
`0x0100`. Using the commonly quoted internal counter `0xABCC` failed. The
test file explains its own arithmetic (reads 64 M-cycles apart, the first
one "immediately after DIV has incremented"), and once I remembered the
cartridge header's `NOP; JP $0150` that runs first (5 M-cycles), the
start value that fits my "tick, then access" bus convention is `0xABC8`.
That also fixed `boot_sclk_align`, since the serial clock is derived from
the same counter.

## LY changes before the line ends

With the CPU change in place, `hblank_ly_scx_timing` measured LY
incrementing one M-cycle too late after the mode 0 interrupt. Moving the LY
increment to dot 452 of the line (4 dots before mode 2 of the next line)
fixed it without disturbing the mode-to-mode interrupt tests.
`lcdon_timing` later confirmed the same thing from a different angle: it
reads LY at fixed cycle offsets after turning the LCD on and expects 1 at
dot 452, not 456.

## Object penalties: Pan Docs vs. the STAT interrupt

`intr_2_mode0_timing_sprites` has 105 cases: 1 to 10 objects at
various X positions, and how many extra M-cycles mode 3 takes. I first
implemented Pan Docs' penalty algorithm (6 dots per object, plus up to 5
more depending on where the object sits inside its background tile, with a
flat 11 for X=0) and failed case 0.

Two things were off. Ten objects stacked at X=0 should cost 16 M-cycles,
which only works if they share one tile penalty (5 + 6×10 = 65 dots)
instead of 11 each. And with my mode 0 landing on the last dot of an
M-cycle, the measured counts are always `floor(penalty / 4)` while the SCX
penalty is `ceil(scx / 4)`. Working through the cases, the only adjustment
that fits all of them is a total that is 3 dots shorter than Pan Docs'
figure. Pan Docs derives its numbers from mid-scanline palette writes, which
see the pixel pipeline from a different point, so I am not claiming either
is "the" length; the code says what it matches.

## lcdon_timing and lcdon_write_timing

These two read and write OAM and VRAM at fixed offsets after the LCD is
turned on and compare against tables measured on hardware. Things they
taught me:

- Line 0 after LCD-on has no OAM scan; it sits in mode 0 and goes straight
  to mode 3.
- VRAM *reads* lock 4 dots before STAT says mode 3. VRAM *writes* lock
  exactly at mode 3.
- OAM *reads* lock 4 dots before the next visible line's OAM scan. OAM
  *writes* still land during the last M-cycle of OAM scan.
- The LY=LYC flag drops to 0 when LY changes and is compared again 4 dots
  later.

## stat_lyc_onoff

Turning the LCD off and on again must not fire a STAT interrupt if the LY=LYC
comparison result does not change (LY=$90/LYC=$90 before, LY=0/LYC=0 after).
I was resetting the internal STAT line when the LCD went off, so turning it
back on saw a fresh rising edge. The line and the comparison flag now keep
their values while the LCD is off.

## dmg-acid2 passed on the first run

Not a bug, but a surprise worth writing down: the scanline renderer matched
the dmg-acid2 reference image pixel for pixel the first time it ran. The
object priority rule that trips people up (the highest-priority opaque
object claims the pixel even if the background then wins over it, so a
lower-priority object never shows through) was in the renderer from the
start.

## Making it fast

The first benchmark: 1762 frames/s on Tobu Tobu Girl. The obvious suspect
was audio, since the APU mixed and converted four channels to floats every
M-cycle. Moving the mix to output-sample rate (summing integer DAC levels
per M-cycle and mixing once per output sample) only gained about 5%
(1762 to 1861 frames/s).

The real cost was the PPU being stepped dot by dot, 4 times per M-cycle,
each step re-evaluating the STAT line. Almost all of those dots change
nothing. The PPU now computes the next dot at which anything happens (mode
change, LY change, the LYC hold expiring) and, if that is more than 4 dots
away, just advances the counter. That took the same run to 4096 frames/s,
and the full test suite stayed green.

Near the end, switching the release profile from thin to fat LTO was worth
another 10 to 30 percent natively (Tobu Tobu Girl went from about 3,600 to
about 4,000 frames/s, `cpu_instrs` from about 3,400 to about 4,000), which
is more than I expected from a two-crate binary. The wasm module came out
the same size.

## MBC3 with 256 banks

`mbc3-tester` is a 4 MiB ROM and expects all 256 banks to be reachable. A
stock MBC3 has a 7-bit bank register; the 4 MiB variant (MBC30) uses all 8
bits. The register is now 8 bits wide, which is harmless for smaller ROMs
because bank numbers are reduced modulo the ROM size anyway.

## Mixer headroom

Recording 30 seconds of the demo game with `--wav` and looking at the
numbers showed a peak of 0.975: four channels at full swing plus the step
when a DAC switches on came close to clipping. The mixer now scales to leave
headroom.

## MCP moved under my feet

The MCP spec revision current at the time of writing (2026-07-28) removed the
`initialize` handshake: requests carry their protocol version in `_meta`, and
servers must implement `server/discover`. Most clients still speak the
2025 revisions, so `dotmatrix-mcp` is "dual-era": it answers `initialize` for
legacy clients and `server/discover` plus per-request version checks for
modern ones. The integration test drives both paths over real stdio.

## Small ones

- Playwright's `page.addStyleTag` before `page.goto` is silently lost on
  navigation. My first GIF capture came out at the wrong size because of it.
- Git on Windows does not record the executable bit, so the shell scripts
  are marked executable explicitly in the index for CI.
