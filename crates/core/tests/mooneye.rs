//! Mooneye Test Suite (Gekkio). Only ROMs meant for DMG revisions A-C are
//! listed; the `-dmg0`, `-mgb`, `-S` (SGB) and `-sgb2` variants check boot
//! state of other models and are expected to fail on a DMG.

mod common;

use dotmatrix_core::harness;

/// Mooneye's own runner gives each test 120 emulated seconds.
const TIMEOUT_FRAMES: u64 = 60 * 120;

fn run(relative: &str) {
    let path = format!("mooneye-test-suite/{relative}");
    let Some(mut gb) = common::load(&path) else { return };
    common::assert_pass(relative, harness::mooneye(&mut gb, TIMEOUT_FRAMES));
}

macro_rules! mooneye_tests {
    ($($name:ident => $path:literal,)*) => {
        $(
            #[test]
            fn $name() {
                run($path);
            }
        )*
    };
}

mooneye_tests! {
    // CPU, interrupts and instruction timing
    add_sp_e_timing => "acceptance/add_sp_e_timing.gb",
    call_cc_timing => "acceptance/call_cc_timing.gb",
    call_cc_timing2 => "acceptance/call_cc_timing2.gb",
    call_timing => "acceptance/call_timing.gb",
    call_timing2 => "acceptance/call_timing2.gb",
    di_timing => "acceptance/di_timing-GS.gb",
    div_timing => "acceptance/div_timing.gb",
    ei_sequence => "acceptance/ei_sequence.gb",
    ei_timing => "acceptance/ei_timing.gb",
    halt_ime0_ei => "acceptance/halt_ime0_ei.gb",
    halt_ime0_nointr_timing => "acceptance/halt_ime0_nointr_timing.gb",
    halt_ime1_timing => "acceptance/halt_ime1_timing.gb",
    halt_ime1_timing2 => "acceptance/halt_ime1_timing2-GS.gb",
    if_ie_registers => "acceptance/if_ie_registers.gb",
    intr_timing => "acceptance/intr_timing.gb",
    jp_cc_timing => "acceptance/jp_cc_timing.gb",
    jp_timing => "acceptance/jp_timing.gb",
    ld_hl_sp_e_timing => "acceptance/ld_hl_sp_e_timing.gb",
    pop_timing => "acceptance/pop_timing.gb",
    push_timing => "acceptance/push_timing.gb",
    rapid_di_ei => "acceptance/rapid_di_ei.gb",
    ret_cc_timing => "acceptance/ret_cc_timing.gb",
    ret_timing => "acceptance/ret_timing.gb",
    reti_intr_timing => "acceptance/reti_intr_timing.gb",
    reti_timing => "acceptance/reti_timing.gb",
    rst_timing => "acceptance/rst_timing.gb",
    instr_daa => "acceptance/instr/daa.gb",
    interrupts_ie_push => "acceptance/interrupts/ie_push.gb",
    bits_mem_oam => "acceptance/bits/mem_oam.gb",
    bits_reg_f => "acceptance/bits/reg_f.gb",
    bits_unused_hwio => "acceptance/bits/unused_hwio-GS.gb",
    // Boot state (DMG A/B/C)
    boot_div_dmg_abc => "acceptance/boot_div-dmgABCmgb.gb",
    boot_hwio_dmg_abc => "acceptance/boot_hwio-dmgABCmgb.gb",
    boot_regs_dmg_abc => "acceptance/boot_regs-dmgABC.gb",
    serial_boot_sclk_align => "acceptance/serial/boot_sclk_align-dmgABCmgb.gb",
    // OAM DMA
    oam_dma_basic => "acceptance/oam_dma/basic.gb",
    oam_dma_reg_read => "acceptance/oam_dma/reg_read.gb",
    oam_dma_sources => "acceptance/oam_dma/sources-GS.gb",
    oam_dma_restart => "acceptance/oam_dma_restart.gb",
    oam_dma_start => "acceptance/oam_dma_start.gb",
    oam_dma_timing => "acceptance/oam_dma_timing.gb",
    // PPU
    ppu_hblank_ly_scx_timing => "acceptance/ppu/hblank_ly_scx_timing-GS.gb",
    ppu_intr_1_2_timing => "acceptance/ppu/intr_1_2_timing-GS.gb",
    ppu_intr_2_0_timing => "acceptance/ppu/intr_2_0_timing.gb",
    ppu_intr_2_mode0_timing => "acceptance/ppu/intr_2_mode0_timing.gb",
    ppu_intr_2_mode0_timing_sprites => "acceptance/ppu/intr_2_mode0_timing_sprites.gb",
    ppu_intr_2_mode3_timing => "acceptance/ppu/intr_2_mode3_timing.gb",
    ppu_intr_2_oam_ok_timing => "acceptance/ppu/intr_2_oam_ok_timing.gb",
    ppu_lcdon_timing => "acceptance/ppu/lcdon_timing-GS.gb",
    ppu_lcdon_write_timing => "acceptance/ppu/lcdon_write_timing-GS.gb",
    ppu_stat_irq_blocking => "acceptance/ppu/stat_irq_blocking.gb",
    ppu_stat_lyc_onoff => "acceptance/ppu/stat_lyc_onoff.gb",
    ppu_vblank_stat_intr => "acceptance/ppu/vblank_stat_intr-GS.gb",
    // Timer
    timer_div_write => "acceptance/timer/div_write.gb",
    timer_rapid_toggle => "acceptance/timer/rapid_toggle.gb",
    timer_tim00 => "acceptance/timer/tim00.gb",
    timer_tim00_div_trigger => "acceptance/timer/tim00_div_trigger.gb",
    timer_tim01 => "acceptance/timer/tim01.gb",
    timer_tim01_div_trigger => "acceptance/timer/tim01_div_trigger.gb",
    timer_tim10 => "acceptance/timer/tim10.gb",
    timer_tim10_div_trigger => "acceptance/timer/tim10_div_trigger.gb",
    timer_tim11 => "acceptance/timer/tim11.gb",
    timer_tim11_div_trigger => "acceptance/timer/tim11_div_trigger.gb",
    timer_tima_reload => "acceptance/timer/tima_reload.gb",
    timer_tima_write_reloading => "acceptance/timer/tima_write_reloading.gb",
    timer_tma_write_reloading => "acceptance/timer/tma_write_reloading.gb",
    // Memory bank controllers
    mbc1_bits_bank1 => "emulator-only/mbc1/bits_bank1.gb",
    mbc1_bits_bank2 => "emulator-only/mbc1/bits_bank2.gb",
    mbc1_bits_mode => "emulator-only/mbc1/bits_mode.gb",
    mbc1_bits_ramg => "emulator-only/mbc1/bits_ramg.gb",
    mbc1_multicart_rom_8mb => "emulator-only/mbc1/multicart_rom_8Mb.gb",
    mbc1_ram_256kb => "emulator-only/mbc1/ram_256kb.gb",
    mbc1_ram_64kb => "emulator-only/mbc1/ram_64kb.gb",
    mbc1_rom_16mb => "emulator-only/mbc1/rom_16Mb.gb",
    mbc1_rom_1mb => "emulator-only/mbc1/rom_1Mb.gb",
    mbc1_rom_2mb => "emulator-only/mbc1/rom_2Mb.gb",
    mbc1_rom_4mb => "emulator-only/mbc1/rom_4Mb.gb",
    mbc1_rom_512kb => "emulator-only/mbc1/rom_512kb.gb",
    mbc1_rom_8mb => "emulator-only/mbc1/rom_8Mb.gb",
    mbc2_bits_ramg => "emulator-only/mbc2/bits_ramg.gb",
    mbc2_bits_romb => "emulator-only/mbc2/bits_romb.gb",
    mbc2_bits_unused => "emulator-only/mbc2/bits_unused.gb",
    mbc2_ram => "emulator-only/mbc2/ram.gb",
    mbc2_rom_1mb => "emulator-only/mbc2/rom_1Mb.gb",
    mbc2_rom_2mb => "emulator-only/mbc2/rom_2Mb.gb",
    mbc2_rom_512kb => "emulator-only/mbc2/rom_512kb.gb",
    mbc5_rom_16mb => "emulator-only/mbc5/rom_16Mb.gb",
    mbc5_rom_1mb => "emulator-only/mbc5/rom_1Mb.gb",
    mbc5_rom_2mb => "emulator-only/mbc5/rom_2Mb.gb",
    mbc5_rom_32mb => "emulator-only/mbc5/rom_32Mb.gb",
    mbc5_rom_4mb => "emulator-only/mbc5/rom_4Mb.gb",
    mbc5_rom_512kb => "emulator-only/mbc5/rom_512kb.gb",
    mbc5_rom_64mb => "emulator-only/mbc5/rom_64Mb.gb",
    mbc5_rom_8mb => "emulator-only/mbc5/rom_8Mb.gb",
}
