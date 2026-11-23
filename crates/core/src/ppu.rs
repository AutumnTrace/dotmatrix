//! Pixel processing unit.
//!
//! Timing is tracked per dot (4 per M-cycle) so STAT mode changes, LY and
//! the STAT interrupt line move at the right moments, including the
//! variable-length mode 3 (SCX fine scroll, window start and the per-object
//! fetch penalty from Pan Docs). Pixels, on the other hand, are produced a
//! whole scanline at a time when mode 3 ends. That is enough for every
//! game that only changes registers between lines, and for dmg-acid2, but
//! not for mid-scanline raster tricks (see the Mealybug tests).

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;
pub const DOTS_PER_LINE: u16 = 456;
pub const LINES_PER_FRAME: u8 = 154;

pub const IRQ_VBLANK: u8 = 0x01;
pub const IRQ_STAT: u8 = 0x02;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    HBlank = 0,
    VBlank = 1,
    OamScan = 2,
    Drawing = 3,
}

#[derive(Clone, Copy, Debug, Default)]
struct Sprite {
    y: u8,
    x: u8,
    tile: u8,
    flags: u8,
}

pub struct Ppu {
    pub(crate) vram: Box<[u8; 0x2000]>,
    pub(crate) oam: [u8; 0xA0],
    lcdc: u8,
    /// STAT bits 3-6 (interrupt selects); the rest of STAT is computed.
    stat_select: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,

    mode: Mode,
    /// Dot within the current line, 0..456.
    dot: u16,
    /// Internal line counter. Differs from `ly` only during line 153.
    line: u8,
    mode3_end: u16,
    lyc_equal: bool,
    /// Dots left during which the LY=LYC flag reads 0 after LY changed.
    lyc_hold: u8,
    stat_line: bool,
    /// Set once WY == LY has been seen this frame; the window can only start
    /// on lines after that.
    wy_triggered: bool,
    window_line: u8,

    sprites: [Sprite; 10],
    sprite_count: usize,

    framebuffer: Box<[u8; WIDTH * HEIGHT]>,
    frame_ready: bool,
}

impl Ppu {
    pub fn new() -> Self {
        let mut ppu = Self {
            vram: Box::new([0; 0x2000]),
            oam: [0; 0xA0],
            lcdc: 0x91,
            stat_select: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            mode: Mode::VBlank,
            dot: 0,
            line: 0,
            mode3_end: 0,
            lyc_equal: true,
            lyc_hold: 0,
            stat_line: false,
            wy_triggered: false,
            window_line: 0,
            sprites: [Sprite::default(); 10],
            sprite_count: 0,
            framebuffer: Box::new([0; WIDTH * HEIGHT]),
            frame_ready: false,
        };
        // The boot ROM hands over during line 0 with the LCD on; start a
        // little before the end of VBlank so the first frame is a full one.
        ppu.line = 153;
        ppu.ly = 0;
        ppu.dot = 400;
        ppu
    }

    pub fn framebuffer(&self) -> &[u8; WIDTH * HEIGHT] {
        &self.framebuffer
    }

    pub(crate) fn take_frame_ready(&mut self) -> bool {
        std::mem::take(&mut self.frame_ready)
    }

