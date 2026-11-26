//! Cartridge header parsing and memory bank controllers.
//!
//! Supported: ROM only, MBC1 (including the "MBC1M" multicart wiring), MBC2,
//! MBC3 (with a real-time clock that counts emulated time while running and
//! catches up on wall-clock time between battery saves) and MBC5.

use std::fmt;

const T_CYCLES_PER_SECOND: u32 = 4_194_304;

/// The RTC footer BGB and VBA-M append to MBC3 battery saves: live then
/// latched registers (seconds, minutes, hours, day low, day high) as ten
/// little-endian u32s, then the UNIX time the save was written as a u64.
pub const RTC_FOOTER_LEN: usize = 48;
/// Older VBA-M builds wrote the timestamp as a u32.
const RTC_FOOTER_LEN_SHORT: usize = 44;
/// Writable bits of the five RTC registers.
const RTC_MASKS: [u8; 5] = [0x3F, 0x3F, 0x1F, 0xFF, 0xC1];
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartridgeError {
    TooSmall(usize),
    Unsupported(u8),
    CgbOnly,
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall(len) => write!(f, "file is {len} bytes, too small to be a Game Boy ROM"),
            Self::Unsupported(kind) => {
                write!(f, "cartridge type 0x{kind:02X} ({}) is not supported", cartridge_type_name(*kind))
            }
            Self::CgbOnly => write!(f, "this is a Game Boy Color-only game; dotmatrix emulates the original DMG"),
        }
    }
}

impl std::error::Error for CartridgeError {}

pub fn cartridge_type_name(kind: u8) -> &'static str {
    match kind {
        0x00 => "ROM ONLY",
        0x01 => "MBC1",
        0x02 => "MBC1+RAM",
        0x03 => "MBC1+RAM+BATTERY",
        0x05 => "MBC2",
        0x06 => "MBC2+BATTERY",
        0x08 => "ROM+RAM",
        0x09 => "ROM+RAM+BATTERY",
        0x0B..=0x0D => "MMM01",
        0x0F => "MBC3+TIMER+BATTERY",
        0x10 => "MBC3+TIMER+RAM+BATTERY",
        0x11 => "MBC3",
        0x12 => "MBC3+RAM",
        0x13 => "MBC3+RAM+BATTERY",
        0x19 => "MBC5",
        0x1A => "MBC5+RAM",
        0x1B => "MBC5+RAM+BATTERY",
        0x1C => "MBC5+RUMBLE",
        0x1D => "MBC5+RUMBLE+RAM",
        0x1E => "MBC5+RUMBLE+RAM+BATTERY",
        0x20 => "MBC6",
        0x22 => "MBC7",
        0xFC => "POCKET CAMERA",
        0xFD => "BANDAI TAMA5",
        0xFE => "HuC3",
        0xFF => "HuC1",
        _ => "unknown",
    }
}

/// Fields from the cartridge header at 0x0100-0x014F that matter here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub cartridge_type: u8,
    pub rom_size: usize,
    pub ram_size: usize,
    pub cgb_flag: u8,
    pub header_checksum_ok: bool,
}

impl Header {
    pub fn parse(rom: &[u8]) -> Result<Self, CartridgeError> {
        if rom.len() < 0x150 {
            return Err(CartridgeError::TooSmall(rom.len()));
        }
        let cgb_flag = rom[0x143];
        // Titles are up to 16 bytes; on newer carts the tail is the
        // manufacturer code and CGB flag, which are not printable.
        let title: String = rom[0x134..0x144]
            .iter()
            .take_while(|&&b| b != 0 && b < 0x80)
            .map(|&b| b as char)
            .collect::<String>()
            .trim()
            .to_string();
        let rom_size = match rom[0x148] {
            n @ 0..=8 => 0x8000 << n,
            _ => rom.len(),
        };
        let ram_size = match rom[0x149] {
            2 => 0x2000,
            3 => 0x8000,
            4 => 0x20000,
            5 => 0x10000,
            _ => 0,
        };
        let mut sum: u8 = 0;
        for &b in &rom[0x134..=0x14C] {
            sum = sum.wrapping_sub(b).wrapping_sub(1);
        }
        Ok(Self {
            title,
            cartridge_type: rom[0x147],
            rom_size,
            ram_size,
            cgb_flag,
            header_checksum_ok: sum == rom[0x14D],
        })
    }
}

