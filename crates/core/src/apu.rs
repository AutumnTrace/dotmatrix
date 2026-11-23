//! Audio processing unit: two pulse channels (the first with frequency
//! sweep), a wave channel and a noise channel.
//!
//! Channels are clocked in T-cycles. The 512 Hz frame sequencer that drives
//! length counters, sweep and volume envelopes is not a free-running timer:
//! it is clocked by the falling edge of DIV bit 4, which is why the bus calls
//! [`Apu::frame_sequencer_step`] from the timer side. Output is box-filtered
//! down to the host sample rate and passed through a DC-blocking high-pass,
//! like the capacitor on the real board.

const CPU_HZ: f64 = 4_194_304.0;

const DUTY: [[u8; 8]; 4] =
    [[0, 0, 0, 0, 0, 0, 0, 1], [1, 0, 0, 0, 0, 0, 0, 1], [1, 0, 0, 0, 0, 1, 1, 1], [0, 1, 1, 1, 1, 1, 1, 0]];

/// Bits that always read back as 1, for 0xFF10..=0xFF2F.
const READ_MASK: [u8; 0x20] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // NR20-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // NR40-NR44
    0x00, 0x00, 0x70, // NR50-NR52
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

#[derive(Clone, Default)]
struct Length {
    counter: u16,
    enabled: bool,
}

impl Length {
    /// Returns true if the channel should be switched off.
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            return self.counter == 0;
        }
        false
    }
}

#[derive(Clone, Default)]
struct Envelope {
    initial: u8,
    up: bool,
    period: u8,
    timer: u8,
    volume: u8,
}

impl Envelope {
    fn write(&mut self, v: u8) {
        self.initial = v >> 4;
        self.up = v & 0x08 != 0;
        self.period = v & 0x07;
    }

    fn trigger(&mut self) {
        self.timer = if self.period == 0 { 8 } else { self.period };
        self.volume = self.initial;
    }

    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period;
            if self.up && self.volume < 15 {
                self.volume += 1;
            } else if !self.up && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

#[derive(Clone, Default)]
struct Sweep {
    period: u8,
    negate: bool,
    shift: u8,
    timer: u8,
    enabled: bool,
    shadow: u16,
    /// A subtraction happened since the last trigger; clearing the negate
    /// bit after that kills the channel.
    negate_used: bool,
}

#[derive(Clone, Default)]
struct Pulse {
    enabled: bool,
    dac: bool,
    duty: u8,
    step: u8,
    freq: u16,
    timer: i32,
    length: Length,
    env: Envelope,
    sweep: Sweep,
}

impl Pulse {
    fn tick(&mut self, cycles: i32) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += (2048 - i32::from(self.freq)) * 4;
            self.step = (self.step + 1) & 7;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled { DUTY[usize::from(self.duty)][usize::from(self.step)] * self.env.volume } else { 0 }
    }

    fn sweep_calc(&mut self) -> u16 {
        let delta = self.sweep.shadow >> self.sweep.shift;
        let f = if self.sweep.negate {
            self.sweep.negate_used = true;
            self.sweep.shadow.wrapping_sub(delta)
        } else {
            self.sweep.shadow + delta
        };
        if f > 2047 {
            self.enabled = false;
        }
        f
    }

    fn sweep_clock(&mut self) {
        self.sweep.timer = self.sweep.timer.saturating_sub(1);
        if self.sweep.timer != 0 {
            return;
        }
        self.sweep.timer = if self.sweep.period == 0 { 8 } else { self.sweep.period };
        if self.sweep.enabled && self.sweep.period != 0 {
            let f = self.sweep_calc();
            if f <= 2047 && self.sweep.shift != 0 {
                self.freq = f;
                self.sweep.shadow = f;
                self.sweep_calc();
            }
        }
    }

    fn trigger(&mut self, with_sweep: bool) {
        self.enabled = self.dac;
        self.timer = (2048 - i32::from(self.freq)) * 4;
        self.env.trigger();
        if with_sweep {
            self.sweep.shadow = self.freq;
            self.sweep.timer = if self.sweep.period == 0 { 8 } else { self.sweep.period };
            self.sweep.enabled = self.sweep.period != 0 || self.sweep.shift != 0;
            self.sweep.negate_used = false;
            if self.sweep.shift != 0 {
                self.sweep_calc();
            }
        }
    }
}

