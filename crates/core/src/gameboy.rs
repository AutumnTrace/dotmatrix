use crate::bus::{Bus, IRQ_JOYPAD};
use crate::cartridge::{Cartridge, CartridgeError, Header};
use crate::cpu::{Cpu, Registers};
use crate::joypad::Button;
use crate::ppu::{HEIGHT, WIDTH};

/// T-cycles per second (the DMG master clock).
pub const CLOCK_HZ: u32 = 4_194_304;
/// T-cycles per video frame: 154 lines of 456 dots.
pub const CYCLES_PER_FRAME: u64 = 70_224;
/// Native refresh rate, about 59.73 Hz.
pub const FRAMES_PER_SECOND: f64 = CLOCK_HZ as f64 / CYCLES_PER_FRAME as f64;

pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// A complete DMG: CPU, bus and everything hanging off it.
pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
    frames: u64,
}

impl GameBoy {
    /// Boots straight into the cartridge with post-boot-ROM register state.
    pub fn new(rom: Vec<u8>) -> Result<Self, CartridgeError> {
        Self::with_sample_rate(rom, DEFAULT_SAMPLE_RATE)
    }

    pub fn with_sample_rate(rom: Vec<u8>, sample_rate: u32) -> Result<Self, CartridgeError> {
        let cart = Cartridge::new(rom)?;
        Ok(Self { cpu: Cpu::new(), bus: Bus::new(cart, sample_rate), frames: 0 })
    }

    pub fn header(&self) -> &Header {
        self.bus.cart.header()
    }

    /// Executes one CPU step (an instruction, an interrupt dispatch, or one
    /// idle M-cycle while halted).
    pub fn step(&mut self) {
        self.cpu.step(&mut self.bus);
    }

    /// Runs until the PPU finishes a frame (start of VBlank). With the LCD
    /// off there is no VBlank, so it stops after one frame's worth of cycles.
    pub fn run_frame(&mut self) {
        let start = self.bus.cycles;
        loop {
            self.cpu.step(&mut self.bus);
            if self.bus.ppu.take_frame_ready() {
                break;
            }
            let elapsed = self.bus.cycles - start;
            if elapsed >= CYCLES_PER_FRAME && (!self.bus.ppu.lcd_enabled() || elapsed >= 2 * CYCLES_PER_FRAME) {
                break;
            }
        }
        self.frames += 1;
    }

    /// Runs for at least `cycles` T-cycles.
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.bus.cycles + cycles;
        while self.bus.cycles < target {
            self.cpu.step(&mut self.bus);
        }
    }

    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// T-cycles since power-on.
    pub fn cycles(&self) -> u64 {
        self.bus.cycles
    }

    /// 160x144 shades, row-major, 0 = lightest .. 3 = darkest.
    pub fn framebuffer(&self) -> &[u8; WIDTH * HEIGHT] {
        self.bus.ppu.framebuffer()
    }

    pub fn set_button(&mut self, button: Button, pressed: bool) {
        if self.bus.joypad.set(button, pressed) {
            self.bus.request_interrupt(IRQ_JOYPAD);
        }
    }

    /// Interleaved stereo f32 samples generated since the last
    /// [`GameBoy::clear_audio`].
    pub fn audio_samples(&self) -> &[f32] {
        self.bus.apu.samples()
    }

    pub fn clear_audio(&mut self) {
        self.bus.apu.clear_samples();
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.bus.apu.set_sample_rate(rate);
    }

    /// Everything written to the serial port so far.
    pub fn serial_output(&self) -> &[u8] {
        self.bus.serial.output()
    }

    pub fn has_battery(&self) -> bool {
        self.bus.cart.has_battery()
    }

    /// Battery-backed cartridge RAM, if the cartridge has a battery.
    pub fn save_data(&self) -> Option<&[u8]> {
        self.has_battery().then(|| self.bus.cart.ram())
    }

    /// Restores battery RAM. Returns false if the size does not match.
    pub fn load_save_data(&mut self, data: &[u8]) -> bool {
        self.bus.cart.load_ram(data)
    }

    pub fn registers(&self) -> Registers {
        self.cpu.regs
    }

    /// Reads memory the way a debugger would: no side effects, and VRAM/OAM
    /// are visible even while the PPU is using them.
    pub fn peek(&self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }

    /// True once the CPU has executed an illegal opcode and locked up.
    pub fn is_locked(&self) -> bool {
        self.cpu.is_locked()
    }
}