    pub fn lcd_enabled(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn ly(&self) -> u8 {
        self.ly
    }

    /// Advances the PPU by one M-cycle (4 dots) and returns requested
    /// interrupts. Most M-cycles contain no mode or LY change, so those just
    /// move the dot counter; the rest go through [`Ppu::step`] dot by dot.
    pub fn tick(&mut self) -> u8 {
        if !self.lcd_enabled() {
            return 0;
        }
        if self.lyc_hold == 0 && self.dot + 4 < self.next_event() {
            self.dot += 4;
            return 0;
        }
        let mut irq = 0;
        for _ in 0..4 {
            irq |= self.step();
        }
        irq
    }

    /// The next dot in this line at which some PPU state changes.
    fn next_event(&self) -> u16 {
        let d = self.dot;
        match self.line {
            0..=143 => {
                if d < 80 {
                    80
                } else if d < self.mode3_end {
                    self.mode3_end
                } else if d < 452 {
                    452
                } else {
                    DOTS_PER_LINE
                }
            }
            // The DMG's extra mode 2 STAT source is live for the first 4 dots
            // of VBlank; on line 153 LY drops to 0 at dot 4.
            144 | 153 if d < 4 => 4,
            153 if d < 8 => 8,
            153 => DOTS_PER_LINE,
            _ if d < 452 => 452,
            _ => DOTS_PER_LINE,
        }
    }

    /// Advances the PPU by one dot and returns requested interrupts.
    pub fn step(&mut self) -> u8 {
        if !self.lcd_enabled() {
            return 0;
        }
        let mut irq = 0;
        self.lyc_hold = self.lyc_hold.saturating_sub(1);
        self.dot += 1;
        if self.dot == DOTS_PER_LINE {
            self.dot = 0;
            self.line += 1;
            if self.line == LINES_PER_FRAME {
                self.line = 0;
                self.window_line = 0;
                self.wy_triggered = false;
            }
            self.ly = self.line;
            if self.line < HEIGHT as u8 {
                self.mode = Mode::OamScan;
            } else if self.line == HEIGHT as u8 {
                self.mode = Mode::VBlank;
                irq |= IRQ_VBLANK;
                self.frame_ready = true;
            }
        }
        // LY moves on a few dots before the line actually ends; on line 153
        // it reads 0 almost immediately.
        // The LY=LYC flag drops whenever LY changes and is only re-evaluated
        // 4 dots later.
        if self.dot == 452 && self.line < 153 {
            self.ly = self.line + 1;
            self.lyc_hold = 4;
        }
        if self.line == 153 && self.dot == 4 {
            self.ly = 0;
            self.lyc_hold = 4;
        }
        if self.line < HEIGHT as u8 {
            if self.dot == 80 {
                if self.wy == self.ly {
                    self.wy_triggered = true;
                }
                self.oam_scan();
                self.mode3_end = 80 + self.mode3_length();
                self.mode = Mode::Drawing;
            } else if self.dot == self.mode3_end && self.mode == Mode::Drawing {
                self.render_line();
                self.mode = Mode::HBlank;
            }
        }
        if self.update_stat_line() {
            irq |= IRQ_STAT;
        }
        irq
    }

    /// Recomputes the STAT interrupt line; returns true on a rising edge.
    /// Because all sources are OR-ed into one line, a new source becoming
    /// active while another one already holds the line high does not cause a
    /// second interrupt ("STAT blocking").
    fn update_stat_line(&mut self) -> bool {
        self.lyc_equal = self.lyc_hold == 0 && self.ly == self.lyc;
        let s = self.stat_select;
        let mode_source = match self.mode {
            Mode::HBlank => s & 0x08 != 0,
            // On DMG the mode 2 select also fires when VBlank begins.
            Mode::VBlank => s & 0x10 != 0 || (self.line == 144 && self.dot < 4 && s & 0x20 != 0),
            Mode::OamScan => s & 0x20 != 0,
            Mode::Drawing => false,
        };
        let line = mode_source || (s & 0x40 != 0 && self.lyc_equal);
        let rising = line && !self.stat_line;
        self.stat_line = line;
        rising
    }

    fn oam_scan(&mut self) {
        let height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let target = u16::from(self.ly) + 16;
        self.sprite_count = 0;
        for entry in self.oam.chunks_exact(4) {
            let y = u16::from(entry[0]);
            if target >= y && target < y + height {
                self.sprites[self.sprite_count] = Sprite { y: entry[0], x: entry[1], tile: entry[2], flags: entry[3] };
                self.sprite_count += 1;
                if self.sprite_count == 10 {
                    break;
                }
            }
        }
        // Lower X wins; on a tie the lower OAM index (scan order) wins. The
        // sort is stable, so scan order survives for equal X.
        self.sprites[..self.sprite_count].sort_by_key(|s| s.x);
    }

    fn window_visible_on_line(&self) -> bool {
        self.lcdc & 0x21 == 0x21 && self.wy_triggered && self.wx <= 166
    }

    /// Mode 3 duration in dots: 172 + SCX%8 + window setup + object fetches.
    fn mode3_length(&self) -> u16 {
        let mut len = 172 + u16::from(self.scx & 7);
        let window = self.window_visible_on_line();
        if window {
            len += 6;
        }
        if self.lcdc & 0x02 != 0 {
            let win_start = i16::from(self.wx) - 7;
            let mut seen_tiles: [i16; 10] = [i16::MIN; 10];
            let mut penalty = 0u16;
            for (i, s) in self.sprites[..self.sprite_count].iter().enumerate() {
                if s.x >= 168 {
                    continue;
                }
                let px = i16::from(s.x) - 8;
                let (tile, offset) = if s.x == 0 {
                    (-1, 0)
                } else if window && px >= win_start {
                    let wx = px - win_start;
                    (0x100 + wx / 8, wx % 8)
                } else {
                    let bx = i16::from(s.x) + i16::from(self.scx);
                    (bx / 8, bx % 8)
                };
                if !seen_tiles[..i].contains(&tile) {
                    penalty += (5 - offset).max(0) as u16;
                }
                seen_tiles[i] = tile;
                penalty += 6;
            }
            // Pan Docs derives these penalties from mid-line palette writes.
            // Measured through the STAT mode 0 interrupt instead (Mooneye's
            // intr_2_mode0_timing_sprites), every case lines up with our
            // SCX timing only if the total is 3 dots shorter.
            len += penalty.saturating_sub(3);
        }
        len.min(289)
    }

    fn tile_row(&self, tile_addr: usize, row: usize) -> (u8, u8) {
        let a = tile_addr + row * 2;
        (self.vram[a], self.vram[a + 1])
    }

    fn bg_tile_addr(&self, tile: u8) -> usize {
        if self.lcdc & 0x10 != 0 { usize::from(tile) * 16 } else { (0x1000 + i32::from(tile as i8) * 16) as usize }
    }

    fn render_line(&mut self) {
        let ly = usize::from(self.ly);
        let mut bg_color = [0u8; WIDTH];

        if self.lcdc & 0x01 != 0 {
            let window = self.window_visible_on_line();
            let win_start = if window { i16::from(self.wx) - 7 } else { WIDTH as i16 };
            let bg_map = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
            let win_map = if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
            let bg_y = usize::from(self.ly.wrapping_add(self.scy));
            let win_y = usize::from(self.window_line);

            for (x, out) in bg_color.iter_mut().enumerate() {
                let (map, px, py) = if (x as i16) >= win_start {
                    (win_map, (x as i16 - win_start) as usize, win_y)
                } else {
                    (bg_map, (x + usize::from(self.scx)) & 0xFF, bg_y)
                };
                let tile = self.vram[map + (py / 8) * 32 + px / 8];
                let (lo, hi) = self.tile_row(self.bg_tile_addr(tile), py & 7);
                let bit = 7 - (px & 7);
                *out = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
            }
            if win_start < WIDTH as i16 {
                self.window_line += 1;
            }
        }

        let row = &mut self.framebuffer[ly * WIDTH..(ly + 1) * WIDTH];
        let bgp = self.bgp;
        let bg_on = self.lcdc & 0x01 != 0;
        for (out, &c) in row.iter_mut().zip(bg_color.iter()) {
            *out = if bg_on { (bgp >> (c * 2)) & 3 } else { 0 };
        }

        if self.lcdc & 0x02 == 0 {
            return;
        }
        let tall = self.lcdc & 0x04 != 0;
        let height = if tall { 16 } else { 8 };
        let mut claimed = [false; WIDTH];
        for s in &self.sprites[..self.sprite_count] {
            let mut sprite_row = (ly + 16 - usize::from(s.y)) as u8;
            if s.flags & 0x40 != 0 {
                sprite_row = height - 1 - sprite_row;
            }
            let tile = if tall { s.tile & 0xFE } else { s.tile };
            let addr = usize::from(tile) * 16 + usize::from(sprite_row) * 2;
            let (lo, hi) = (self.vram[addr], self.vram[addr + 1]);
            let palette = if s.flags & 0x10 != 0 { self.obp1 } else { self.obp0 };
            for px in 0..8u8 {
                let x = i16::from(s.x) - 8 + i16::from(px);
                if !(0..WIDTH as i16).contains(&x) {
                    continue;
                }
                let x = x as usize;
                if claimed[x] {
                    continue;
                }
                let bit = if s.flags & 0x20 != 0 { px } else { 7 - px };
                let color = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                if color == 0 {
                    continue;
                }
                // The highest-priority opaque object owns this pixel even
                // if it then loses to the background.
                claimed[x] = true;
                if s.flags & 0x80 != 0 && bg_color[x] != 0 {
                    continue;
                }
                row[x] = (palette >> (color * 2)) & 3;
            }
        }
    }

    /// VRAM locks 4 dots before STAT reports mode 3 (except on the first
    /// line after the LCD is switched on, which has no OAM scan).
    fn vram_blocked(&self) -> bool {
        self.lcd_enabled() && (self.mode == Mode::Drawing || (self.mode == Mode::OamScan && self.dot >= 76))
    }

    /// OAM locks 4 dots before the next visible line starts its OAM scan.
    pub(crate) fn oam_blocked(&self) -> bool {
        self.lcd_enabled()
            && (matches!(self.mode, Mode::OamScan | Mode::Drawing)
                || (self.dot >= 452 && (self.line < HEIGHT as u8 - 1 || self.line == LINES_PER_FRAME - 1)))
    }

    pub fn read_vram(&self, addr: u16) -> u8 {
        if self.vram_blocked() { 0xFF } else { self.vram[usize::from(addr & 0x1FFF)] }
    }

    /// Writes are locked out for a shorter window than reads: lcdon_write_timing
    /// shows VRAM writes landing right up to the start of mode 3, and OAM
    /// writes landing in the last M-cycle of OAM scan and the 4 dots before it.
    fn vram_write_blocked(&self) -> bool {
        self.lcd_enabled() && self.mode == Mode::Drawing
    }

    fn oam_write_blocked(&self) -> bool {
        self.lcd_enabled() && (self.mode == Mode::Drawing || (self.mode == Mode::OamScan && self.dot < 76))
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        if !self.vram_write_blocked() {
            self.vram[usize::from(addr & 0x1FFF)] = value;
        }
    }

    pub fn read_oam(&self, addr: u16) -> u8 {
        if self.oam_blocked() { 0xFF } else { self.oam[usize::from(addr & 0xFF)] }
    }

    pub fn write_oam(&mut self, addr: u16, value: u8) {
        if !self.oam_write_blocked() {
            self.oam[usize::from(addr & 0xFF)] = value;
        }
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => {
                let mode = if self.lcd_enabled() { self.mode as u8 } else { 0 };
                0x80 | self.stat_select | (u8::from(self.lyc_equal) << 2) | mode
            }
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => 0xFF,
        }
    }