#[derive(Clone, Default)]
struct Wave {
    enabled: bool,
    dac: bool,
    volume_code: u8,
    freq: u16,
    timer: i32,
    position: u8,
    sample: u8,
    length: Length,
}

impl Wave {
    fn tick(&mut self, cycles: i32, ram: &[u8; 16]) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += (2048 - i32::from(self.freq)) * 2;
            self.position = (self.position + 1) & 31;
            let byte = ram[usize::from(self.position / 2)];
            self.sample = if self.position & 1 == 0 { byte >> 4 } else { byte & 0x0F };
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.volume_code == 0 {
            return 0;
        }
        self.sample >> (self.volume_code - 1)
    }
}

#[derive(Clone)]
struct Noise {
    enabled: bool,
    dac: bool,
    shift: u8,
    narrow: bool,
    divisor_code: u8,
    timer: i32,
    lfsr: u16,
    length: Length,
    env: Envelope,
}

impl Noise {
    fn period(&self) -> i32 {
        let divisor = if self.divisor_code == 0 { 8 } else { i32::from(self.divisor_code) * 16 };
        divisor << self.shift
    }

    fn tick(&mut self, cycles: i32) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period();
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.narrow {
                self.lfsr = (self.lfsr & !0x40) | (bit << 6);
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.lfsr & 1 == 0 { self.env.volume } else { 0 }
    }
}

pub struct Apu {
    power: bool,
    /// Raw register bytes as last written, for read-back.
    regs: [u8; 0x20],
    wave_ram: [u8; 16],
    ch1: Pulse,
    ch2: Pulse,
    ch3: Wave,
    ch4: Noise,
    /// Next frame sequencer step, 0..8.
    frame_step: u8,

    sample_rate: u32,
    cycles_per_sample: f64,
    sample_clock: f64,
    acc: [i32; 4],
    acc_count: u32,
    hp_left: f32,
    hp_right: f32,
    hp_factor: f32,
    samples: Vec<f32>,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Self {
        let mut apu = Self {
            power: true,
            regs: [0; 0x20],
            wave_ram: [0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, 0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA],
            ch1: Pulse::default(),
            ch2: Pulse::default(),
            ch3: Wave::default(),
            ch4: Noise {
                enabled: false,
                dac: false,
                shift: 0,
                narrow: false,
                divisor_code: 0,
                timer: 8,
                lfsr: 0x7FFF,
                length: Length::default(),
                env: Envelope::default(),
            },
            frame_step: 0,
            sample_rate: 0,
            cycles_per_sample: 0.0,
            sample_clock: 0.0,
            acc: [0; 4],
            acc_count: 0,
            hp_left: 0.0,
            hp_right: 0.0,
            hp_factor: 0.0,
            samples: Vec::with_capacity(4096),
        };
        apu.set_sample_rate(sample_rate);
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
        apu.ch1.enabled = true;
        apu.ch1.env.volume = 0;
        apu
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate.max(1);
        self.cycles_per_sample = CPU_HZ / f64::from(self.sample_rate);
        // Charge factor of the output capacitor, scaled to our sample rate.
        self.hp_factor = 0.999_958_f64.powf(self.cycles_per_sample) as f32;
    }

