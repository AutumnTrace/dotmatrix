//! Pass/fail detection for the common test ROM protocols.
//!
//! Shared by the integration tests and the `dotmatrix check` command.

use crate::GameBoy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(String),
    Timeout,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        *self == Verdict::Pass
    }
}

/// Mooneye protocol: the test executes `LD B,B` (0x40) when done, with
/// B/C/D/E/H/L = 3/5/8/13/21/34 on success (or all 0x42 on failure).
pub fn mooneye(gb: &mut GameBoy, max_frames: u64) -> Verdict {
    let limit = gb.cycles() + max_frames * crate::CYCLES_PER_FRAME;
    while gb.cycles() < limit {
        let pc = gb.registers().pc;
        if gb.peek(pc) == 0x40 {
            let r = gb.registers();
            let fib = [r.b, r.c, r.d, r.e, r.h, r.l];
            return if fib == [3, 5, 8, 13, 21, 34] {
                Verdict::Pass
            } else {
                Verdict::Fail(format!("registers {fib:02X?}"))
            };
        }
        if gb.is_locked() {
            return Verdict::Fail("CPU locked up on an illegal opcode".into());
        }
        gb.step();
    }
    Verdict::Timeout
}

/// Blargg protocol (serial flavour): the result is printed to the link port
/// and ends with "Passed" or "Failed".
pub fn blargg_serial(gb: &mut GameBoy, max_frames: u64) -> Verdict {
    for _ in 0..max_frames {
        gb.run_frame();
        let text = String::from_utf8_lossy(gb.serial_output());
        if text.contains("Passed") {
            return Verdict::Pass;
        }
        if text.contains("Failed") {
            return Verdict::Fail(text.trim().to_string());
        }
    }
    Verdict::Timeout
}

/// Blargg protocol (memory flavour): 0xA001..0xA003 hold DE B0 61 once the
/// test has started, 0xA000 is 0x80 while running and then the result code,
/// and a zero-terminated text log starts at 0xA004.
pub fn blargg_memory(gb: &mut GameBoy, max_frames: u64) -> Verdict {
    for _ in 0..max_frames {
        gb.run_frame();
        let signature = [gb.peek(0xA001), gb.peek(0xA002), gb.peek(0xA003)];
        if signature != [0xDE, 0xB0, 0x61] {
            continue;
        }
        match gb.peek(0xA000) {
            0x80 => continue,
            0x00 => return Verdict::Pass,
            code => return Verdict::Fail(format!("code {code}: {}", blargg_memory_text(gb).trim())),
        }
    }
    Verdict::Timeout
}

pub fn blargg_memory_text(gb: &GameBoy) -> String {
    let mut text = String::new();
    for addr in 0xA004..0xC000u16 {
        match gb.peek(addr) {
            0 => break,
            b => text.push(b as char),
        }
    }
    text
}

/// Compares the frame against reference shades (0 = lightest).
pub fn compare_frame(gb: &GameBoy, expected: &[u8]) -> Verdict {
    let actual = gb.framebuffer();
    let diff = actual.iter().zip(expected).filter(|(a, b)| a != b).count();
    if diff == 0 { Verdict::Pass } else { Verdict::Fail(format!("{diff} pixels differ from the reference")) }
}

/// Detects whichever protocol the ROM speaks: Mooneye's `LD B,B`, Blargg's
/// serial text, or Blargg's 0xA000 memory block.
pub fn auto(gb: &mut GameBoy, max_frames: u64) -> Verdict {
    let mut next_check = gb.cycles() + crate::CYCLES_PER_FRAME;
    let limit = gb.cycles() + max_frames * crate::CYCLES_PER_FRAME;
    while gb.cycles() < limit {
        let pc = gb.registers().pc;
        if gb.peek(pc) == 0x40 {
            // Blargg ROMs execute LD B,B too (06-ld r,r, instr_timing), so
            // only the two Mooneye register signatures end the run here.
            let r = gb.registers();
            let fib = [r.b, r.c, r.d, r.e, r.h, r.l];
            if fib == [3, 5, 8, 13, 21, 34] {
                return Verdict::Pass;
            }
            if fib == [0x42; 6] {
                return Verdict::Fail("Mooneye failure signature (0x42)".into());
            }
        }
        if gb.is_locked() {
            return Verdict::Fail("CPU locked up on an illegal opcode".into());
        }
        gb.step();
        if gb.cycles() >= next_check {
            next_check += crate::CYCLES_PER_FRAME;
            let text = String::from_utf8_lossy(gb.serial_output());
            if text.contains("Passed") {
                return Verdict::Pass;
            }
            if text.contains("Failed") {
                return Verdict::Fail(text.trim().to_string());
            }
            if [gb.peek(0xA001), gb.peek(0xA002), gb.peek(0xA003)] == [0xDE, 0xB0, 0x61] {
                match gb.peek(0xA000) {
                    0x80 => {}
                    0x00 => return Verdict::Pass,
                    code => return Verdict::Fail(format!("code {code}: {}", blargg_memory_text(gb).trim())),
                }
            }
        }
    }
    Verdict::Timeout
}
