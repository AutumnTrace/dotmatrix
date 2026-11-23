//! Test ROMs that report through the picture rather than through serial or
//! registers. Each frame is compared pixel for pixel with the reference
//! screenshot shipped in the test ROM bundle (greys 00/55/AA/FF).

mod common;

use dotmatrix_core::{GameBoy, harness};

/// Runs until the ROM is about to execute `LD B,B`, its "done" marker.
fn run_until_ld_b_b(gb: &mut GameBoy, name: &str) {
    let mut steps = 0u64;
    while gb.peek(gb.registers().pc) != 0x40 {
        gb.step();
        steps += 1;
        assert!(steps < 20_000_000, "{name} never reached LD B,B");
    }
}

fn compare(name: &str, rom: &str, reference: &str, finish: impl FnOnce(&mut GameBoy)) {
    let Some(mut gb) = common::load(rom) else { return };
    let expected = common::reference_shades(reference).expect("reference image");
    assert_eq!(expected.len(), 160 * 144);
    finish(&mut gb);
    common::assert_pass(name, harness::compare_frame(&gb, &expected));
}

/// Matt Currie's PPU smoke test: background, window and object features
/// drawn into a face.
#[test]
fn dmg_acid2() {
    compare("dmg-acid2", "dmg-acid2/dmg-acid2.gb", "dmg-acid2/dmg-acid2-dmg.png", |gb| {
        run_until_ld_b_b(gb, "dmg-acid2");
        // Let the PPU finish the frame it is drawing.
        gb.run_frame();
        gb.run_frame();
    });
}

/// Object-to-object priority (lower X wins, then lower OAM index) and the
/// 10-per-line limit.
#[test]
fn mooneye_sprite_priority() {
    compare(
        "sprite_priority",
        "mooneye-test-suite/manual-only/sprite_priority.gb",
        "mooneye-test-suite/manual-only/sprite_priority-dmg.png",
        |gb| {
            run_until_ld_b_b(gb, "sprite_priority");
            gb.run_frame();
            gb.run_frame();
        },
    );
}

/// Blargg's halt_bug only reports on screen; the c-sp how-to gives it two
/// emulated seconds.
#[test]
fn blargg_halt_bug() {
    compare("halt_bug", "blargg/halt_bug.gb", "blargg/halt_bug-dmg-cgb.png", |gb| {
        for _ in 0..(60 * 3) {
            gb.run_frame();
        }
    });
}

/// MBC3 ROM banking; the result is on screen after about 40 frames.
#[test]
fn mbc3_tester() {
    compare("mbc3-tester", "mbc3-tester/mbc3-tester.gb", "mbc3-tester/mbc3-tester-dmg.png", |gb| {
        for _ in 0..60 {
            gb.run_frame();
        }
    });
}
