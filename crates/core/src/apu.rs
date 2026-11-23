//! Audio registers, without the sound yet.
//!
//! Games poke the APU constantly, so even before there is any audio the
//! registers need to read back the way hardware does: unused bits as 1,
//! write-only fields hidden, wave RAM as plain memory. Sample output comes
//! later; until then `samples()` stays empty.

/// Bits that always read back as 1, for 0xFF10..=0xFF2F.
const READ_MASK: [u8; 0x20] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // NR20-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // NR40-NR44
    0x00, 0x00, 0x70, // NR50-NR52
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

pub struct Apu {
    power: bool,
    /// NR52 channel-active bits. Nothing plays yet, but the boot chime
    /// leaves channel 1 marked active and software can see that.
    status: u8,
    regs: [u8; 0x20],
    wave_ram: [u8; 16],
    sample_rate: u32,
    samples: Vec<f32>,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Self {
        let mut apu =
            Self { power: true, status: 0x01, regs: [0; 0x20], wave_ram: [0; 16], sample_rate, samples: Vec::new() };
        // Register state left by the DMG boot ROM after its chime.
        for (addr, v) in [
            (0xFF10, 0x80),
            (0xFF11, 0xBF),
            (0xFF12, 0xF3),
            (0xFF14, 0x3F),
            (0xFF16, 0x3F),
            (0xFF19, 0x3F),
            (0xFF1A, 0x7F),
            (0xFF1B, 0xFF),
            (0xFF1C, 0x9F),
            (0xFF1E, 0x3F),
            (0xFF20, 0xFF),
            (0xFF23, 0x3F),
            (0xFF24, 0x77),
            (0xFF25, 0xF3),
        ] {
            apu.write(addr, v);
        }
        apu
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate.max(1);
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }

    pub fn tick(&mut self) {}

    pub fn frame_sequencer_step(&mut self) {}

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF26 => 0x70 | u8::from(self.power) << 7 | self.status,
            0xFF10..=0xFF2F => {
                let i = usize::from(addr - 0xFF10);
                self.regs[i] | READ_MASK[i]
            }
            0xFF30..=0xFF3F => self.wave_ram[usize::from(addr - 0xFF30)],
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF30..=0xFF3F => self.wave_ram[usize::from(addr - 0xFF30)] = value,
            0xFF26 => {
                let on = value & 0x80 != 0;
                if self.power && !on {
                    self.regs[..0x16].fill(0);
                    self.status = 0;
                }
                self.power = on;
            }
            0xFF10..=0xFF25 if self.power => self.regs[usize::from(addr - 0xFF10)] = value,
            _ => {}
        }
    }
}
