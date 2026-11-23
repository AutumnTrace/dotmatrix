//! Memory map, I/O dispatch, and the per-M-cycle clock for every component.

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::serial::Serial;
use crate::timer::Timer;

pub const IRQ_VBLANK: u8 = 0x01;
pub const IRQ_STAT: u8 = 0x02;
pub const IRQ_TIMER: u8 = 0x04;
pub const IRQ_SERIAL: u8 = 0x08;
pub const IRQ_JOYPAD: u8 = 0x10;

#[derive(Clone, Debug, Default)]
struct OamDma {
    register: u8,
    active: bool,
    source: u16,
    index: u8,
    /// A newly requested transfer waits one M-cycle before it starts copying.
    /// A transfer that is already running keeps going during that time.
    pending: Option<(u16, u8)>,
}

pub struct Bus {
    pub cart: Cartridge,
    wram: Box<[u8; 0x2000]>,
    hram: [u8; 0x7F],
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub joypad: Joypad,
    pub serial: Serial,
    pub ie: u8,
    pub if_reg: u8,
    dma: OamDma,
    /// T-cycles since power-on.
    pub cycles: u64,
}

impl Bus {
    pub fn new(cart: Cartridge, sample_rate: u32) -> Self {
        Self {
            cart,
            wram: Box::new([0; 0x2000]),
            hram: [0; 0x7F],
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate),
            timer: Timer::new(),
            joypad: Joypad::default(),
            serial: Serial::default(),
            ie: 0,
            if_reg: 0x01,
            dma: OamDma { register: 0xFF, ..Default::default() },
            cycles: 0,
        }
    }

    pub fn pending_interrupts(&self) -> u8 {
        self.ie & self.if_reg & 0x1F
    }

    pub fn request_interrupt(&mut self, mask: u8) {
        self.if_reg |= mask;
    }

    /// Advances every component by one M-cycle (4 T-cycles).
    pub fn tick(&mut self) {
        self.cycles += 4;
        self.step_dma();

        let before = self.timer.counter;
        if self.timer.tick() {
            self.if_reg |= IRQ_TIMER;
        }
        let after = self.timer.counter;
        // The APU frame sequencer is clocked by DIV bit 4 (counter bit 12).
        if before & !after & 0x1000 != 0 {
            self.apu.frame_sequencer_step();
        }
        if self.serial.tick(before, after) {
            self.if_reg |= IRQ_SERIAL;
        }

        self.if_reg |= self.ppu.tick();
        self.apu.tick();
        self.cart.tick(4);
    }

    fn step_dma(&mut self) {
        // OAM stays locked through the M-cycle that copies the last byte and
        // frees up on the next one.
        if self.dma.active && self.dma.index == 160 {
            self.dma.active = false;
        }
        if let Some((source, delay)) = self.dma.pending {
            if delay == 0 {
                self.dma.pending = None;
                self.dma.active = true;
                self.dma.source = source;
                self.dma.index = 0;
            } else {
                self.dma.pending = Some((source, delay - 1));
            }
        }
        if self.dma.active {
            let i = self.dma.index;
            let byte = self.dma_read(self.dma.source + u16::from(i));
            self.ppu.oam[usize::from(i)] = byte;
            self.dma.index += 1;
        }
    }

    /// OAM DMA reads bypass PPU access restrictions. Sources at 0xE000 and
    /// above end up in work RAM on DMG.
    fn dma_read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.vram[usize::from(addr & 0x1FFF)],
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            _ => self.wram[usize::from(addr & 0x1FFF)],
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.read_vram(addr),
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xFDFF => self.wram[usize::from(addr & 0x1FFF)],
            0xFE00..=0xFE9F => {
                if self.dma.active {
                    0xFF
                } else {
                    self.ppu.read_oam(addr)
                }
            }
            0xFEA0..=0xFEFF => {
                if self.dma.active || self.ppu.oam_blocked() {
                    0xFF
                } else {
                    0x00
                }
            }
            0xFF00 => self.joypad.read(),
            0xFF01 | 0xFF02 => self.serial.read(addr),
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.if_reg | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF46 => self.dma.register,
            0xFF40..=0xFF4B => self.ppu.read_register(addr),
            0xFF80..=0xFFFE => self.hram[usize::from(addr - 0xFF80)],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.cart.write_rom(addr, value),
            0x8000..=0x9FFF => self.ppu.write_vram(addr, value),
            0xA000..=0xBFFF => self.cart.write_ram(addr, value),
            0xC000..=0xFDFF => self.wram[usize::from(addr & 0x1FFF)] = value,
            0xFE00..=0xFE9F => {
                if !self.dma.active {
                    self.ppu.write_oam(addr, value);
                }
            }
            0xFEA0..=0xFEFF => {}
            0xFF00 => {
                if self.joypad.write(value) {
                    self.if_reg |= IRQ_JOYPAD;
                }
            }
            0xFF01 | 0xFF02 => self.serial.write(addr, value),
            0xFF04 => {
                let before = self.timer.counter;
                self.timer.write_div();
                // Resetting the counter is a falling edge for DIV bit 4 too.
                if before & 0x1000 != 0 {
                    self.apu.frame_sequencer_step();
                }
            }
            0xFF05..=0xFF07 => self.timer.write(addr, value),
            0xFF0F => self.if_reg = value & 0x1F,
            0xFF10..=0xFF3F => self.apu.write(addr, value),
            0xFF46 => {
                self.dma.register = value;
                self.dma.pending = Some((u16::from(value) << 8, 1));
            }
            0xFF40..=0xFF4B => self.if_reg |= self.ppu.write_register(addr, value),
            0xFF80..=0xFFFE => self.hram[usize::from(addr - 0xFF80)] = value,
            0xFFFF => self.ie = value,
            _ => {}
        }
    }

    /// Debugger view of memory: no side effects and no PPU/DMA blocking.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.ppu.vram[usize::from(addr & 0x1FFF)],
            0xFE00..=0xFE9F => self.ppu.oam[usize::from(addr & 0xFF)],
            _ => self.read(addr),
        }
    }
}
