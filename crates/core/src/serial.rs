//! Serial port with nothing plugged in.
//!
//! Every byte the game starts sending is captured, which is how Blargg's
//! test ROMs report results. With the internal clock, bits shift out on the
//! falling edge of bit 8 of the system counter (8192 Hz), so a transfer
//! takes 4096 T-cycles and receives 0xFF from the empty link port.

#[derive(Clone, Debug, Default)]
pub struct Serial {
    sb: u8,
    sc: u8,
    bits_left: u8,
    output: Vec<u8>,
}

impl Serial {
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    pub fn clear_output(&mut self) {
        self.output.clear();
    }

    pub fn read(&self, addr: u16) -> u8 {
        if addr == 0xFF01 { self.sb } else { self.sc | 0x7E }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        if addr == 0xFF01 {
            self.sb = value;
            return;
        }
        self.sc = value & 0x81;
        if value & 0x80 != 0 {
            if self.output.len() < 1 << 20 {
                self.output.push(self.sb);
            }
            self.bits_left = 8;
        } else {
            self.bits_left = 0;
        }
    }

    /// Called every M-cycle with the system counter before and after the
    /// tick. Returns true when a transfer completes (serial interrupt).
    pub fn tick(&mut self, counter_before: u16, counter_after: u16) -> bool {
        if self.bits_left == 0 || self.sc & 0x01 == 0 {
            return false;
        }
        if counter_before & !counter_after & 0x0100 == 0 {
            return false;
        }
        self.sb = (self.sb << 1) | 1;
        self.bits_left -= 1;
        if self.bits_left == 0 {
            self.sc &= 0x7F;
            return true;
        }
        false
    }
}