    /// Interleaved stereo samples produced since the last [`Apu::clear_samples`].
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }

    /// Advances all channels by one M-cycle and feeds the resampler.
    pub fn tick(&mut self) {
        if self.power {
            self.ch1.tick(4);
            self.ch2.tick(4);
            self.ch3.tick(4, &self.wave_ram);
            self.ch4.tick(4);
            // Each DAC maps digital 0..15 to an analog level from -1 to +1
            // (stored here as 2v-15, i.e. scaled by 15). A disabled DAC is
            // silent rather than outputting its "zero" level. Summing these
            // integers is a box filter; mixing is linear, so it can wait
            // until a whole output sample has been collected.
            let dac = |on: bool, v: u8| if on { 2 * i32::from(v) - 15 } else { 0 };
            self.acc[0] += dac(self.ch1.dac, self.ch1.output());
            self.acc[1] += dac(self.ch2.dac, self.ch2.output());
            self.acc[2] += dac(self.ch3.dac, self.ch3.output());
            self.acc[3] += dac(self.ch4.dac, self.ch4.output());
        }
        self.acc_count += 1;
        self.sample_clock += 4.0;
        if self.sample_clock >= self.cycles_per_sample {
            self.sample_clock -= self.cycles_per_sample;
            let (l, r) = self.mix();
            self.acc = [0; 4];
            self.acc_count = 0;
            let out_l = l - self.hp_left;
            self.hp_left = l - out_l * self.hp_factor;
            let out_r = r - self.hp_right;
            self.hp_right = r - out_r * self.hp_factor;
            self.samples.push(out_l);
            self.samples.push(out_r);
        }
    }

    /// Mixes the averaged channel levels through NR51 panning and NR50
    /// master volume.
    fn mix(&self) -> (f32, f32) {
        let nr50 = self.regs[0x14];
        let nr51 = self.regs[0x15];
        let mut l = 0;
        let mut r = 0;
        for (i, &v) in self.acc.iter().enumerate() {
            if nr51 & (0x10 << i) != 0 {
                l += v;
            }
            if nr51 & (1 << i) != 0 {
                r += v;
            }
        }
        // Four channels at full swing and master volume 8 would reach 1.0;
        // keep some headroom for the transient after a DAC switches on.
        let scale = 1.0 / (15.0 * self.acc_count.max(1) as f32 * 40.0);
        let vol_l = f32::from((nr50 >> 4) & 7) + 1.0;
        let vol_r = f32::from(nr50 & 7) + 1.0;
        (l as f32 * vol_l * scale, r as f32 * vol_r * scale)
    }

    /// Called on the falling edge of DIV bit 4 (512 Hz).
    pub fn frame_sequencer_step(&mut self) {
        if !self.power {
            return;
        }
        let step = self.frame_step;
        self.frame_step = (step + 1) & 7;
        if step & 1 == 0 {
            if self.ch1.length.clock() {
                self.ch1.enabled = false;
            }
            if self.ch2.length.clock() {
                self.ch2.enabled = false;
            }
            if self.ch3.length.clock() {
                self.ch3.enabled = false;
            }
            if self.ch4.length.clock() {
                self.ch4.enabled = false;
            }
        }
        if step == 2 || step == 6 {
            self.ch1.sweep_clock();
        }
        if step == 7 {
            self.ch1.env.clock();
            self.ch2.env.clock();
            self.ch4.env.clock();
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF26 => {
                let status = u8::from(self.ch1.enabled)
                    | u8::from(self.ch2.enabled) << 1
                    | u8::from(self.ch3.enabled) << 2
                    | u8::from(self.ch4.enabled) << 3;
                0x70 | u8::from(self.power) << 7 | status
            }
            0xFF10..=0xFF2F => {
                let i = usize::from(addr - 0xFF10);
                self.regs[i] | READ_MASK[i]
            }
            // While the wave channel runs, the CPU sees the byte it is
            // currently playing (a simplification of the DMG behaviour).
            0xFF30..=0xFF3F if self.ch3.enabled => self.wave_ram[usize::from(self.ch3.position / 2)],
            0xFF30..=0xFF3F => self.wave_ram[usize::from(addr - 0xFF30)],
            _ => 0xFF,
        }
    }

    /// Length enable writes can clock the length counter immediately when
    /// the frame sequencer's next step is not a length step.
    fn length_enable_write(length: &mut Length, enable: bool, frame_step: u8) -> bool {
        let was = length.enabled;
        length.enabled = enable;
        if !was && enable && frame_step & 1 == 1 && length.counter > 0 {
            length.counter -= 1;
            return length.counter == 0;
        }
        false
    }

    fn reload_on_trigger(length: &mut Length, max: u16, frame_step: u8) {
        if length.counter == 0 {
            length.counter = if length.enabled && frame_step & 1 == 1 { max - 1 } else { max };
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        if let 0xFF30..=0xFF3F = addr {
            self.wave_ram[usize::from(addr - 0xFF30)] = value;
            return;
        }
        if !(0xFF10..=0xFF2F).contains(&addr) {
            return;
        }
        if addr == 0xFF26 {
            let on = value & 0x80 != 0;
            if self.power && !on {
                // Power-off clears every register, but on DMG the length
                // counters survive.
                let lengths = [
                    self.ch1.length.counter,
                    self.ch2.length.counter,
                    self.ch3.length.counter,
                    self.ch4.length.counter,
                ];
                for a in 0xFF10..=0xFF25 {
                    self.write(a, 0);
                }
                self.ch1.length.counter = lengths[0];
                self.ch2.length.counter = lengths[1];
                self.ch3.length.counter = lengths[2];
                self.ch4.length.counter = lengths[3];
                self.ch1.enabled = false;
                self.ch2.enabled = false;
                self.ch3.enabled = false;
                self.ch4.enabled = false;
                self.power = false;
            } else if !self.power && on {
                self.power = true;
                self.frame_step = 0;
                self.ch1.step = 0;
                self.ch2.step = 0;
                self.ch3.sample = 0;
            }
            return;
        }
        // With the APU off only the length timers can be written (DMG).
        if !self.power && !matches!(addr, 0xFF11 | 0xFF16 | 0xFF1B | 0xFF20) {
            return;
        }
        let length_only = !self.power;
        let i = usize::from(addr - 0xFF10);
        if !length_only {
            self.regs[i] = value;
        }
        let fs = self.frame_step;
        match addr {
            0xFF10 => {
                let s = &mut self.ch1.sweep;
                s.period = (value >> 4) & 7;
                s.negate = value & 0x08 != 0;
                s.shift = value & 7;
                if !s.negate && s.negate_used {
                    self.ch1.enabled = false;
                }
            }
            0xFF11 | 0xFF16 => {
                let ch = if addr == 0xFF11 { &mut self.ch1 } else { &mut self.ch2 };
                if !length_only {
                    ch.duty = value >> 6;
                }
                ch.length.counter = 64 - u16::from(value & 0x3F);
            }
            0xFF12 | 0xFF17 => {
                let ch = if addr == 0xFF12 { &mut self.ch1 } else { &mut self.ch2 };
                ch.env.write(value);
                ch.dac = value & 0xF8 != 0;
                if !ch.dac {
                    ch.enabled = false;
                }
            }
            0xFF13 | 0xFF18 => {
                let ch = if addr == 0xFF13 { &mut self.ch1 } else { &mut self.ch2 };
                ch.freq = (ch.freq & 0x700) | u16::from(value);
            }
            0xFF14 | 0xFF19 => {
                let ch = if addr == 0xFF14 { &mut self.ch1 } else { &mut self.ch2 };
                ch.freq = (ch.freq & 0xFF) | (u16::from(value & 7) << 8);
                if Self::length_enable_write(&mut ch.length, value & 0x40 != 0, fs) && value & 0x80 == 0 {
                    ch.enabled = false;
                }
                if value & 0x80 != 0 {
                    Self::reload_on_trigger(&mut ch.length, 64, fs);
                    ch.trigger(addr == 0xFF14);
                }
            }
            0xFF1A => {
                self.ch3.dac = value & 0x80 != 0;
                if !self.ch3.dac {
                    self.ch3.enabled = false;
                }
            }
            0xFF1B => self.ch3.length.counter = 256 - u16::from(value),
            0xFF1C => self.ch3.volume_code = (value >> 5) & 3,
            0xFF1D => self.ch3.freq = (self.ch3.freq & 0x700) | u16::from(value),
            0xFF1E => {
                let ch = &mut self.ch3;
                ch.freq = (ch.freq & 0xFF) | (u16::from(value & 7) << 8);
                if Self::length_enable_write(&mut ch.length, value & 0x40 != 0, fs) && value & 0x80 == 0 {
                    ch.enabled = false;
                }
                if value & 0x80 != 0 {
                    Self::reload_on_trigger(&mut ch.length, 256, fs);
                    ch.enabled = ch.dac;
                    // The first sample is delayed by a few cycles after trigger.
                    ch.timer = (2048 - i32::from(ch.freq)) * 2 + 6;
                    ch.position = 0;
                }
            }
            0xFF20 => self.ch4.length.counter = 64 - u16::from(value & 0x3F),
            0xFF21 => {
                self.ch4.env.write(value);
                self.ch4.dac = value & 0xF8 != 0;
                if !self.ch4.dac {
                    self.ch4.enabled = false;
                }
            }
            0xFF22 => {
                self.ch4.shift = value >> 4;
                self.ch4.narrow = value & 0x08 != 0;
                self.ch4.divisor_code = value & 7;
            }
            0xFF23 => {
                let ch = &mut self.ch4;
                if Self::length_enable_write(&mut ch.length, value & 0x40 != 0, fs) && value & 0x80 == 0 {
                    ch.enabled = false;
                }
                if value & 0x80 != 0 {
                    Self::reload_on_trigger(&mut ch.length, 64, fs);
                    ch.enabled = ch.dac;
                    ch.timer = ch.period();
                    ch.lfsr = 0x7FFF;
                    ch.env.trigger();
                }
            }
            _ => {}
        }
    }
}
