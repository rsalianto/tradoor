//! The text layer: a 2D canvas over the GPU one.

use tradoor_render::{Align, Baseline, Frame};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d as Ctx, Path2d};

/// One filled path of the venue's mark, with the group transform it sits under.
struct MarkPath {
    path: Path2d,
    fill: String,
    tx: f64,
    ty: f64,
    scale: f64,
}

/// An exchange's mark: its paths, and the side of the square they are drawn in.
#[derive(Default)]
struct Mark {
    paths: Vec<MarkPath>,
    size: f64,
}

pub struct TextLayer {
    canvas: OffscreenCanvas,
    ctx: Ctx,
    /// Numbered as the page numbers exchanges; a liquidation names its mark by index.
    marks: Vec<Mark>,
}

fn font(size: f32) -> String {
    format!("{}px ui-monospace, SFMono-Regular, Menlo, Consolas, \"Liberation Mono\", monospace", tradoor_core::format::plain(f64::from(size)))
}

impl TextLayer {
    pub fn new(canvas: OffscreenCanvas) -> Result<Self, JsValue> {
        let ctx: Ctx = canvas.get_context("2d")?.ok_or("2D canvas is not available")?.dyn_into()?;
        Ok(Self { canvas, ctx, marks: Vec::new() })
    }

    /// Exchange `index`'s mark, one path a line as `tx|ty|scale|fill|d`. An
    /// empty spec clears it, and a path the browser cannot parse is left out.
    pub fn set_mark(&mut self, index: u8, box_size: f32, spec: &str) {
        let index = usize::from(index);
        if self.marks.len() <= index {
            self.marks.resize_with(index + 1, Mark::default);
        }
        let paths = spec
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(5, '|');
                let tx = tradoor_core::format::parse_number(parts.next()?)?;
                let ty = tradoor_core::format::parse_number(parts.next()?)?;
                let scale = tradoor_core::format::parse_number(parts.next()?)?;
                let fill = parts.next()?.to_string();
                let path = Path2d::new_with_path_string(parts.next()?).ok()?;
                Some(MarkPath { path, fill, tx, ty, scale })
            })
            .collect();
        self.marks[index] = Mark { paths, size: f64::from(box_size).max(1.0) };
    }

    /// The exchange marks on the liquidation feed's rows.
    fn draw_marks(&self, frame: &Frame) {
        let ctx = &self.ctx;
        for icon in &frame.icons {
            let Some(mark) = self.marks.get(usize::from(icon.mark)).filter(|m| !m.paths.is_empty()) else { continue };
            ctx.save();
            if let Some(c) = icon.clip {
                ctx.begin_path();
                ctx.rect(f64::from(c.x), f64::from(c.y), f64::from(c.w), f64::from(c.h));
                ctx.clip();
            }
            ctx.set_global_alpha(f64::from(icon.alpha) / 255.0);
            let size = f64::from(icon.size);
            let _ = ctx.translate(f64::from(icon.x) - size / 2.0, f64::from(icon.y) - size / 2.0);
            let _ = ctx.scale(size / mark.size, size / mark.size);
            for p in &mark.paths {
                ctx.save();
                let _ = ctx.translate(p.tx, p.ty);
                let _ = ctx.scale(p.scale, p.scale);
                ctx.set_fill_style_str(&p.fill);
                ctx.fill_with_path_2d(&p.path);
                ctx.restore();
            }
            ctx.restore();
        }
        ctx.set_global_alpha(1.0);
    }

    /// Axis labels are monospace, so one glyph's width sizes every label.
    pub fn char_width(&self, size: f32) -> f32 {
        self.ctx.set_font(&font(size));
        self.ctx.measure_text("0").map_or(size * 0.6, |m| m.width() as f32)
    }

    pub fn draw(&mut self, frame: &Frame) {
        self.begin(frame, true);
        self.texts(frame);
        self.draw_marks(frame);
    }

    /// A frame of its own on this canvas — the liquidation feed's, over the
    /// chart: its shapes filled here rather than on the GPU, then its text and
    /// marks. Not `clear`ed, it goes over what the canvas holds, for a picture.
    pub fn draw_layer(&mut self, frame: &Frame, clear: bool) {
        self.begin(frame, clear);
        crate::gl::fill_triangles(&self.ctx, frame);
        self.texts(frame);
        self.draw_marks(frame);
    }

    /// The canvas at the frame's size and scale, and wiped if `clear`.
    fn begin(&self, frame: &Frame, clear: bool) {
        let (w, h) = ((frame.width * frame.dpr).round().max(1.0) as u32, (frame.height * frame.dpr).round().max(1.0) as u32);
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        let dpr = f64::from(frame.dpr);
        let _ = self.ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        if clear {
            self.ctx.clear_rect(0.0, 0.0, f64::from(frame.width), f64::from(frame.height));
        }
    }

    fn texts(&self, frame: &Frame) {
        let ctx = &self.ctx;
        let mut current = (0.0f32, None::<Align>, None::<Baseline>);
        for t in &frame.texts {
            if let Some(c) = t.clip {
                ctx.save();
                ctx.begin_path();
                ctx.rect(f64::from(c.x), f64::from(c.y), f64::from(c.w), f64::from(c.h));
                ctx.clip();
            }
            if t.clip.is_some() || current.0 != t.size {
                ctx.set_font(&font(t.size));
            }
            if t.clip.is_some() || current.1 != Some(t.align) {
                ctx.set_text_align(match t.align {
                    Align::Left => "left",
                    Align::Center => "center",
                    Align::Right => "right",
                });
            }
            if t.clip.is_some() || current.2 != Some(t.baseline) {
                ctx.set_text_baseline(match t.baseline {
                    Baseline::Top => "top",
                    Baseline::Middle => "middle",
                    Baseline::Bottom => "bottom",
                });
            }
            current = (t.size, Some(t.align), Some(t.baseline));
            let (x, y) = (f64::from(t.x), f64::from(t.y));
            if let Some(bg) = t.background {
                let width = ctx.measure_text(&t.text).map_or(0.0, |m| m.width());
                let left = match t.align {
                    Align::Left => x,
                    Align::Center => x - width / 2.0,
                    Align::Right => x - width,
                };
                let height = f64::from(t.size) + 4.0;
                let top = match t.baseline {
                    Baseline::Top => y - 2.0,
                    Baseline::Middle => y - height / 2.0,
                    Baseline::Bottom => y - height + 2.0,
                };
                ctx.set_fill_style_str(&bg.css());
                ctx.fill_rect(left - 3.0, top, width + 6.0, height);
            }
            ctx.set_fill_style_str(&t.color.css());
            let _ = ctx.fill_text(&t.text, x, y);
            if t.clip.is_some() {
                ctx.restore();
                current = (0.0, None, None);
            }
        }
    }
}