#[derive(Clone, Debug, Default)]
struct Rtc {
    /// Live registers: seconds, minutes, hours, day low, day high/flags.
    live: [u8; 5],
    latched: [u8; 5],
    latch_armed: bool,
    subsecond: u32,
}

impl Rtc {
    fn halted(&self) -> bool {
        self.live[4] & 0x40 != 0
    }

    fn tick(&mut self, t_cycles: u32) {
        if self.halted() {
            return;
        }
        self.subsecond += t_cycles;
        while self.subsecond >= T_CYCLES_PER_SECOND {
            self.subsecond -= T_CYCLES_PER_SECOND;
            self.advance_second();
        }
    }

    fn advance_second(&mut self) {
        // Registers are 6/6/5/9 bits wide; out-of-range values written by
        // software count up to the bit width and wrap to 0, like hardware.
        self.live[0] = (self.live[0] + 1) & 0x3F;
        if self.live[0] != 60 {
            return;
        }
        self.live[0] = 0;
        self.live[1] = (self.live[1] + 1) & 0x3F;
        if self.live[1] != 60 {
            return;
        }
        self.live[1] = 0;
        self.live[2] = (self.live[2] + 1) & 0x1F;
        if self.live[2] != 24 {
            return;
        }
        self.live[2] = 0;
        let day = (u16::from(self.live[4] & 1) << 8 | u16::from(self.live[3])) + 1;
        self.live[3] = day as u8;
        self.live[4] = (self.live[4] & 0xFE) | ((day >> 8) as u8 & 1);
        if day > 0x1FF {
            self.live[4] |= 0x80; // day counter carry, sticky until cleared
        }
    }

    fn write(&mut self, reg: u8, value: u8) {
        let i = usize::from(reg - 0x08);
        self.live[i] = value & RTC_MASKS[i];
        if i == 0 {
            self.subsecond = 0;
        }
    }

    /// Runs the clock forward by `seconds` of wall-clock time, as if the
    /// cartridge had sat on a shelf with its battery in. A halted clock stays
    /// put.
    fn catch_up(&mut self, mut seconds: u64) {
        if self.halted() {
            return;
        }
        // Software can write out-of-range values (63 seconds, 31 hours), which
        // count up to the register width and wrap without carrying. Step
        // through those one second at a time; from then on it's arithmetic.
        while seconds > 0 && !(self.live[0] < 60 && self.live[1] < 60 && self.live[2] < 24) {
            self.advance_second();
            seconds -= 1;
        }
        if seconds == 0 {
            return;
        }
        // A full 512-day cycle always overflows the day counter; drop whole
        // cycles so the sum below stays small.
        const CYCLE: u64 = 512 * SECONDS_PER_DAY;
        if seconds >= CYCLE {
            self.live[4] |= 0x80;
            seconds %= CYCLE;
        }
        let day = u64::from(self.live[4] & 1) << 8 | u64::from(self.live[3]);
        let now = day * SECONDS_PER_DAY
            + u64::from(self.live[2]) * 3600
            + u64::from(self.live[1]) * 60
            + u64::from(self.live[0])
            + seconds;
        let days = now / SECONDS_PER_DAY;
        self.live[0] = (now % 60) as u8;
        self.live[1] = (now / 60 % 60) as u8;
        self.live[2] = (now / 3600 % 24) as u8;
        self.live[3] = days as u8;
        self.live[4] = (self.live[4] & 0xFE) | ((days >> 8) as u8 & 1);
        if days > 0x1FF {
            self.live[4] |= 0x80;
        }
    }

