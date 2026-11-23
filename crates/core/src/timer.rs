//! DIV/TIMA/TMA/TAC.
//!
//! DIV is the top byte of a 16-bit counter that ticks every T-cycle. TIMA does
//! not have its own prescaler: it increments on the *falling edge* of
//! `counter_bit(TAC) AND timer_enable`. Modelling that AND gate directly gives
//! the documented glitches for free: writing DIV while the selected bit is 1,
//! or disabling the timer while it is 1, both produce a falling edge and an
//! extra increment.
//!
//! On overflow TIMA reads 0x00 for one M-cycle, then gets reloaded from TMA
//! and the interrupt is raised. Writing TIMA during that gap cancels the
//! reload; during the reload cycle itself TIMA writes are dropped and TMA
//! writes go straight through to TIMA.

const TAC_BITS: [u16; 4] = [1 << 9, 1 << 3, 1 << 5, 1 << 7];

#[derive(Clone, Debug)]
pub struct Timer {
    /// Internal 16-bit divider. Always a multiple of 4 because the bus only
    /// advances it in whole M-cycles.
    pub(crate) counter: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    overflow_pending: bool,
    reloading: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            // Value on DMG A/B/C at the moment the boot ROM jumps to 0x0100.
            counter: 0xABC8,
            tima: 0,
            tma: 0,
            tac: 0,
            overflow_pending: false,
            reloading: false,
        }
    }

    fn signal(&self) -> bool {
        self.tac & 0x04 != 0 && self.counter & TAC_BITS[usize::from(self.tac & 3)] != 0
    }

    fn increment(&mut self) {
        let (v, overflow) = self.tima.overflowing_add(1);
        self.tima = v;
        if overflow {
            self.overflow_pending = true;
        }
    }

    /// Advances one M-cycle. Returns true when the timer interrupt fires.
    pub fn tick(&mut self) -> bool {
        self.reloading = false;
        let mut irq = false;
        if self.overflow_pending {
            self.overflow_pending = false;
            self.tima = self.tma;
            self.reloading = true;
            irq = true;
        }
        let before = self.signal();
        self.counter = self.counter.wrapping_add(4);
        if before && !self.signal() {
            self.increment();
        }
        irq
    }

    pub fn div(&self) -> u8 {
        (self.counter >> 8) as u8
    }

    /// Any write to DIV clears the whole internal counter.
    pub fn write_div(&mut self) {
        let before = self.signal();
        self.counter = 0;
        if before && !self.signal() {
            self.increment();
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => self.div(),
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            _ => self.tac | 0xF8,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF04 => self.write_div(),
            0xFF05 => {
                if !self.reloading {
                    self.tima = value;
                    self.overflow_pending = false;
                }
            }
            0xFF06 => {
                self.tma = value;
                if self.reloading {
                    self.tima = value;
                }
            }
            _ => {
                let before = self.signal();
                self.tac = value & 0x07;
                if before && !self.signal() {
                    self.increment();
                }
            }
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer_with(tac: u8) -> Timer {
        let mut t = Timer::new();
        t.counter = 0;
        t.write(0xFF07, tac);
        t
    }

    #[test]
    fn tima_rate_follows_tac() {
        // TAC=01 selects 262144 Hz: one increment every 16 T-cycles (4 M-cycles).
        let mut t = timer_with(0x05);
        for _ in 0..40 {
            t.tick();
        }
        assert_eq!(t.read(0xFF05), 10);

        // TAC=00 selects 4096 Hz: one increment every 1024 T-cycles.
        let mut t = timer_with(0x04);
        for _ in 0..256 * 3 {
            t.tick();
        }
        assert_eq!(t.read(0xFF05), 3);
    }

    #[test]
    fn overflow_reloads_one_cycle_late() {
        let mut t = timer_with(0x05);
        t.write(0xFF06, 0xAB);
        t.write(0xFF05, 0xFF);
        // Three M-cycles bring the counter to 12; the fourth drops bit 3.
        for _ in 0..3 {
            assert!(!t.tick());
        }
        assert!(!t.tick());
        assert_eq!(t.read(0xFF05), 0x00, "TIMA reads 0 during the delay cycle");
        assert!(t.tick(), "interrupt is raised on the reload cycle");
        assert_eq!(t.read(0xFF05), 0xAB);
    }

    #[test]
    fn writing_tima_during_delay_cancels_reload() {
        let mut t = timer_with(0x05);
        t.write(0xFF06, 0xAB);
        t.write(0xFF05, 0xFF);
        for _ in 0..4 {
            t.tick();
        }
        t.write(0xFF05, 0x42);
        assert!(!t.tick());
        assert_eq!(t.read(0xFF05), 0x42);
    }

    #[test]
    fn tma_write_on_reload_cycle_reaches_tima() {
        let mut t = timer_with(0x05);
        t.write(0xFF05, 0xFF);
        for _ in 0..5 {
            t.tick();
        }
        t.write(0xFF05, 0x11); // ignored: this is the reload cycle
        t.write(0xFF06, 0x77);
        assert_eq!(t.read(0xFF05), 0x77);
    }

    #[test]
    fn div_reset_can_clock_tima() {
        let mut t = timer_with(0x05);
        for _ in 0..2 {
            t.tick();
        }
        // counter = 8: bit 3 is high, so resetting DIV is a falling edge.
        assert_eq!(t.read(0xFF05), 0);
        t.write(0xFF04, 0);
        assert_eq!(t.read(0xFF05), 1);
        assert_eq!(t.read(0xFF04), 0);
    }

    #[test]
    fn disabling_timer_while_bit_high_clocks_tima() {
        let mut t = timer_with(0x05);
        for _ in 0..2 {
            t.tick();
        }
        t.write(0xFF07, 0x01);
        assert_eq!(t.read(0xFF05), 1);
        assert_eq!(t.read(0xFF07), 0xF9);
    }
}
