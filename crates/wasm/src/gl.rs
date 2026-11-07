//! The shape layer: every triangle of a frame in one buffer, drawn in order —
//! on the GPU where the browser gives a worker WebGL2, and with a 2D canvas
//! where it does not (iOS before 17, WebKit without a GPU). The 2D path is
//! slower but draws the same frame, so a chart opens everywhere.

use tradoor_render::Frame;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{OffscreenCanvas, WebGl2RenderingContext as GL, WebGlBuffer, WebGlProgram, WebGlShader, WebGlUniformLocation, WebGlVertexArrayObject, WebglLoseContext};

const VERTEX: &str = "#version 300 es
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_color;
uniform vec2 u_size;
out vec4 v_color;
void main() {
  vec2 clip = a_pos / u_size * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
  v_color = a_color;
}";

const FRAGMENT: &str = "#version 300 es
precision mediump float;
in vec4 v_color;
out vec4 color;
void main() { color = v_color; }";

pub struct Gl {
    canvas: OffscreenCanvas,
    gl: GL,
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    positions: WebGlBuffer,
    colors: WebGlBuffer,
    u_size: Option<WebGlUniformLocation>,
    capacity: usize,
}

/// A browser allows a page about sixteen live WebGL contexts and quietly kills the
/// oldest past that. A context otherwise lives until the collector finds its
/// canvas, long after its chart is gone — so a few tab switches blank a chart.
/// Giving it back the moment the chart goes keeps the count to the charts shown.
impl Drop for Gl {
    fn drop(&mut self) {
        if let Ok(Some(ext)) = self.gl.get_extension("WEBGL_lose_context") {
            ext.unchecked_into::<WebglLoseContext>().lose_context();
        }
    }
}

fn compile(gl: &GL, kind: u32, source: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl.create_shader(kind).ok_or("could not create a shader")?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);
    if gl.get_shader_parameter(&shader, GL::COMPILE_STATUS).as_bool() != Some(true) {
        return Err(JsValue::from_str(&gl.get_shader_info_log(&shader).unwrap_or_default()));
    }
    Ok(shader)
}

/// The program, vertex array and buffers a frame is drawn with: made with the
/// context, and again when a lost context is given back.
type Parts = (WebGlProgram, WebGlVertexArrayObject, WebGlBuffer, WebGlBuffer, Option<WebGlUniformLocation>);

fn parts(gl: &GL) -> Result<Parts, JsValue> {
    let program = gl.create_program().ok_or("could not create a program")?;
    gl.attach_shader(&program, &compile(gl, GL::VERTEX_SHADER, VERTEX)?);
    gl.attach_shader(&program, &compile(gl, GL::FRAGMENT_SHADER, FRAGMENT)?);
    gl.link_program(&program);
    if gl.get_program_parameter(&program, GL::LINK_STATUS).as_bool() != Some(true) {
        return Err(JsValue::from_str(&gl.get_program_info_log(&program).unwrap_or_default()));
    }

    let vao = gl.create_vertex_array().ok_or("could not create a vertex array")?;
    gl.bind_vertex_array(Some(&vao));
    let positions = gl.create_buffer().ok_or("could not create a buffer")?;
    gl.bind_buffer(GL::ARRAY_BUFFER, Some(&positions));
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_with_i32(0, 2, GL::FLOAT, false, 0, 0);
    let colors = gl.create_buffer().ok_or("could not create a buffer")?;
    gl.bind_buffer(GL::ARRAY_BUFFER, Some(&colors));
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_with_i32(1, 4, GL::UNSIGNED_BYTE, true, 0, 0);

    gl.enable(GL::BLEND);
    gl.blend_func_separate(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA, GL::ONE, GL::ONE_MINUS_SRC_ALPHA);
    let u_size = gl.get_uniform_location(&program, "u_size");
    Ok((program, vao, positions, colors, u_size))
}