    fn footer(&self, unix_time: u64) -> [u8; RTC_FOOTER_LEN] {
        let mut out = [0; RTC_FOOTER_LEN];
        for (i, &value) in self.live.iter().chain(&self.latched).enumerate() {
            out[i * 4] = value;
        }
        out[40..].copy_from_slice(&unix_time.to_le_bytes());
        out
    }

    /// Parses a 44- or 48-byte footer into a clock and the time it was saved.
    fn from_footer(bytes: &[u8]) -> (Self, u64) {
        let reg = |i: usize| bytes[i * 4] & RTC_MASKS[i % 5];
        let mut rtc = Self::default();
        for i in 0..5 {
            rtc.live[i] = reg(i);
            rtc.latched[i] = reg(i + 5);
        }
        let tail = &bytes[40..];
        let saved_at = match <[u8; 8]>::try_from(tail) {
            Ok(t) => u64::from_le_bytes(t),
            Err(_) => <[u8; 4]>::try_from(tail).map_or(0, |t| u64::from(u32::from_le_bytes(t))),
        };
        (rtc, saved_at)
    }

    fn latch_write(&mut self, value: u8) {
        if value == 0 {
            self.latch_armed = true;
        } else {
            if value == 1 && self.latch_armed {
                self.latched = self.live;
            }
            self.latch_armed = false;
        }
    }
}

#[derive(Clone, Debug)]
enum Mbc {
    None,
    Mbc1 { bank1: u8, bank2: u8, mode: bool, multicart: bool },
    Mbc2 { bank: u8 },
    Mbc3 { rom_bank: u8, ram_select: u8, rtc: Option<Box<Rtc>> },
    Mbc5 { rom_bank: u16, ram_bank: u8 },
}

#[derive(Clone)]
pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    header: Header,
    mbc: Mbc,
    ram_enabled: bool,
    battery: bool,
    rom_banks: usize,
    /// Byte offsets of the two 16 KiB ROM windows and the 8 KiB RAM window,
    /// recomputed on every bank switch so reads are a single index.
    rom0_offset: usize,
    romx_offset: usize,
    ram_offset: usize,
}

