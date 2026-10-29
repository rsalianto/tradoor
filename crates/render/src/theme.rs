//! Every colour the chart draws with. The defaults are rsalianto's trading-chart
//! palette; a workspace can override any of them by name.

use crate::color::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub text: Color,
    pub border: Color,
    /// Muted sage and clay, so the ribbon behind them is the brightest thing on the
    /// pane and "which side of the ribbon are we on" reads first.
    pub bull: Color,
    pub bear: Color,
    pub candle_alpha: u8,
    pub volume_alpha: u8,
    pub ema200: Color,
    pub ribbon_fast: Color,
    pub ribbon_slow: Color,
    pub ribbon_fill_alpha: u8,
    pub crosshair: Color,
    pub label_background: Color,
    pub countdown_background: Color,
    pub countdown_text: Color,
    pub study_palette: [Color; 6],
    /// Drawings are the reader's opinion and get a hue no computed series uses.
    pub drawing: Color,
    pub drawing_selected: Color,
    /// Price alert lines and their axis tags.
    pub alert: Color,
    pub drawing_fill: Color,
    pub handle: Color,
    pub risk: Color,
    pub risk_fill: Color,
    pub reward: Color,
    pub reward_fill: Color,
    /// TPO blocks inside the value area, outside it, and the point of control's
    /// row and line; every other TPO colour is worked out from these and the candles'.
    pub tpo: Color,
    pub tpo_outside: Color,
    pub tpo_poc: Color,
    pub font_px: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::hex(0x0f0f0f),
            text: Color::hex(0x7c847d),
            border: Color::hex(0x232823),
            bull: Color::hex(0xa4c588),
            bear: Color::hex(0xd97161),
            candle_alpha: 0xcc,
            volume_alpha: 0x40,
            ema200: Color::hex(0xffb03a),
            ribbon_fast: Color::hex(0xaafd35),
            ribbon_slow: Color::hex(0xff293a),
            ribbon_fill_alpha: 51,
            crosshair: Color::hex(0x7c847d),
            label_background: Color::hex(0x2a2f2a),
            countdown_background: Color::hex(0x2a2f2a),
            countdown_text: Color::hex(0xd7ded7),
            study_palette: [Color::hex(0x7fb2ff), Color::hex(0xc58fff), Color::hex(0xffd166), Color::hex(0x4ecdc4), Color::hex(0xff8fab), Color::hex(0x9ae66e)],
            drawing: Color::hex(0xff5fd2),
            drawing_selected: Color::hex(0xffa8e6),
            alert: Color::hex(0x22ab94),
            drawing_fill: Color::hex(0xff5fd2).with_alpha(26),
            handle: Color::hex(0xffffff),
            risk: Color::hex(0xd97161),
            risk_fill: Color::hex(0xd97161).with_alpha(36),
            reward: Color::hex(0xa4c588),
            reward_fill: Color::hex(0xa4c588).with_alpha(36),
            tpo: Color::hex(0x5f9f3a),
            tpo_outside: Color::hex(0x858c86),
            tpo_poc: Color::hex(0xe6f5d3),
            font_px: 11.0,
        }
    }
}

impl Theme {
    /// Set one entry by the name a workspace stores it under. Returns false for an
    /// unknown name or an unparseable value, leaving the theme unchanged.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        if let Some(i) = key.strip_prefix("study").and_then(|n| n.parse::<usize>().ok()) {
            return match (self.study_palette.get_mut(i), Color::parse(value)) {
                (Some(slot), Some(c)) => {
                    *slot = c;
                    true
                }
                _ => false,
            };
        }
        if key == "fontPx" {
            return value.parse::<f32>().ok().filter(|v| (8.0..=20.0).contains(v)).map(|v| self.font_px = v).is_some();
        }
        let Some(color) = Color::parse(value) else { return false };
        let slot = match key {
            "background" => &mut self.background,
            "text" => &mut self.text,
            "border" => &mut self.border,
            "bull" => &mut self.bull,
            "bear" => &mut self.bear,
            "ema200" => &mut self.ema200,
            "ribbonFast" => &mut self.ribbon_fast,
            "ribbonSlow" => &mut self.ribbon_slow,
            "crosshair" => &mut self.crosshair,
            "labelBackground" => &mut self.label_background,
            "drawing" => &mut self.drawing,
            "drawingSelected" => &mut self.drawing_selected,
            "alert" => &mut self.alert,
            "tpo" => &mut self.tpo,
            "tpoOutside" => &mut self.tpo_outside,
            "tpoPoc" => &mut self.tpo_poc,
            _ => return false,
        };
        *slot = color;
        true
    }
}