impl Gl {
    pub fn new(canvas: OffscreenCanvas) -> Result<Self, JsValue> {
        let options = js_sys::Object::new();
        for (key, value) in [("antialias", true), ("alpha", false), ("depth", false), ("stencil", false), ("preserveDrawingBuffer", false)] {
            js_sys::Reflect::set(&options, &key.into(), &value.into())?;
        }
        let gl: GL = canvas.get_context_with_context_options("webgl2", &options)?.ok_or("WebGL2 is not available in this browser")?.dyn_into()?;
        let (program, vao, positions, colors, u_size) = parts(&gl)?;
        Ok(Self { canvas, gl, program, vao, positions, colors, u_size, capacity: 0 })
    }

    /// The context back after the browser took it — iOS takes a page's when it
    /// is put away or short of memory. Nothing made on it before survives, so
    /// it is all made again, and the next frame fills the buffers anew.
    pub fn restore(&mut self) -> Result<(), JsValue> {
        (self.program, self.vao, self.positions, self.colors, self.u_size) = parts(&self.gl)?;
        self.capacity = 0;
        Ok(())
    }

    pub fn draw(&mut self, frame: &Frame) {
        let gl = &self.gl;
        // A lost context draws nothing, whatever it is asked: the frame waits for it to come back.
        if gl.is_context_lost() {
            return;
        }
        let (w, h) = ((frame.width * frame.dpr).round().max(1.0) as u32, (frame.height * frame.dpr).round().max(1.0) as u32);
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(GL::SCISSOR_TEST);
        let bg = frame.background;
        gl.clear_color(f32::from(bg.r) / 255.0, f32::from(bg.g) / 255.0, f32::from(bg.b) / 255.0, 1.0);
        gl.clear(GL::COLOR_BUFFER_BIT);
        let vertices = frame.vertex_count();
        if vertices == 0 {
            return;
        }

        gl.use_program(Some(&self.program));
        gl.bind_vertex_array(Some(&self.vao));
        let grow = vertices > self.capacity;
        if grow {
            self.capacity = vertices + vertices / 2;
        }
        // SAFETY: the views are handed straight to buffer calls that copy them,
        // with no allocation in between that could move WASM memory.
        unsafe {
            let positions = js_sys::Float32Array::view(&frame.positions);
            let colors = js_sys::Uint8Array::view(&frame.colors);
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.positions));
            if grow {
                gl.buffer_data_with_i32(GL::ARRAY_BUFFER, (self.capacity * 8) as i32, GL::DYNAMIC_DRAW);
            }
            gl.buffer_sub_data_with_i32_and_array_buffer_view(GL::ARRAY_BUFFER, 0, &positions);
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.colors));
            if grow {
                gl.buffer_data_with_i32(GL::ARRAY_BUFFER, (self.capacity * 4) as i32, GL::DYNAMIC_DRAW);
            }
            gl.buffer_sub_data_with_i32_and_array_buffer_view(GL::ARRAY_BUFFER, 0, &colors);
        }
        gl.uniform2f(self.u_size.as_ref(), frame.width, frame.height);

        for batch in frame.batches.iter().filter(|b| b.count > 0) {
            match batch.clip {
                Some(c) => {
                    gl.enable(GL::SCISSOR_TEST);
                    let x = (c.x * frame.dpr).floor();
                    let y = ((frame.height - c.bottom()) * frame.dpr).floor();
                    gl.scissor(x as i32, y as i32, (c.w * frame.dpr).ceil().max(0.0) as i32, (c.h * frame.dpr).ceil().max(0.0) as i32);
                }
                None => gl.disable(GL::SCISSOR_TEST),
            }
            gl.draw_arrays(GL::TRIANGLES, batch.first as i32, batch.count as i32);
        }
    }
}

/// A frame's triangles on a 2D canvas: runs of one colour filled as one path,
/// so the two triangles of a rectangle meet without a seam.
pub struct Flat {
    canvas: OffscreenCanvas,
    ctx: web_sys::OffscreenCanvasRenderingContext2d,
}

