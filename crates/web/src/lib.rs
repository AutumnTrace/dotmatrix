//! WebAssembly exports for the browser frontend.
//!
//! No wasm-bindgen: the page talks to a handful of `extern "C"` functions
//! and reads the framebuffer and audio samples straight out of linear
//! memory. All pointers handed to JavaScript stay valid until the next call
//! into this module, but JS must rebuild its typed-array views after any
//! call that may grow memory (loading a ROM does).
//!
//! Return codes for `dm_load_rom`: 0 ok, 1 not a ROM, 2 unsupported mapper,
//! 3 Game Boy Color only.

use std::cell::RefCell;

use dotmatrix_core::{Button, CartridgeError, GameBoy, HEIGHT, WIDTH};

struct State {
    gb: Option<GameBoy>,
    /// Scratch buffer JS writes ROM and save data into.
    input: Vec<u8>,
    /// The last battery save built for JS to read.
    save: Vec<u8>,
    rgba: Vec<u8>,
    palette: [[u8; 4]; 4],
    audio: Vec<f32>,
    sample_rate: u32,
    title: String,
}

impl State {
    fn new() -> Self {
        Self {
            gb: None,
            input: Vec::new(),
            save: Vec::new(),
            rgba: vec![0; WIDTH * HEIGHT * 4],
            palette: [
                [0x9B, 0xBC, 0x0F, 0xFF],
                [0x8B, 0xAC, 0x0F, 0xFF],
                [0x30, 0x62, 0x30, 0xFF],
                [0x0F, 0x38, 0x0F, 0xFF],
            ],
            audio: Vec::new(),
            sample_rate: 48_000,
            title: String::new(),
        }
    }

