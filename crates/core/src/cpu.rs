//! The Sharp SM83 CPU.
//!
//! There is no cycle table in here. Every memory access goes through
//! [`Cpu::read`] / [`Cpu::write`], which advance the rest of the machine by
//! one M-cycle before touching the bus, and instructions that spend time
//! doing internal work call [`Bus::tick`] explicitly. Instruction timing,
//! and the position of each access inside an instruction, falls out of the
//! instruction bodies themselves. That is what Blargg's `mem_timing` and the
//! Mooneye `*_timing` tests look at.

use crate::bus::Bus;

pub(crate) const FLAG_Z: u8 = 0x80;
pub(crate) const FLAG_N: u8 = 0x40;
pub(crate) const FLAG_H: u8 = 0x20;
pub(crate) const FLAG_C: u8 = 0x10;

/// The CPU register file. `f` only ever holds the upper four flag bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Registers {
    /// Register contents a DMG (revisions A/B/C) leaves behind when its boot
    /// ROM hands control to the cartridge at `0x0100`.
    pub fn post_boot_dmg() -> Self {
        Self { a: 0x01, f: 0xB0, b: 0x00, c: 0x13, d: 0x00, e: 0xD8, h: 0x01, l: 0x4D, sp: 0xFFFE, pc: 0x0100 }
    }

    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    pub fn set_af(&mut self, v: u16) {
        let [hi, lo] = v.to_be_bytes();
        self.a = hi;
        self.f = lo & 0xF0;
    }
    pub fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }
    pub fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }
    pub fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }

    fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }

    fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.f = (u8::from(z) << 7) | (u8::from(n) << 6) | (u8::from(h) << 5) | (u8::from(c) << 4);
    }
}

