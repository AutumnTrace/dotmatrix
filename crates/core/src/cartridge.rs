//! Cartridge header parsing and memory bank controllers.
//!
//! Supported so far: ROM only (optionally with RAM) and MBC1, including the
//! "MBC1M" multicart wiring.

use std::fmt;

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

#[derive(Clone, Debug)]
enum Mbc {
    None,
    Mbc1 { bank1: u8, bank2: u8, mode: bool, multicart: bool },
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

    pub fn has_battery(&self) -> bool {
        self.battery && !self.ram.is_empty()
    }

    /// Cartridge RAM contents, for battery saves.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Restores battery RAM. Returns false (and changes nothing) if the size
    /// does not match this cartridge.
    pub fn load_ram(&mut self, data: &[u8]) -> bool {
        if data.len() != self.ram.len() {
            return false;
        }
        self.ram.copy_from_slice(data);
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
        };
        self.rom0_offset = (rom0 % banks) * 0x4000;
        self.romx_offset = (romx % banks) * 0x4000;

        let ram_bank = match &self.mbc {
            Mbc::Mbc1 { bank2, mode: true, .. } => usize::from(*bank2),
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
        }
        self.update_offsets();
    }

    pub fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xFF;
        }
        self.ram[(self.ram_offset + usize::from(addr & 0x1FFF)) % self.ram.len()]
    }

    pub fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let len = self.ram.len();
        self.ram[(self.ram_offset + usize::from(addr & 0x1FFF)) % len] = value;
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
    fn rejects_cgb_only_and_unknown_mappers() {
        let mut rom = banked_rom(0x00, 2, 0);
        rom[0x143] = 0xC0;
        assert!(matches!(Cartridge::new(rom), Err(CartridgeError::CgbOnly)));
        let rom = banked_rom(0x22, 2, 0);
        assert!(matches!(Cartridge::new(rom), Err(CartridgeError::Unsupported(0x22))));
    }
}
