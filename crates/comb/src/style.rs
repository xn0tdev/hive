//! Colours and text attributes. Truecolor only (RGB) plus the terminal default.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    /// The terminal's default foreground/background.
    Reset,
    Rgb(u8, u8, u8),
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color::Rgb(r, g, b)
    }
}

/// Text attributes, packed as a bitset so styles stay `Copy`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modifier(pub u16);

impl Modifier {
    pub const NONE: Modifier = Modifier(0);
    pub const BOLD: Modifier = Modifier(1 << 0);
    pub const DIM: Modifier = Modifier(1 << 1);
    pub const ITALIC: Modifier = Modifier(1 << 2);
    pub const UNDERLINE: Modifier = Modifier(1 << 3);
    pub const REVERSE: Modifier = Modifier(1 << 4);

    pub const fn contains(self, other: Modifier) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifier {
    type Output = Modifier;
    fn bitor(self, rhs: Modifier) -> Modifier {
        Modifier(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifier {
    fn bitor_assign(&mut self, rhs: Modifier) {
        self.0 |= rhs.0;
    }
}

/// A foreground colour, background colour, and attribute set. `None` colours
/// mean "inherit whatever is already there" when patched onto another style.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub mods: Modifier,
}

impl Style {
    pub const fn new() -> Self {
        Style {
            fg: None,
            bg: None,
            mods: Modifier::NONE,
        }
    }

    pub const fn fg(mut self, c: Color) -> Self {
        self.fg = Some(c);
        self
    }

    pub const fn bg(mut self, c: Color) -> Self {
        self.bg = Some(c);
        self
    }

    pub const fn add(mut self, m: Modifier) -> Self {
        self.mods = Modifier(self.mods.0 | m.0);
        self
    }

    pub const fn bold(self) -> Self {
        self.add(Modifier::BOLD)
    }

    pub const fn dim(self) -> Self {
        self.add(Modifier::DIM)
    }

    pub const fn italic(self) -> Self {
        self.add(Modifier::ITALIC)
    }

    /// Overlay `other` on top of `self`: its set colours win, its modifiers are
    /// added. Used to resolve a span's style against a surface default.
    pub fn patch(mut self, other: Style) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        self.mods |= other.mods;
        self
    }
}
