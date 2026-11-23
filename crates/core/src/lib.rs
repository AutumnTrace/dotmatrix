//! dotmatrix: a Game Boy (DMG) emulator core with no dependencies.
//!
//! ```no_run
//! use dotmatrix_core::{Button, GameBoy};
//!
//! let rom = std::fs::read("game.gb").unwrap();
//! let mut gb = GameBoy::new(rom).unwrap();
//! gb.set_button(Button::Start, true);
//! gb.run_frame();
//! let shades: &[u8] = gb.framebuffer(); // 160x144, 0 = lightest
//! # let _ = shades;
//! ```

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
mod gameboy;
pub mod harness;
pub mod joypad;
pub mod png;
pub mod ppu;
pub mod serial;
pub mod timer;

pub use cartridge::{CartridgeError, Header};
pub use cpu::Registers;
pub use gameboy::{CLOCK_HZ, CYCLES_PER_FRAME, DEFAULT_SAMPLE_RATE, FRAMES_PER_SECOND, GameBoy};
pub use joypad::Button;
pub use ppu::{HEIGHT, WIDTH};
