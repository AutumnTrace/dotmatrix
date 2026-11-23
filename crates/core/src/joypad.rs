//! The P1/JOYP register: a 2x4 key matrix selected by bits 4 and 5.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

impl Button {
    pub const ALL: [Button; 8] =
        [Button::Right, Button::Left, Button::Up, Button::Down, Button::A, Button::B, Button::Select, Button::Start];

    /// Bit in the internal pressed mask: d-pad in the low nibble, buttons in
    /// the high nibble, each in P1 bit order.
    fn mask(self) -> u8 {
        1 << self as u8
    }

    pub fn from_name(name: &str) -> Option<Button> {
        Some(match name.to_ascii_lowercase().as_str() {
            "right" => Button::Right,
            "left" => Button::Left,
            "up" => Button::Up,
            "down" => Button::Down,
            "a" => Button::A,
            "b" => Button::B,
            "select" => Button::Select,
            "start" => Button::Start,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Joypad {
    pressed: u8,
    select: u8,
}

impl Joypad {
    pub fn read(&self) -> u8 {
        0xC0 | self.select | self.lines()
    }

    /// Low nibble of P1 (0 = pressed) for the currently selected rows.
    fn lines(&self) -> u8 {
        let mut low = 0;
        if self.select & 0x10 == 0 {
            low |= self.pressed & 0x0F;
        }
        if self.select & 0x20 == 0 {
            low |= self.pressed >> 4;
        }
        !low & 0x0F
    }

    /// Returns true if the write caused a joypad interrupt.
    pub fn write(&mut self, value: u8) -> bool {
        let before = self.lines();
        self.select = value & 0x30;
        before & !self.lines() != 0
    }

    /// Returns true if pressing caused a high-to-low transition on a
    /// selected line, which requests the joypad interrupt.
    pub fn set(&mut self, button: Button, pressed: bool) -> bool {
        let before = self.lines();
        if pressed {
            self.pressed |= button.mask();
        } else {
            self.pressed &= !button.mask();
        }
        before & !self.lines() != 0
    }

    pub fn any_pressed(&self) -> bool {
        self.pressed != 0
    }
}
