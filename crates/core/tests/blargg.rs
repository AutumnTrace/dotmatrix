//! Blargg's CPU and timing test ROMs.

mod common;

use dotmatrix_core::harness;

/// Generous frame budgets: the slowest of these finishes in about 55
/// emulated seconds on a DMG.
const SERIAL_TIMEOUT: u64 = 60 * 70;

fn serial(relative: &str) {
    let Some(mut gb) = common::load(relative) else { return };
    common::assert_pass(relative, harness::blargg_serial(&mut gb, SERIAL_TIMEOUT));
}

fn memory(relative: &str) {
    let Some(mut gb) = common::load(relative) else { return };
    common::assert_pass(relative, harness::blargg_memory(&mut gb, SERIAL_TIMEOUT));
}

macro_rules! blargg_tests {
    ($runner:ident: $($name:ident => $path:literal,)*) => {
        $(
            #[test]
            fn $name() {
                $runner($path);
            }
        )*
    };
}

blargg_tests! { serial:
    cpu_instrs_01_special => "blargg/cpu_instrs/individual/01-special.gb",
    cpu_instrs_02_interrupts => "blargg/cpu_instrs/individual/02-interrupts.gb",
    cpu_instrs_03_op_sp_hl => "blargg/cpu_instrs/individual/03-op sp,hl.gb",
    cpu_instrs_04_op_r_imm => "blargg/cpu_instrs/individual/04-op r,imm.gb",
    cpu_instrs_05_op_rp => "blargg/cpu_instrs/individual/05-op rp.gb",
    cpu_instrs_06_ld_r_r => "blargg/cpu_instrs/individual/06-ld r,r.gb",
    cpu_instrs_07_jr_jp_call_ret_rst => "blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb",
    cpu_instrs_08_misc_instrs => "blargg/cpu_instrs/individual/08-misc instrs.gb",
    cpu_instrs_09_op_r_r => "blargg/cpu_instrs/individual/09-op r,r.gb",
    cpu_instrs_10_bit_ops => "blargg/cpu_instrs/individual/10-bit ops.gb",
    cpu_instrs_11_op_a_hl => "blargg/cpu_instrs/individual/11-op a,(hl).gb",
    instr_timing => "blargg/instr_timing/instr_timing.gb",
    mem_timing_01_read => "blargg/mem_timing/individual/01-read_timing.gb",
    mem_timing_02_write => "blargg/mem_timing/individual/02-write_timing.gb",
    mem_timing_03_modify => "blargg/mem_timing/individual/03-modify_timing.gb",
}

blargg_tests! { memory:
    mem_timing_2 => "blargg/mem_timing-2/mem_timing.gb",
    // The OAM corruption bug itself is not emulated; these three check that
    // nothing is corrupted when it should not be, and LCD timing.
    oam_bug_1_lcd_sync => "blargg/oam_bug/rom_singles/1-lcd_sync.gb",
    oam_bug_3_non_causes => "blargg/oam_bug/rom_singles/3-non_causes.gb",
    oam_bug_6_timing_no_bug => "blargg/oam_bug/rom_singles/6-timing_no_bug.gb",
}

/// The full cpu_instrs ROM runs all 11 groups back to back on one MBC1 cart.
#[test]
fn cpu_instrs_combined() {
    serial("blargg/cpu_instrs/cpu_instrs.gb");
}