    /// Returns interrupts requested as a side effect of the write.
    pub fn write_register(&mut self, addr: u16, value: u8) -> u8 {
        match addr {
            0xFF40 => {
                let was_on = self.lcd_enabled();
                self.lcdc = value;
                if was_on && !self.lcd_enabled() {
                    self.ly = 0;
                    self.line = 0;
                    self.dot = 0;
                    self.mode = Mode::HBlank;
                    // The LY=LYC flag and the STAT line keep their last values
                    // while the comparison clock is stopped.
                    self.framebuffer.fill(0);
                    self.frame_ready = true;
                } else if !was_on && self.lcd_enabled() {
                    self.ly = 0;
                    self.line = 0;
                    self.dot = 0;
                    self.mode = Mode::HBlank;
                    self.wy_triggered = false;
                    self.window_line = 0;
                    if self.update_stat_line() {
                        return IRQ_STAT;
                    }
                }
            }
            0xFF41 => {
                // DMG quirk: a STAT write behaves as if 0xFF was written for
                // one cycle, which can raise a spurious interrupt.
                let mut irq = 0;
                if self.lcd_enabled() {
                    self.stat_select = 0x78;
                    if self.update_stat_line() {
                        irq = IRQ_STAT;
                    }
                }
                self.stat_select = value & 0x78;
                if self.lcd_enabled() {
                    self.update_stat_line();
                }
                return irq;
            }
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF44 => {}
            0xFF45 => {
                self.lyc = value;
                if self.lcd_enabled() && self.update_stat_line() {
                    return IRQ_STAT;
                }
            }
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            _ => {}
        }
        0
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}