    fn render(&mut self) {
        let Some(gb) = &self.gb else { return };
        for (px, &shade) in self.rgba.chunks_exact_mut(4).zip(gb.framebuffer().iter()) {
            px.copy_from_slice(&self.palette[usize::from(shade & 3)]);
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Resizes the input buffer to `len` bytes and returns its address.
#[unsafe(no_mangle)]
pub extern "C" fn dm_input_buffer(len: usize) -> *mut u8 {
    with(|s| {
        s.input.clear();
        s.input.resize(len, 0);
        s.input.as_mut_ptr()
    })
}

/// Boots the ROM currently in the input buffer.
#[unsafe(no_mangle)]
pub extern "C" fn dm_load_rom() -> u32 {
    with(|s| {
        let rom = std::mem::take(&mut s.input);
        match GameBoy::with_sample_rate(rom, s.sample_rate) {
            Ok(gb) => {
                s.title = gb.header().title.clone();
                s.gb = Some(gb);
                s.audio.clear();
                s.render();
                0
            }
            Err(CartridgeError::TooSmall(_)) => 1,
            Err(CartridgeError::Unsupported(_)) => 2,
            Err(CartridgeError::CgbOnly) => 3,
        }
    })
}

/// Emulates one video frame and refreshes the RGBA framebuffer. Audio
/// samples accumulate until JS drains them with `dm_audio_clear`.
#[unsafe(no_mangle)]
pub extern "C" fn dm_run_frame() {
    with(|s| {
        let Some(gb) = &mut s.gb else { return };
        gb.run_frame();
        s.audio.extend_from_slice(gb.audio_samples());
        gb.clear_audio();
        // Never let an undrained buffer grow without bound (tab in background).
        let cap = s.sample_rate as usize * 2;
        if s.audio.len() > cap {
            let excess = s.audio.len() - cap;
            s.audio.drain(..excess);
        }
        s.render();
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn dm_framebuffer() -> *const u8 {
    with(|s| s.rgba.as_ptr())
}

/// Sets shade `index` (0 = lightest) to an RGB colour and redraws.
#[unsafe(no_mangle)]
pub extern "C" fn dm_set_palette(index: u32, r: u32, g: u32, b: u32) {
    with(|s| {
        if let Some(slot) = s.palette.get_mut(index as usize) {
            *slot = [r as u8, g as u8, b as u8, 0xFF];
        }
        s.render();
    })
}

/// Buttons: 0 right, 1 left, 2 up, 3 down, 4 A, 5 B, 6 select, 7 start.
#[unsafe(no_mangle)]
pub extern "C" fn dm_set_button(button: u32, pressed: u32) {
    with(|s| {
        if let (Some(gb), Some(&b)) = (&mut s.gb, Button::ALL.get(button as usize)) {
            gb.set_button(b, pressed != 0);
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn dm_set_sample_rate(rate: u32) {
    with(|s| {
        s.sample_rate = rate;
        if let Some(gb) = &mut s.gb {
            gb.set_sample_rate(rate);
        }
    })
}

/// Interleaved stereo f32 samples produced since the last clear.
#[unsafe(no_mangle)]
pub extern "C" fn dm_audio_ptr() -> *const f32 {
    with(|s| s.audio.as_ptr())
}

/// Number of f32 values (not frames) available at `dm_audio_ptr`.
#[unsafe(no_mangle)]
pub extern "C" fn dm_audio_len() -> usize {
    with(|s| s.audio.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn dm_audio_clear() {
    with(|s| s.audio.clear())
}

/// Seconds since the UNIX epoch, from a JavaScript `Date.now() / 1000`.
fn unix_seconds(js: f64) -> u64 {
    // `as` saturates: NaN and negative times become 0, "unknown" to the core.
    js as u64
}

/// Length of the battery save, or 0 if the cartridge has no battery.
#[unsafe(no_mangle)]
pub extern "C" fn dm_save_len() -> usize {
    with(|s| s.gb.as_ref().map_or(0, GameBoy::save_len))
}

/// Length of the cartridge RAM at the start of the battery save, without the
/// RTC footer. The page compares this part to decide when to store the save:
/// the clock catches up from the footer's timestamp, so a running clock alone
/// doesn't need a write.
#[unsafe(no_mangle)]
pub extern "C" fn dm_save_ram_len() -> usize {
    with(|s| {
        s.gb.as_ref().map_or(0, |gb| {
            gb.save_len().saturating_sub(if gb.has_clock() { dotmatrix_core::cartridge::RTC_FOOTER_LEN } else { 0 })
        })
    })
}

/// Builds the battery save, stamped with `unix_time` (seconds), and returns
/// its length; read it from `dm_save_ptr`. 0 if the cartridge has no battery.
#[unsafe(no_mangle)]
pub extern "C" fn dm_save_build(unix_time: f64) -> usize {
    with(|s| {
        s.save = s.gb.as_ref().and_then(|gb| gb.save_data(unix_seconds(unix_time))).unwrap_or_default();
        s.save.len()
    })
}

/// The save built by the last `dm_save_build`.
#[unsafe(no_mangle)]
pub extern "C" fn dm_save_ptr() -> *const u8 {
    with(|s| s.save.as_ptr())
}

/// Restores the battery save in the input buffer; an RTC in it catches up to
/// `unix_time` (seconds). Returns 1 on success.
#[unsafe(no_mangle)]
pub extern "C" fn dm_load_save(unix_time: f64) -> u32 {
    with(|s| {
        let data = std::mem::take(&mut s.input);
        s.gb.as_mut().is_some_and(|gb| gb.load_save_data(&data, unix_seconds(unix_time))) as u32
    })
}

/// Cartridge title (ASCII), see `dm_title_len`.
#[unsafe(no_mangle)]
pub extern "C" fn dm_title_ptr() -> *const u8 {
    with(|s| s.title.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn dm_title_len() -> usize {
    with(|s| s.title.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage_and_survives_calls_without_a_rom() {
        dm_run_frame();
        dm_set_button(4, 1);
        assert_eq!(dm_save_len(), 0);
        assert_eq!(dm_save_build(0.0), 0);
        assert_eq!(dm_save_ram_len(), 0);
        let ptr = dm_input_buffer(16);
        assert!(!ptr.is_null());
        assert_eq!(dm_load_rom(), 1);
    }

    fn write_input(bytes: &[u8]) {
        let ptr = dm_input_buffer(bytes.len());
        // SAFETY: dm_input_buffer just sized the buffer to bytes.len().
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    }

    #[test]
    fn mbc3_save_carries_the_clock_through_the_exports() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x10; // MBC3+TIMER+RAM+BATTERY
        rom[0x149] = 0x02; // 8 KiB RAM
        write_input(&rom);
        assert_eq!(dm_load_rom(), 0);
        assert_eq!(dm_save_len(), 0x2000 + 48);
        assert_eq!(dm_save_ram_len(), 0x2000);

        let saved_at = 1_790_000_000.0;
        let len = dm_save_build(saved_at);
        // SAFETY: dm_save_ptr points at the `len` bytes dm_save_build made.
        let mut save = unsafe { std::slice::from_raw_parts(dm_save_ptr(), len) }.to_vec();
        assert_eq!(&save[0x2000 + 40..], &1_790_000_000u64.to_le_bytes());

        // Reload the save 90 seconds later: the live seconds register reads 30
        // after one full minute carried.
        write_input(&rom);
        assert_eq!(dm_load_rom(), 0);
        save[0x2000] = 0; // live seconds
        write_input(&save);
        assert_eq!(dm_load_save(saved_at + 90.0), 1);
        let rebuilt = dm_save_build(saved_at + 90.0);
        // SAFETY: as above, for the save just rebuilt.
        let after = unsafe { std::slice::from_raw_parts(dm_save_ptr(), rebuilt) };
        assert_eq!((after[0x2000], after[0x2000 + 4]), (30, 1));
    }
}