impl Cartridge {
    pub fn new(mut rom: Vec<u8>) -> Result<Self, CartridgeError> {
        let header = Header::parse(&rom)?;
        if header.cgb_flag == 0xC0 {
            return Err(CartridgeError::CgbOnly);
        }
        // Pad to a power of two of at least 32 KiB so bank masking just works.
        let padded = rom.len().max(0x8000).next_power_of_two();
        rom.resize(padded, 0xFF);
        let kind = header.cartridge_type;
        let (mbc, battery, ram_size) = match kind {
            0x00 => (Mbc::None, false, 0),
            0x08 | 0x09 => (Mbc::None, kind == 0x09, header.ram_size.max(0x2000)),
            0x01..=0x03 => {
                let multicart = detect_mbc1_multicart(&rom);
                let mbc = Mbc::Mbc1 { bank1: 1, bank2: 0, mode: false, multicart };
                let ram = if kind == 0x01 { 0 } else { header.ram_size };
                (mbc, kind == 0x03, ram)
            }
            // MBC2 has 512 half-bytes of RAM built into the mapper.
            0x05 | 0x06 => (Mbc::Mbc2 { bank: 1 }, kind == 0x06, 512),
            0x0F..=0x13 => {
                let rtc = matches!(kind, 0x0F | 0x10).then(Box::<Rtc>::default);
                let ram = if matches!(kind, 0x0F | 0x11) { 0 } else { header.ram_size };
                (Mbc::Mbc3 { rom_bank: 1, ram_select: 0, rtc }, matches!(kind, 0x0F | 0x10 | 0x13), ram)
            }
            0x19..=0x1E => {
                let ram = if matches!(kind, 0x19 | 0x1C) { 0 } else { header.ram_size };
                (Mbc::Mbc5 { rom_bank: 1, ram_bank: 0 }, matches!(kind, 0x1B | 0x1E), ram)
            }
            other => return Err(CartridgeError::Unsupported(other)),
        };
        let rom_banks = rom.len() / 0x4000;
        let mut cart = Self {
            rom,
            ram: vec![0; ram_size],
            header,
            mbc,
            ram_enabled: false,
            battery,
            rom_banks,
            rom0_offset: 0,
            romx_offset: 0x4000,
            ram_offset: 0,
        };
        cart.update_offsets();
        Ok(cart)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// True if the cartridge keeps anything across power cycles: battery RAM,
    /// an MBC3 clock, or both.
    pub fn has_battery(&self) -> bool {
        self.battery && (!self.ram.is_empty() || self.rtc().is_some())
    }

    /// Cartridge RAM contents.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// True for MBC3 carts with a real-time clock.
    pub fn has_clock(&self) -> bool {
        self.rtc().is_some()
    }

    fn rtc(&self) -> Option<&Rtc> {
        match &self.mbc {
            Mbc::Mbc3 { rtc: Some(rtc), .. } => Some(rtc),
            _ => None,
        }
    }

    /// Length of [`Cartridge::battery_save`], or 0 without a battery.
    pub fn battery_save_len(&self) -> usize {
        if !self.has_battery() {
            return 0;
        }
        self.ram.len() + if self.rtc().is_some() { RTC_FOOTER_LEN } else { 0 }
    }

    /// A battery save in the layout BGB and VBA-M use: cartridge RAM, then,
    /// for MBC3 carts with a clock, the 48-byte RTC footer stamped with
    /// `unix_time` (wall-clock seconds) so a later load can catch up. Empty
    /// without a battery.
    pub fn battery_save(&self, unix_time: u64) -> Vec<u8> {
        if !self.has_battery() {
            return Vec::new();
        }
        let mut out = self.ram.clone();
        if let Some(rtc) = self.rtc() {
            out.extend_from_slice(&rtc.footer(unix_time));
        }
        out
    }

    /// Restores a battery save. Accepts cartridge RAM alone (older saves, or
    /// an emulator that drops the clock) or RAM followed by a 48- or 44-byte
    /// RTC footer, after which the clock catches up on the time between the
    /// footer's timestamp and `unix_time`. A clock footer on a cart without a
    /// clock is ignored. Returns false and changes nothing if the size fits
    /// none of these.
    pub fn load_battery_save(&mut self, data: &[u8], unix_time: u64) -> bool {
        let ram_len = self.ram.len();
        let footer = match data.len().checked_sub(ram_len) {
            Some(0) => None,
            Some(RTC_FOOTER_LEN | RTC_FOOTER_LEN_SHORT) => Some(&data[ram_len..]),
            _ => return false,
        };
        self.ram.copy_from_slice(&data[..ram_len]);
        if let (Some(footer), Mbc::Mbc3 { rtc: Some(rtc), .. }) = (footer, &mut self.mbc) {
            let (mut restored, saved_at) = Rtc::from_footer(footer);
            // A zero timestamp means the writer didn't know the time; a
            // timestamp in the future means the host clock went back. Neither
            // says how long the cart was away, so the clock just resumes.
            if saved_at != 0 {
                restored.catch_up(unix_time.saturating_sub(saved_at));
            }
            **rtc = restored;
        }
        true
    }

    fn update_offsets(&mut self) {
        let banks = self.rom_banks;
        let (rom0, romx) = match &self.mbc {
            Mbc::None => (0, 1),
            Mbc::Mbc1 { bank1, bank2, mode, multicart } => {
                let (low, shift) = if *multicart { (bank1 & 0x0F, 4) } else { (*bank1, 5) };
                let high = usize::from(*bank2) << shift;
                let rom0 = if *mode { high } else { 0 };
                (rom0, high | usize::from(low))
            }
            Mbc::Mbc2 { bank } => (0, usize::from(*bank)),
            Mbc::Mbc3 { rom_bank, .. } => (0, usize::from(*rom_bank)),
            Mbc::Mbc5 { rom_bank, .. } => (0, usize::from(*rom_bank)),
        };
        self.rom0_offset = (rom0 % banks) * 0x4000;
        self.romx_offset = (romx % banks) * 0x4000;

        let ram_bank = match &self.mbc {
            Mbc::Mbc1 { bank2, mode: true, .. } => usize::from(*bank2),
            Mbc::Mbc3 { ram_select, .. } if *ram_select < 4 => usize::from(*ram_select),
            Mbc::Mbc5 { ram_bank, .. } => usize::from(*ram_bank),
            _ => 0,
        };
        let ram_banks = (self.ram.len() / 0x2000).max(1);
        self.ram_offset = (ram_bank % ram_banks) * 0x2000;
    }

    pub fn read_rom(&self, addr: u16) -> u8 {
        let a = usize::from(addr);
        if a < 0x4000 { self.rom[self.rom0_offset + a] } else { self.rom[self.romx_offset + (a & 0x3FFF)] }
    }

    pub fn write_rom(&mut self, addr: u16, value: u8) {
        match &mut self.mbc {
            Mbc::None => return,
            Mbc::Mbc1 { bank1, bank2, mode, .. } => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x3FFF => *bank1 = (value & 0x1F).max(1),
                0x4000..=0x5FFF => *bank2 = value & 0x03,
                _ => *mode = value & 1 != 0,
            },
            Mbc::Mbc2 { bank } => {
                if addr < 0x4000 {
                    // Address bit 8 selects between RAM enable and ROM bank.
                    if addr & 0x0100 == 0 {
                        self.ram_enabled = value & 0x0F == 0x0A;
                    } else {
                        *bank = (value & 0x0F).max(1);
                    }
                }
            }
            Mbc::Mbc3 { rom_bank, ram_select, rtc } => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
                // 7 bits on a stock MBC3; MBC30 carts (4 MiB) use all 8.
                0x2000..=0x3FFF => *rom_bank = value.max(1),
                0x4000..=0x5FFF => *ram_select = value & 0x0F,
                _ => {
                    if let Some(rtc) = rtc {
                        rtc.latch_write(value);
                    }
                }
            },
            Mbc::Mbc5 { rom_bank, ram_bank } => match addr {
                0x0000..=0x1FFF => self.ram_enabled = value == 0x0A,
                0x2000..=0x2FFF => *rom_bank = (*rom_bank & 0x100) | u16::from(value),
                0x3000..=0x3FFF => *rom_bank = (*rom_bank & 0xFF) | (u16::from(value & 1) << 8),
                0x4000..=0x5FFF => *ram_bank = value & 0x0F,
                _ => {}
            },
        }
        self.update_offsets();
    }

    pub fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled {
            return 0xFF;
        }
        let a = usize::from(addr & 0x1FFF);
        match &self.mbc {
            Mbc::Mbc2 { .. } => self.ram[a & 0x1FF] | 0xF0,
            Mbc::Mbc3 { ram_select: sel @ 0x08..=0x0C, rtc: Some(rtc), .. } => rtc.latched[usize::from(*sel - 0x08)],
            _ if self.ram.is_empty() => 0xFF,
            _ => self.ram[(self.ram_offset + a) % self.ram.len()],
        }
    }

    pub fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled {
            return;
        }
        let a = usize::from(addr & 0x1FFF);
        match &mut self.mbc {
            Mbc::Mbc2 { .. } => self.ram[a & 0x1FF] = value & 0x0F,
            Mbc::Mbc3 { ram_select: sel @ 0x08..=0x0C, rtc: Some(rtc), .. } => rtc.write(*sel, value),
            _ if self.ram.is_empty() => {}
            _ => {
                let len = self.ram.len();
                self.ram[(self.ram_offset + a) % len] = value;
            }
        }
    }

    /// Advances the MBC3 real-time clock.
    pub fn tick(&mut self, t_cycles: u32) {
        if let Mbc::Mbc3 { rtc: Some(rtc), .. } = &mut self.mbc {
            rtc.tick(t_cycles);
        }
    }
}