pub struct Cpu {
    pub regs: Registers,
    pub(crate) ime: bool,
    /// Set by `EI`; IME turns on after the *next* instruction.
    ei_pending: bool,
    pub(crate) halted: bool,
    /// HALT with IME=0 and an interrupt already pending: the following opcode
    /// byte is read twice because PC fails to increment.
    halt_bug: bool,
    pub(crate) stopped: bool,
    /// An illegal opcode hard-locks the real CPU until power off.
    locked: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: Registers::post_boot_dmg(),
            ime: false,
            ei_pending: false,
            halted: false,
            halt_bug: false,
            stopped: false,
            locked: false,
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Runs one instruction, one interrupt dispatch, or one idle M-cycle
    /// while halted/stopped.
    pub fn step(&mut self, bus: &mut Bus) {
        if self.locked {
            bus.tick();
            return;
        }
        if self.stopped {
            bus.tick();
            if bus.joypad.any_pressed() {
                self.stopped = false;
            }
            return;
        }
        // Interrupts are sampled in the M-cycle that fetches the next opcode.
        // An interrupt raised during that very cycle still wins: the fetched
        // byte is thrown away and dispatch begins instead. While halted the
        // CPU keeps doing exactly that, so waking up needs no extra cycle.
        bus.tick();
        if self.halted {
            if bus.pending_interrupts() == 0 {
                return;
            }
            self.halted = false;
        }
        if self.ime && bus.pending_interrupts() != 0 {
            self.dispatch_interrupt(bus);
            return;
        }
        if self.ei_pending {
            self.ei_pending = false;
            self.ime = true;
        }
        let pc = self.regs.pc;
        let op = bus.read(pc);
        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.regs.pc = pc.wrapping_add(1);
        }
        self.execute(bus, op);
    }

    /// Interrupt dispatch: the discarded opcode fetch, one idle M-cycle,
    /// push PC (2), jump (1). Five M-cycles in total.
    ///
    /// The vector is chosen *after* the high byte of PC has been pushed. If
    /// that push lands on IE (SP=0x0000) and clears the pending bit, the CPU
    /// jumps to 0x0000 instead. Mooneye's `ie_push` checks exactly this.
    fn dispatch_interrupt(&mut self, bus: &mut Bus) {
        bus.tick();
        let [pc_hi, pc_lo] = self.regs.pc.to_be_bytes();
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, pc_hi);
        let pending = bus.pending_interrupts();
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, pc_lo);
        self.ime = false;
        if pending == 0 {
            self.regs.pc = 0x0000;
        } else {
            let bit = pending.trailing_zeros();
            bus.if_reg &= !(1 << bit);
            self.regs.pc = 0x0040 + 8 * bit as u16;
        }
        bus.tick();
    }

    // ---- bus helpers -------------------------------------------------------

    fn read(&mut self, bus: &mut Bus, addr: u16) -> u8 {
        bus.tick();
        bus.read(addr)
    }

    fn write(&mut self, bus: &mut Bus, addr: u16, value: u8) {
        bus.tick();
        bus.write(addr, value);
    }

    fn imm8(&mut self, bus: &mut Bus) -> u8 {
        let v = self.read(bus, self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    fn imm16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.imm8(bus);
        let hi = self.imm8(bus);
        u16::from_le_bytes([lo, hi])
    }

    fn push(&mut self, bus: &mut Bus, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, hi);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, lo);
    }

    fn pop(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.read(bus, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = self.read(bus, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    /// 8-bit operand by its 3-bit encoding: B C D E H L (HL) A.
    fn get_r8(&mut self, bus: &mut Bus, idx: u8) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            6 => self.read(bus, self.regs.hl()),
            _ => self.regs.a,
        }
    }

    fn set_r8(&mut self, bus: &mut Bus, idx: u8, value: u8) {
        match idx {
            0 => self.regs.b = value,
            1 => self.regs.c = value,
            2 => self.regs.d = value,
            3 => self.regs.e = value,
            4 => self.regs.h = value,
            5 => self.regs.l = value,
            6 => self.write(bus, self.regs.hl(), value),
            _ => self.regs.a = value,
        }
    }

    /// 16-bit operand by its 2-bit encoding: BC DE HL SP.
    fn get_r16(&self, idx: u8) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            _ => self.regs.sp,
        }
    }

    fn set_r16(&mut self, idx: u8, value: u16) {
        match idx {
            0 => self.regs.set_bc(value),
            1 => self.regs.set_de(value),
            2 => self.regs.set_hl(value),
            _ => self.regs.sp = value,
        }
    }

    fn condition(&self, idx: u8) -> bool {
        match idx & 3 {
            0 => !self.regs.flag(FLAG_Z),
            1 => self.regs.flag(FLAG_Z),
            2 => !self.regs.flag(FLAG_C),
            _ => self.regs.flag(FLAG_C),
        }
    }

    // ---- execution ---------------------------------------------------------

    fn execute(&mut self, bus: &mut Bus, op: u8) {
        match op {
            0x00 => {}
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.imm16(bus);
                self.set_r16(op >> 4, v);
            }
            0x02 => self.write(bus, self.regs.bc(), self.regs.a),
            0x12 => self.write(bus, self.regs.de(), self.regs.a),
            0x22 => {
                let hl = self.regs.hl();
                self.write(bus, hl, self.regs.a);
                self.regs.set_hl(hl.wrapping_add(1));
            }
            0x32 => {
                let hl = self.regs.hl();
                self.write(bus, hl, self.regs.a);
                self.regs.set_hl(hl.wrapping_sub(1));
            }
            0x0A => self.regs.a = self.read(bus, self.regs.bc()),
            0x1A => self.regs.a = self.read(bus, self.regs.de()),
            0x2A => {
                let hl = self.regs.hl();
                self.regs.a = self.read(bus, hl);
                self.regs.set_hl(hl.wrapping_add(1));
            }
            0x3A => {
                let hl = self.regs.hl();
                self.regs.a = self.read(bus, hl);
                self.regs.set_hl(hl.wrapping_sub(1));
            }
            0x03 | 0x13 | 0x23 | 0x33 => {
                let idx = op >> 4;
                self.set_r16(idx, self.get_r16(idx).wrapping_add(1));
                bus.tick();
            }
            0x0B | 0x1B | 0x2B | 0x3B => {
                let idx = op >> 4;
                self.set_r16(idx, self.get_r16(idx).wrapping_sub(1));
                bus.tick();
            }
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let idx = (op >> 3) & 7;
                let v = self.get_r8(bus, idx);
                let r = v.wrapping_add(1);
                let c = self.regs.flag(FLAG_C);
                self.regs.set_flags(r == 0, false, v & 0x0F == 0x0F, c);
                self.set_r8(bus, idx, r);
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let idx = (op >> 3) & 7;
                let v = self.get_r8(bus, idx);
                let r = v.wrapping_sub(1);
                let c = self.regs.flag(FLAG_C);
                self.regs.set_flags(r == 0, true, v & 0x0F == 0, c);
                self.set_r8(bus, idx, r);
            }
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let v = self.imm8(bus);
                self.set_r8(bus, (op >> 3) & 7, v);
            }
            0x07 => {
                let r = self.rlc(self.regs.a);
                self.regs.a = r;
                self.regs.f &= !FLAG_Z;
            }
            0x0F => {
                let r = self.rrc(self.regs.a);
                self.regs.a = r;
                self.regs.f &= !FLAG_Z;
            }
            0x17 => {
                let r = self.rl(self.regs.a);
                self.regs.a = r;
                self.regs.f &= !FLAG_Z;
            }
            0x1F => {
                let r = self.rr(self.regs.a);
                self.regs.a = r;
                self.regs.f &= !FLAG_Z;
            }
            0x08 => {
                let addr = self.imm16(bus);
                let [hi, lo] = self.regs.sp.to_be_bytes();
                self.write(bus, addr, lo);
                self.write(bus, addr.wrapping_add(1), hi);
            }
            0x09 | 0x19 | 0x29 | 0x39 => {
                let hl = self.regs.hl();
                let v = self.get_r16(op >> 4);
                let (r, carry) = hl.overflowing_add(v);
                let half = (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF;
                let z = self.regs.flag(FLAG_Z);
                self.regs.set_flags(z, false, half, carry);
                self.regs.set_hl(r);
                bus.tick();
            }
            0x10 => {
                // STOP is a two-byte opcode; the second byte is skipped.
                self.regs.pc = self.regs.pc.wrapping_add(1);
                bus.timer.write_div();
                if !bus.joypad.any_pressed() {
                    self.stopped = true;
                }
            }
            0x18 => {
                let e = self.imm8(bus) as i8;
                self.regs.pc = self.regs.pc.wrapping_add_signed(e.into());
                bus.tick();
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let e = self.imm8(bus) as i8;
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = self.regs.pc.wrapping_add_signed(e.into());
                    bus.tick();
                }
            }
            0x27 => self.daa(),
            0x2F => {
                self.regs.a = !self.regs.a;
                self.regs.f |= FLAG_N | FLAG_H;
            }
            0x37 => self.regs.f = (self.regs.f & FLAG_Z) | FLAG_C,
            0x3F => self.regs.f = (self.regs.f & (FLAG_Z | FLAG_C)) ^ FLAG_C,
            0x76 => {
                if !self.ime && bus.pending_interrupts() != 0 {
                    self.halt_bug = true;
                } else {
                    self.halted = true;
                }
            }
            0x40..=0x7F => {
                let v = self.get_r8(bus, op & 7);
                self.set_r8(bus, (op >> 3) & 7, v);
            }
            0x80..=0xBF => {
                let v = self.get_r8(bus, op & 7);
                self.alu((op >> 3) & 7, v);
            }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                bus.tick();
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = self.pop(bus);
                    bus.tick();
                }
            }
            0xC1 | 0xD1 | 0xE1 => {
                let v = self.pop(bus);
                self.set_r16((op >> 4) & 3, v);
            }
            0xF1 => {
                let v = self.pop(bus);
                self.regs.set_af(v);
            }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let addr = self.imm16(bus);
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = addr;
                    bus.tick();
                }
            }
            0xC3 => {
                self.regs.pc = self.imm16(bus);
                bus.tick();
            }
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let addr = self.imm16(bus);
                if self.condition((op >> 3) & 3) {
                    bus.tick();
                    self.push(bus, self.regs.pc);
                    self.regs.pc = addr;
                }
            }
            0xC5 | 0xD5 | 0xE5 => {
                bus.tick();
                self.push(bus, self.get_r16((op >> 4) & 3));
            }
            0xF5 => {
                bus.tick();
                self.push(bus, self.regs.af());
            }
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.imm8(bus);
                self.alu((op >> 3) & 7, v);
            }
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                bus.tick();
                self.push(bus, self.regs.pc);
                self.regs.pc = u16::from(op & 0x38);
            }
            0xC9 => {
                self.regs.pc = self.pop(bus);
                bus.tick();
            }
            0xD9 => {
                self.regs.pc = self.pop(bus);
                bus.tick();
                self.ime = true;
            }
            0xCB => self.execute_cb(bus),
            0xCD => {
                let addr = self.imm16(bus);
                bus.tick();
                self.push(bus, self.regs.pc);
                self.regs.pc = addr;
            }
            0xE0 => {
                let n = self.imm8(bus);
                self.write(bus, 0xFF00 | u16::from(n), self.regs.a);
            }
            0xF0 => {
                let n = self.imm8(bus);
                self.regs.a = self.read(bus, 0xFF00 | u16::from(n));
            }
            0xE2 => self.write(bus, 0xFF00 | u16::from(self.regs.c), self.regs.a),
            0xF2 => self.regs.a = self.read(bus, 0xFF00 | u16::from(self.regs.c)),
            0xE8 => {
                let e = self.imm8(bus);
                self.regs.sp = self.add_sp_e(e);
                bus.tick();
                bus.tick();
            }
            0xF8 => {
                let e = self.imm8(bus);
                let r = self.add_sp_e(e);
                self.regs.set_hl(r);
                bus.tick();
            }
            0xE9 => self.regs.pc = self.regs.hl(),
            0xF9 => {
                self.regs.sp = self.regs.hl();
                bus.tick();
            }
            0xEA => {
                let addr = self.imm16(bus);
                self.write(bus, addr, self.regs.a);
            }
            0xFA => {
                let addr = self.imm16(bus);
                self.regs.a = self.read(bus, addr);
            }
            0xF3 => {
                self.ime = false;
                self.ei_pending = false;
            }
            0xFB => self.ei_pending = true,
            // D3 DB DD E3 E4 EB EC ED F4 FC FD
            _ => self.locked = true,
        }
    }

    fn execute_cb(&mut self, bus: &mut Bus) {
        let op = self.imm8(bus);
        let idx = op & 7;
        let bit = (op >> 3) & 7;
        let v = self.get_r8(bus, idx);
        match op >> 6 {
            0 => {
                let r = match bit {
                    0 => self.rlc(v),
                    1 => self.rrc(v),
                    2 => self.rl(v),
                    3 => self.rr(v),
                    4 => self.sla(v),
                    5 => self.sra(v),
                    6 => self.swap(v),
                    _ => self.srl(v),
                };
                self.set_r8(bus, idx, r);
            }
            1 => {
                let c = self.regs.flag(FLAG_C);
                self.regs.set_flags(v & (1 << bit) == 0, false, true, c);
            }
            2 => self.set_r8(bus, idx, v & !(1 << bit)),
            _ => self.set_r8(bus, idx, v | (1 << bit)),
        }
    }

    // ---- ALU ---------------------------------------------------------------

    fn alu(&mut self, op: u8, v: u8) {
        let a = self.regs.a;
        let carry_in = u8::from(self.regs.flag(FLAG_C));
        match op {
            0 | 1 => {
                let c = if op == 1 { carry_in } else { 0 };
                let r = u16::from(a) + u16::from(v) + u16::from(c);
                let half = (a & 0x0F) + (v & 0x0F) + c > 0x0F;
                self.regs.a = r as u8;
                self.regs.set_flags(r as u8 == 0, false, half, r > 0xFF);
            }
            2 | 3 | 7 => {
                let c = if op == 3 { carry_in } else { 0 };
                let r = a.wrapping_sub(v).wrapping_sub(c);
                let half = (a & 0x0F) < (v & 0x0F) + c;
                let borrow = u16::from(a) < u16::from(v) + u16::from(c);
                self.regs.set_flags(r == 0, true, half, borrow);
                if op != 7 {
                    self.regs.a = r;
                }
            }
            4 => {
                self.regs.a = a & v;
                self.regs.set_flags(self.regs.a == 0, false, true, false);
            }
            5 => {
                self.regs.a = a ^ v;
                self.regs.set_flags(self.regs.a == 0, false, false, false);
            }
            _ => {
                self.regs.a = a | v;
                self.regs.set_flags(self.regs.a == 0, false, false, false);
            }
        }
    }

    /// Shared by `ADD SP,e` and `LD HL,SP+e`: the offset is signed, but H and
    /// C come from an *unsigned* add of the low byte of SP and the raw byte.
    fn add_sp_e(&mut self, e: u8) -> u16 {
        let sp = self.regs.sp;
        let half = (sp & 0x0F) + u16::from(e & 0x0F) > 0x0F;
        let carry = (sp & 0xFF) + u16::from(e) > 0xFF;
        self.regs.set_flags(false, false, half, carry);
        sp.wrapping_add_signed(i16::from(e as i8))
    }

    /// Decimal-adjusts A after a BCD add or subtract, using N/H/C left behind
    /// by that operation.
    fn daa(&mut self) {
        let mut a = self.regs.a;
        let n = self.regs.flag(FLAG_N);
        let h = self.regs.flag(FLAG_H);
        let mut c = self.regs.flag(FLAG_C);
        if !n {
            if c || a > 0x99 {
                a = a.wrapping_add(0x60);
                c = true;
            }
            if h || a & 0x0F > 0x09 {
                a = a.wrapping_add(0x06);
            }
        } else {
            if c {
                a = a.wrapping_sub(0x60);
            }
            if h {
                a = a.wrapping_sub(0x06);
            }
        }
        self.regs.a = a;
        self.regs.set_flags(a == 0, n, false, c);
    }

    fn rlc(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(1);
        self.regs.set_flags(r == 0, false, false, v & 0x80 != 0);
        r
    }
    fn rrc(&mut self, v: u8) -> u8 {
        let r = v.rotate_right(1);
        self.regs.set_flags(r == 0, false, false, v & 1 != 0);
        r
    }
    fn rl(&mut self, v: u8) -> u8 {
        let r = (v << 1) | u8::from(self.regs.flag(FLAG_C));
        self.regs.set_flags(r == 0, false, false, v & 0x80 != 0);
        r
    }
    fn rr(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (u8::from(self.regs.flag(FLAG_C)) << 7);
        self.regs.set_flags(r == 0, false, false, v & 1 != 0);
        r
    }
    fn sla(&mut self, v: u8) -> u8 {
        let r = v << 1;
        self.regs.set_flags(r == 0, false, false, v & 0x80 != 0);
        r
    }
    fn sra(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (v & 0x80);
        self.regs.set_flags(r == 0, false, false, v & 1 != 0);
        r
    }
    fn swap(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(4);
        self.regs.set_flags(r == 0, false, false, false);
        r
    }
    fn srl(&mut self, v: u8) -> u8 {
        let r = v >> 1;
        self.regs.set_flags(r == 0, false, false, v & 1 != 0);
        r
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    fn machine(program: &[u8]) -> (Cpu, Bus) {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x100 + program.len()].copy_from_slice(program);
        (Cpu::new(), Bus::new(Cartridge::new(rom).unwrap(), 48_000))
    }

    /// Runs `program` from 0x0100 on a bare ROM-only cartridge until PC
    /// falls off its end. Returns the CPU, the bus and the T-cycles spent.
    fn run(program: &[u8], setup: impl FnOnce(&mut Cpu, &mut Bus)) -> (Cpu, Bus, u64) {
        let (mut cpu, mut bus) = machine(program);
        setup(&mut cpu, &mut bus);
        let start = bus.cycles;
        let end = 0x100 + program.len() as u16;
        while cpu.regs.pc != end {
            cpu.step(&mut bus);
        }
        let spent = bus.cycles - start;
        (cpu, bus, spent)
    }

    fn flags(cpu: &Cpu) -> String {
        let f = cpu.regs.f;
        [(FLAG_Z, "Z"), (FLAG_N, "N"), (FLAG_H, "H"), (FLAG_C, "C")]
            .iter()
            .map(|&(mask, name)| if f & mask != 0 { name } else { "-" })
            .collect()
    }

    #[test]
    fn daa_after_addition() {
        // 45 + 38 = 83 in BCD
        let (cpu, ..) = run(&[0x3E, 0x45, 0xC6, 0x38, 0x27], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x83, "----"));
        // 99 + 01 = 100: wraps to 00 with carry
        let (cpu, ..) = run(&[0x3E, 0x99, 0xC6, 0x01, 0x27], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x00, "Z--C"));
        // 09 + 08 = 17: the binary sum 0x11 has H set and needs +6
        let (cpu, ..) = run(&[0x3E, 0x09, 0xC6, 0x08, 0x27], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x17, "----"));
    }

    #[test]
    fn daa_after_subtraction() {
        // 42 - 09 = 33: the binary result 0x39 borrowed from the low nibble
        let (cpu, ..) = run(&[0x3E, 0x42, 0xD6, 0x09, 0x27], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x33, "-N--"));
        // 10 - 20 = 90 with borrow
        let (cpu, ..) = run(&[0x3E, 0x10, 0xD6, 0x20, 0x27], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x90, "-N-C"));
    }

    #[test]
    fn add_sp_e_takes_flags_from_the_low_byte() {
        // SP=0x00FF, ADD SP,+1: carries out of bits 3 and 7.
        let (cpu, _, cycles) = run(&[0xE8, 0x01], |c, _| c.regs.sp = 0x00FF);
        assert_eq!((cpu.regs.sp, flags(&cpu).as_str()), (0x0100, "--HC"));
        assert_eq!(cycles, 16);
        // SP=0x0FF8, ADD SP,-8: the unsigned F8+F8 carries both ways even
        // though the signed result goes down. Z is always cleared.
        let (cpu, ..) = run(&[0xAF, 0xE8, 0xF8], |c, _| c.regs.sp = 0x0FF8);
        assert_eq!((cpu.regs.sp, flags(&cpu).as_str()), (0x0FF0, "--HC"));
        // SP=0x1000, ADD SP,-1: no carries at all.
        let (cpu, ..) = run(&[0xE8, 0xFF], |c, _| c.regs.sp = 0x1000);
        assert_eq!((cpu.regs.sp, flags(&cpu).as_str()), (0x0FFF, "----"));
    }

    #[test]
    fn ld_hl_sp_e_matches_add_sp_flags() {
        let (cpu, _, cycles) = run(&[0xF8, 0x01], |c, _| c.regs.sp = 0x00FF);
        assert_eq!((cpu.regs.hl(), cpu.regs.sp, flags(&cpu).as_str()), (0x0100, 0x00FF, "--HC"));
        assert_eq!(cycles, 12);
    }

    #[test]
    fn half_carry_rules() {
        // ADD: carry out of bit 3
        let (cpu, ..) = run(&[0x3E, 0x0F, 0xC6, 0x01], |_, _| {});
        assert_eq!(flags(&cpu), "--H-");
        // ADC counts the incoming carry in the nibble sum: 0E + 01 + 1
        let (cpu, ..) = run(&[0x37, 0x3E, 0x0E, 0xCE, 0x01], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x10, "--H-"));
        // SUB: borrow from bit 4
        let (cpu, ..) = run(&[0x3E, 0x10, 0xD6, 0x01], |_, _| {});
        assert_eq!(flags(&cpu), "-NH-");
        // SBC with carry: 10 - 0F - 1 = 00, borrowing into the low nibble
        let (cpu, ..) = run(&[0x37, 0x3E, 0x10, 0xDE, 0x0F], |_, _| {});
        assert_eq!((cpu.regs.a, flags(&cpu).as_str()), (0x00, "ZNH-"));
        // INC leaves C alone
        let (cpu, ..) = run(&[0x37, 0x3E, 0x0F, 0x3C], |_, _| {});
        assert_eq!(flags(&cpu), "--HC");
        // ADD HL,rr: H comes from bit 11, Z is preserved
        let (cpu, ..) = run(&[0xAF, 0x21, 0xFF, 0x0F, 0x01, 0x01, 0x00, 0x09], |_, _| {});
        assert_eq!((cpu.regs.hl(), flags(&cpu).as_str()), (0x1000, "Z-H-"));
    }

    #[test]
    fn pop_af_masks_low_flag_bits() {
        // LD BC,0x12FF; PUSH BC; POP AF
        let (cpu, ..) = run(&[0x01, 0xFF, 0x12, 0xC5, 0xF1], |_, _| {});
        assert_eq!((cpu.regs.a, cpu.regs.f), (0x12, 0xF0));
    }

    #[test]
    fn instruction_timing_comes_from_memory_accesses() {
        let cases: &[(&[u8], u64)] = &[
            (&[0x00], 4),              // NOP
            (&[0x01, 0x34, 0x12], 12), // LD BC,nn
            (&[0x03], 8),              // INC BC (internal cycle)
            (&[0xC5], 16),             // PUSH BC
            (&[0x18, 0x00], 12),       // JR +0
            (&[0x20, 0x00], 8),        // JR NZ, not taken (Z is set after boot)
            (&[0x36, 0x00], 12),       // LD (HL),n
            (&[0xCB, 0x46], 12),       // BIT 0,(HL)
            (&[0xCB, 0xC6], 16),       // SET 0,(HL)
            (&[0x08, 0x00, 0xC0], 20), // LD (nn),SP
        ];
        for &(program, expected) in cases {
            let (.., cycles) = run(program, |c, _| c.regs.set_hl(0xC000));
            assert_eq!(cycles, expected, "program {program:02X?}");
        }
    }

    #[test]
    fn call_and_ret_round_trip() {
        // 0100: CALL 0108 / 0103: JR +4 / ... / 0108: RET
        let (mut cpu, mut bus) = machine(&[0xCD, 0x08, 0x01, 0x18, 0x04, 0x00, 0x00, 0x00, 0xC9]);
        let t0 = bus.cycles;
        cpu.step(&mut bus);
        assert_eq!((cpu.regs.pc, cpu.regs.sp, bus.cycles - t0), (0x0108, 0xFFFC, 24));
        let t1 = bus.cycles;
        cpu.step(&mut bus);
        assert_eq!((cpu.regs.pc, cpu.regs.sp, bus.cycles - t1), (0x0103, 0xFFFE, 16));
    }

    #[test]
    fn halt_bug_executes_the_next_byte_twice() {
        // IME=0 with an interrupt already pending: HALT does not halt, and
        // the INC A after it runs twice because PC fails to advance.
        let (cpu, ..) = run(&[0xAF, 0x76, 0x3C], |_, bus| {
            bus.ie = 0x04;
            bus.if_reg = 0x04;
        });
        assert_eq!(cpu.regs.a, 2);
        assert!(!cpu.halted);
    }

    #[test]
    fn ei_takes_effect_after_the_next_instruction() {
        // EI; DI: the interrupt window never opens.
        let (cpu, ..) = run(&[0xFB, 0xF3, 0x00], |_, bus| {
            bus.ie = 0x04;
            bus.if_reg = 0x04;
        });
        assert_eq!(cpu.regs.pc, 0x0103, "no interrupt was taken");

        // EI; NOP: dispatched right after the NOP, to the timer vector.
        let (mut cpu, mut bus) = machine(&[0xFB, 0x00, 0x00]);
        bus.ie = 0x04;
        bus.if_reg = 0x04;
        cpu.step(&mut bus); // EI
        cpu.step(&mut bus); // NOP
        assert_eq!(cpu.regs.pc, 0x0102);
        let t = bus.cycles;
        cpu.step(&mut bus); // dispatch
        assert_eq!((cpu.regs.pc, bus.cycles - t, bus.if_reg & 0x04), (0x0050, 20, 0));
    }
}
