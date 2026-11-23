//! Helpers shared by the test ROM suites.
//!
//! The ROMs are not part of the repository. `scripts/fetch-test-roms.sh`
//! downloads them into `test-roms/`; without them every ROM test prints a
//! SKIPPED line and passes, unless `DOTMATRIX_REQUIRE_TEST_ROMS=1` is set
//! (CI sets it, so a broken download cannot turn the suite green).

#![allow(dead_code)]

use std::path::PathBuf;

use dotmatrix_core::GameBoy;
use dotmatrix_core::harness::Verdict;

pub fn test_rom_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-roms")
}

/// Loads a test ROM, or returns None (after saying so) if it is missing.
pub fn load(relative: &str) -> Option<GameBoy> {
    let path = test_rom_dir().join(relative);
    match std::fs::read(&path) {
        Ok(rom) => Some(GameBoy::new(rom).expect("test ROM should load")),
        Err(_) => {
            if std::env::var_os("DOTMATRIX_REQUIRE_TEST_ROMS").is_some() {
                panic!("{} is missing and DOTMATRIX_REQUIRE_TEST_ROMS is set", path.display());
            }
            eprintln!("SKIPPED: {} not found (run scripts/fetch-test-roms.sh)", path.display());
            None
        }
    }
}

pub fn assert_pass(name: &str, verdict: Verdict) {
    match verdict {
        Verdict::Pass => {}
        Verdict::Fail(why) => panic!("{name} failed: {why}"),
        Verdict::Timeout => panic!("{name} timed out"),
    }
}