/// MBC1 multicarts (e.g. "Mortal Kombat I & II") wire the upper bank bits one
/// position lower. They can only be told apart by finding a second Nintendo
/// logo at the start of bank 0x10.
fn detect_mbc1_multicart(rom: &[u8]) -> bool {
    if rom.len() != 0x100000 {
        return false;
    }
    let logo = &rom[0x104..0x134];
    rom[0x40104..0x40134] == *logo
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a ROM where every 16 KiB bank starts with its own bank number.
    fn banked_rom(kind: u8, banks: usize, ram_code: u8) -> Vec<u8> {
        let mut rom = vec![0u8; banks * 0x4000];
        for bank in 0..banks {
            rom[bank * 0x4000] = bank as u8;
            rom[bank * 0x4000 + 1] = (bank >> 8) as u8;
        }
        rom[0x147] = kind;
        rom[0x148] = (banks / 2).trailing_zeros() as u8;
        rom[0x149] = ram_code;
        rom
    }

    fn romx_bank(cart: &Cartridge) -> usize {
        usize::from(cart.read_rom(0x4000)) | usize::from(cart.read_rom(0x4001)) << 8
    }

    #[test]
    fn mbc1_bank_zero_maps_to_one_and_upper_bits_combine() {
        let mut cart = Cartridge::new(banked_rom(0x01, 128, 0)).unwrap();
        assert_eq!(romx_bank(&cart), 1);
        cart.write_rom(0x2000, 0x00);
        assert_eq!(romx_bank(&cart), 1, "bank 0 is remapped to 1");
        cart.write_rom(0x2000, 0x20); // only the low five bits count
        assert_eq!(romx_bank(&cart), 1);
        cart.write_rom(0x2000, 0x05);
        cart.write_rom(0x4000, 0x02);
        assert_eq!(romx_bank(&cart), 0x45);
        // Mode 0: the 0x0000 window is always bank 0.
        assert_eq!(cart.read_rom(0x0000), 0);
        // Mode 1: the 0x0000 window follows the upper bits too.
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_rom(0x0000), 0x40);
    }

    #[test]
    fn mbc1_ram_needs_enable_and_banks_in_mode_1() {
        let mut cart = Cartridge::new(banked_rom(0x03, 4, 3)).unwrap();
        cart.write_ram(0xA000, 0x12);
        assert_eq!(cart.read_ram(0xA000), 0xFF, "RAM is disabled after reset");
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA000, 0x12);
        cart.write_rom(0x6000, 0x01);
        cart.write_rom(0x4000, 0x02);
        cart.write_ram(0xA000, 0x34);
        assert_eq!(cart.read_ram(0xA000), 0x34);
        cart.write_rom(0x4000, 0x00);
        assert_eq!(cart.read_ram(0xA000), 0x12);
        assert!(cart.has_battery());
        assert_eq!(cart.ram()[0x4000], 0x34);
    }

    #[test]
    fn mbc5_uses_nine_bit_bank_and_allows_bank_zero() {
        let mut cart = Cartridge::new(banked_rom(0x19, 512, 0)).unwrap();
        cart.write_rom(0x2000, 0x00);
        assert_eq!(romx_bank(&cart), 0);
        cart.write_rom(0x2000, 0x34);
        cart.write_rom(0x3000, 0x01);
        assert_eq!(romx_bank(&cart), 0x134);
    }

    #[test]
    fn mbc3_rtc_latches_and_counts_emulated_time() {
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x08);
        cart.write_ram(0xA000, 58);
        cart.tick(T_CYCLES_PER_SECOND * 3);
        assert_eq!(cart.read_ram(0xA000), 0, "unlatched value is still zero");
        cart.write_rom(0x6000, 0x00);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_ram(0xA000), 1, "58 + 3 seconds wraps to 1");
        cart.write_rom(0x4000, 0x09);
        assert_eq!(cart.read_ram(0xA000), 1, "minutes carried");
    }

    /// Selects an RTC register, writes it, and leaves RAM enabled.
    fn set_rtc(cart: &mut Cartridge, reg: u8, value: u8) {
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, reg);
        cart.write_ram(0xA000, value);
    }

    /// Latches the clock and reads all five registers.
    fn read_rtc(cart: &mut Cartridge) -> [u8; 5] {
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x6000, 0x00);
        cart.write_rom(0x6000, 0x01);
        std::array::from_fn(|i| {
            cart.write_rom(0x4000, 0x08 + i as u8);
            cart.read_ram(0xA000)
        })
    }

    const SAVED_AT: u64 = 1_790_000_000;

    #[test]
    fn mbc3_battery_save_appends_the_48_byte_rtc_footer() {
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        set_rtc(&mut cart, 0x08, 30);
        set_rtc(&mut cart, 0x09, 15);
        set_rtc(&mut cart, 0x0A, 9);
        set_rtc(&mut cart, 0x0B, 0x2C);
        set_rtc(&mut cart, 0x0C, 0x01); // day 0x12C = 300
        cart.write_rom(0x4000, 0x00);
        cart.write_ram(0xA000, 0x5A);
        read_rtc(&mut cart);

        let save = cart.battery_save(SAVED_AT);
        assert_eq!(save.len(), 0x8000 + 48);
        assert_eq!(cart.battery_save_len(), save.len());
        assert_eq!(save[0], 0x5A);
        let footer = &save[0x8000..];
        // Live then latched, one register per little-endian u32.
        assert_eq!(&footer[..20], &[30, 0, 0, 0, 15, 0, 0, 0, 9, 0, 0, 0, 0x2C, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&footer[..20], &footer[20..40]);
        assert_eq!(&footer[40..], &SAVED_AT.to_le_bytes());
    }

    #[test]
    fn mbc3_save_round_trips_and_catches_up_on_wall_clock_time() {
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        set_rtc(&mut cart, 0x0A, 23);
        set_rtc(&mut cart, 0x09, 59);
        set_rtc(&mut cart, 0x08, 50);
        let save = cart.battery_save(SAVED_AT);

        let mut same = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(same.load_battery_save(&save, SAVED_AT));
        assert_eq!(read_rtc(&mut same), [50, 59, 23, 0, 0]);

        // 10 seconds later the day rolls over; 3 days and 1 hour later too.
        let mut later = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(later.load_battery_save(&save, SAVED_AT + 10));
        assert_eq!(read_rtc(&mut later), [0, 0, 0, 1, 0]);
        let mut away = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(away.load_battery_save(&save, SAVED_AT + 3 * 86_400 + 3600));
        assert_eq!(read_rtc(&mut away), [50, 59, 0, 4, 0]);

        // A host clock that went backwards leaves the clock where it was.
        let mut back = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(back.load_battery_save(&save, SAVED_AT - 100));
        assert_eq!(read_rtc(&mut back), [50, 59, 23, 0, 0]);
    }

    #[test]
    fn rtc_catch_up_sets_the_day_carry_and_respects_halt() {
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        set_rtc(&mut cart, 0x0B, 0xFF);
        set_rtc(&mut cart, 0x0C, 0x01); // day 511
        let save = cart.battery_save(SAVED_AT);
        let mut next = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(next.load_battery_save(&save, SAVED_AT + 86_400));
        assert_eq!(read_rtc(&mut next), [0, 0, 0, 0, 0x80], "day 512 wraps to 0 with the carry set");

        let mut long = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(long.load_battery_save(&save, SAVED_AT + 2000 * 86_400));
        // 511 + 2000 days = 2511 = 4 * 512 + 463.
        assert_eq!(read_rtc(&mut long), [0, 0, 0, (463 & 0xFF) as u8, 0x81]);

        set_rtc(&mut cart, 0x0C, 0x41); // halted
        let halted = cart.battery_save(SAVED_AT);
        let mut stays = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(stays.load_battery_save(&halted, SAVED_AT + 86_400));
        assert_eq!(read_rtc(&mut stays), [0, 0, 0, 0xFF, 0x41]);
    }

    #[test]
    fn rtc_catch_up_wraps_out_of_range_values_like_hardware() {
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        set_rtc(&mut cart, 0x08, 62); // counts 62, 63, 0 without carrying
        let save = cart.battery_save(SAVED_AT);
        let mut stepped = cart.clone();
        stepped.tick(T_CYCLES_PER_SECOND * 65);
        let mut loaded = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(loaded.load_battery_save(&save, SAVED_AT + 65));
        assert_eq!(read_rtc(&mut loaded), read_rtc(&mut stepped));
        // Two seconds to wrap to 0, then 63 more: 1 minute 3 seconds.
        assert_eq!(read_rtc(&mut loaded), [3, 1, 0, 0, 0]);
    }

    #[test]
    fn loads_saves_without_a_footer_and_the_short_vba_footer() {
        let mut ram = vec![0u8; 0x8000];
        ram[0x1234] = 0x77;
        let mut cart = Cartridge::new(banked_rom(0x10, 4, 3)).unwrap();
        assert!(cart.load_battery_save(&ram, SAVED_AT), "RAM-only saves from before the footer still load");
        assert_eq!(cart.ram()[0x1234], 0x77);
        assert_eq!(read_rtc(&mut cart), [0; 5]);

        // 44 bytes: ten registers and a 32-bit timestamp.
        let mut short = ram.clone();
        for value in [10u32, 0, 0, 0, 0, 10, 0, 0, 0, 0] {
            short.extend_from_slice(&value.to_le_bytes());
        }
        short.extend_from_slice(&(SAVED_AT as u32).to_le_bytes());
        assert!(cart.load_battery_save(&short, SAVED_AT + 5));
        assert_eq!(read_rtc(&mut cart), [15, 0, 0, 0, 0]);

        assert!(!cart.load_battery_save(&ram[..0x2000], SAVED_AT), "wrong size is refused");
        assert!(!cart.load_battery_save(&[ram.as_slice(), &[0; 47]].concat(), SAVED_AT));
        assert_eq!(cart.ram()[0x1234], 0x77, "a refused save changes nothing");
    }

    #[test]
    fn clock_only_and_clockless_carts() {
        // MBC3+TIMER+BATTERY with no RAM: the save is just the footer.
        let mut clock_only = Cartridge::new(banked_rom(0x0F, 4, 0)).unwrap();
        assert!(clock_only.has_battery());
        set_rtc(&mut clock_only, 0x09, 7);
        let save = clock_only.battery_save(SAVED_AT);
        assert_eq!(save.len(), 48);
        let mut restored = Cartridge::new(banked_rom(0x0F, 4, 0)).unwrap();
        assert!(restored.load_battery_save(&save, SAVED_AT + 60));
        assert_eq!(read_rtc(&mut restored)[1], 8);

        // MBC3+RAM+BATTERY has no clock: no footer, and a stray one is ignored.
        let mut plain = Cartridge::new(banked_rom(0x13, 4, 3)).unwrap();
        assert_eq!(plain.battery_save(SAVED_AT).len(), 0x8000);
        assert!(plain.load_battery_save(&[vec![0; 0x8000], vec![0; 48]].concat(), SAVED_AT));
        // Plain MBC3: no battery at all.
        let no_battery = Cartridge::new(banked_rom(0x11, 4, 0)).unwrap();
        assert!(!no_battery.has_battery());
        assert!(no_battery.battery_save(SAVED_AT).is_empty());
        assert_eq!(no_battery.battery_save_len(), 0);
    }

    #[test]
    fn rejects_cgb_only_and_unknown_mappers() {
        let mut rom = banked_rom(0x00, 2, 0);
        rom[0x143] = 0xC0;
        assert!(matches!(Cartridge::new(rom), Err(CartridgeError::CgbOnly)));
        let rom = banked_rom(0x22, 2, 0);
        assert!(matches!(Cartridge::new(rom), Err(CartridgeError::Unsupported(0x22))));
    }
}