impl Flat {
    pub fn new(canvas: OffscreenCanvas) -> Result<Self, JsValue> {
        let ctx = canvas.get_context("2d")?.ok_or("neither WebGL2 nor a 2D canvas is available in this browser")?.dyn_into()?;
        Ok(Self { canvas, ctx })
    }

    pub fn draw(&mut self, frame: &Frame) {
        let ctx = &self.ctx;
        let (w, h) = ((frame.width * frame.dpr).round().max(1.0) as u32, (frame.height * frame.dpr).round().max(1.0) as u32);
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        let dpr = f64::from(frame.dpr);
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.set_fill_style_str(&frame.background.css());
        ctx.fill_rect(0.0, 0.0, f64::from(frame.width), f64::from(frame.height));
        fill_triangles(ctx, frame);
    }
}

/// A frame's triangles filled on a 2D canvas as it stands, over what it holds.
pub fn fill_triangles(ctx: &web_sys::OffscreenCanvasRenderingContext2d, frame: &Frame) {
    let (p, c) = (&frame.positions, &frame.colors);
    for batch in frame.batches.iter().filter(|b| b.count > 0) {
        ctx.save();
        if let Some(clip) = batch.clip {
            ctx.begin_path();
            ctx.rect(f64::from(clip.x), f64::from(clip.y), f64::from(clip.w), f64::from(clip.h));
            ctx.clip();
        }
        let (first, last) = (batch.first as usize, (batch.first + batch.count) as usize);
        let mut run: Option<&[u8]> = None;
        for v in (first..last).step_by(3) {
            if v + 2 >= last {
                break;
            }
            // A triangle is one colour: the engine gives its three vertices the same.
            let color = &c[v * 4..v * 4 + 4];
            if run != Some(color) {
                if run.is_some() {
                    ctx.fill();
                }
                ctx.set_fill_style_str(&format!("rgba({},{},{},{})", color[0], color[1], color[2], tradoor_core::format::fixed(f64::from(color[3]) / 255.0, 3)));
                ctx.begin_path();
                run = Some(color);
            }
            let at = |i: usize| (f64::from(p[i * 2]), f64::from(p[i * 2 + 1]));
            let (a, b, d) = (at(v), at(v + 1), at(v + 2));
            // Every triangle turned the same way: a run is filled as one path by
            // the nonzero rule, where two that overlap turning opposite ways cancel
            // out. A line's joints are squares turning against it, and a brush
            // stroke came out of a picture in dashes.
            let (b, d) = if (b.0 - a.0) * (d.1 - a.1) < (b.1 - a.1) * (d.0 - a.0) { (d, b) } else { (b, d) };
            ctx.move_to(a.0, a.1);
            ctx.line_to(b.0, b.1);
            ctx.line_to(d.0, d.1);
            ctx.close_path();
        }
        if run.is_some() {
            ctx.fill();
        }
        ctx.restore();
    }
}

/// Whichever layer the browser allows.
pub enum Surface {
    Gpu(Gl),
    Flat(Flat),
}

impl Surface {
    pub fn new(canvas: OffscreenCanvas) -> Result<Self, JsValue> {
        match Gl::new(canvas.clone()) {
            Ok(gl) => Ok(Self::Gpu(gl)),
            // A refused WebGL2 context leaves the canvas free for a 2D one.
            Err(gpu) => Flat::new(canvas).map(Self::Flat).map_err(|_| gpu),
        }
    }

    pub fn draw(&mut self, frame: &Frame) {
        match self {
            Self::Gpu(gl) => gl.draw(frame),
            Self::Flat(flat) => flat.draw(frame),
        }
    }

    /// Made again on a context the browser gave back; a 2D canvas has nothing to lose.
    pub fn restore(&mut self) -> Result<(), JsValue> {
        match self {
            Self::Gpu(gl) => gl.restore(),
            Self::Flat(_) => Ok(()),
        }
    }

    /// `webgl2` or `2d`, for the status line and for tests.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Gpu(_) => "webgl2",
            Self::Flat(_) => "2d",
        }
    }
}
