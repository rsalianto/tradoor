//! RGBA colours and the CSS spellings a theme accepts.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    /// `0xrrggbb`, opaque.
    pub const fn hex(v: u32) -> Self {
        Self { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// `#rgb`, `#rrggbb`, `#rrggbbaa` or `rgba(r, g, b, a)` with `a` in 0–1.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix('#') {
            let v = |i: usize, len: usize| u8::from_str_radix(hex.get(i..i + len)?, 16).ok();
            return match hex.len() {
                3 => Some(Self { r: v(0, 1)? * 17, g: v(1, 1)? * 17, b: v(2, 1)? * 17, a: 255 }),
                6 => Some(Self { r: v(0, 2)?, g: v(2, 2)?, b: v(4, 2)?, a: 255 }),
                8 => Some(Self { r: v(0, 2)?, g: v(2, 2)?, b: v(4, 2)?, a: v(6, 2)? }),
                _ => None,
            };
        }
        let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let channel = |i: usize| parts.get(i)?.parse::<u8>().ok();
        let alpha = match parts.get(3) {
            Some(a) => (a.parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8,
            None => 255,
        };
        (parts.len() == 3 || parts.len() == 4).then(|| Some(Self { r: channel(0)?, g: channel(1)?, b: channel(2)?, a: alpha }))?
    }

    /// What a 2D canvas takes as `fillStyle`.
    pub fn css(self) -> String {
        format!("rgba({},{},{},{:.3})", self.r, self.g, self.b, f32::from(self.a) / 255.0)
    }

    /// Dark or light text, whichever reads on this background.
    pub fn contrast_text(self) -> Color {
        let luminance = 0.299 * f32::from(self.r) + 0.587 * f32::from(self.g) + 0.114 * f32::from(self.b);
        if luminance > 140.0 { Color::hex(0x0f0f0f) } else { Color::hex(0xf2f5f2) }
    }

    /// This colour laid over `under` at its own opacity: what a see-through fill shows.
    pub fn over(self, under: Color) -> Color {
        let t = f32::from(self.a) / 255.0;
        let ch = |a: u8, b: u8| (f32::from(b) + (f32::from(a) - f32::from(b)) * t).round() as u8;
        Color { r: ch(self.r, under.r), g: ch(self.g, under.g), b: ch(self.b, under.b), a: 255 }
    }

    /// Near-black or near-white, whichever stands out more against this colour
    /// by the contrast the eye reads (WCAG's relative luminance): the two are
    /// even at about 0.18, so a mid-tone fill gets dark text, a dim one light.
    pub fn ink(self) -> Color {
        let linear = |c: u8| {
            let v = f32::from(c) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let luminance = 0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b);
        if luminance > 0.179 { Color::hex(0x0f0f0f) } else { Color::hex(0xf2f5f2) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_is_whichever_of_black_and_white_reads_better() {
        // Fills taken from a footprint on the default theme: a mid green reads in
        // black, a dim one and a dim red in white.
        assert_eq!(Color::hex(0x758c63).ink(), Color::hex(0x0f0f0f));
        assert_eq!(Color::hex(0x434d3b).ink(), Color::hex(0xf2f5f2));
        assert_eq!(Color::hex(0x593531).ink(), Color::hex(0xf2f5f2));
        assert_eq!(Color::hex(0xffffff).ink(), Color::hex(0x0f0f0f));
        assert_eq!(Color::hex(0x2962ff).ink(), Color::hex(0xf2f5f2), "a strong blue takes white");
        let half = Color { a: 128, ..Color::hex(0xffffff) };
        assert_eq!(half.over(Color::hex(0x000000)), Color::hex(0x808080));
    }

    #[test]
    fn parses_css_spellings() {
        assert_eq!(Color::parse("#a4c588"), Some(Color::hex(0xa4c588)));
        assert_eq!(Color::parse("#a4c588cc"), Some(Color::hex(0xa4c588).with_alpha(0xcc)));
        assert_eq!(Color::parse("#fff"), Some(Color::hex(0xffffff)));
        assert_eq!(Color::parse("rgba(170, 253, 53, 0.2)"), Some(Color { r: 170, g: 253, b: 53, a: 51 }));
        assert_eq!(Color::parse("red"), None);
        assert_eq!(Color::hex(0xffffff).contrast_text(), Color::hex(0x0f0f0f));
    }
}
