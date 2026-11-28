//! Drawing on Linux, with the same API as the Windows renderer (`gfx.rs`): tiny-skia on the
//! CPU into a softbuffer surface on Sofa's X11 window, text from `linux::text` (Figtree through
//! swash), images decoded with `image` on a worker thread at the size they are drawn and kept
//! under the same budget, and Lucide icons from their SVG paths.
//!
//! The frame is kept in BGRA order, which is what X11 and softbuffer take (0RGB), so presenting
//! is one copy: tiny-skia gets every colour and every image with red and blue swapped. Blending
//! is the same either way round.

use crate::draw::{PathSink, Pt, icon_path_data, tone, walk_path};
pub use crate::draw::{Align, Color, Fit, Rect};
use crate::linux::text::Text;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use tiny_skia as sk;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub const WM_IMAGE: u32 = 0x8000 + 1; // a decoded image is ready

/// tiny-skia's colour for ours, red and blue swapped (the frame is BGRA).
fn sk_color(c: Color) -> sk::Color {
    let f = |v: f32| v.clamp(0.0, 1.0);
    sk::Color::from_rgba(f(c.2), f(c.1), f(c.0), f(c.3)).unwrap_or(sk::Color::TRANSPARENT)
}

fn paint(c: Color) -> sk::Paint<'static> {
    let mut p = sk::Paint::default();
    p.set_color(sk_color(c));
    p.anti_alias = true;
    p
}

/// `a * b / 255`, rounded.
fn mul(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

// ---------- Images ----------

#[derive(Clone, PartialEq, Eq, Hash)]
struct ImageKey {
    path: PathBuf,
    w: u32,
    h: u32,
    cover: bool,
}

struct Decoded {
    key: ImageKey,
    pixels: Option<(u32, u32, Vec<u8>)>,
    tone: Option<Color>,
}

enum Slot {
    Loading,
    Ready { px: sk::Pixmap, w: u32, h: u32, ready_at: f64, used: u64 },
    Failed,
}

/// Decodes images at the size they are drawn, off the UI thread.
fn decoder_thread(rx: Receiver<ImageKey>, tx: Sender<Decoded>) {
    for key in rx {
        let pixels = decode(&key);
        let tone = pixels.as_ref().and_then(|(w, h, px)| tone(px, *w, *h));
        if tx.send(Decoded { key, pixels, tone }).is_err() {
            break;
        }
        crate::post(0, WM_IMAGE, 0);
    }
}

/// The image at `key`'s size (never larger than the file), premultiplied BGRA.
fn decode(key: &ImageKey) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::ImageReader::open(&key.path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let (sw, sh) = (img.width(), img.height());
    if sw == 0 || sh == 0 {
        return None;
    }
    // Never upscale: draw-time scaling handles small sources.
    let s = if key.cover { (key.w as f32 / sw as f32).max(key.h as f32 / sh as f32) } else { (key.w as f32 / sw as f32).min(key.h as f32 / sh as f32) };
    let s = s.min(1.0);
    let (w, h) = (((sw as f32 * s).round() as u32).max(1), ((sh as f32 * s).round() as u32).max(1));
    let rgba = img.into_rgba8();
    let rgba = if (w, h) == (sw, sh) { rgba } else { image::imageops::thumbnail(&rgba, w, h) };
    let (w, h) = rgba.dimensions();
    let mut px = rgba.into_raw();
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        let (r, g, b) = (u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
        p[0] = mul(b, a) as u8;
        p[1] = mul(g, a) as u8;
        p[2] = mul(r, a) as u8;
    }
    Some((w, h, px))
}

// ---------- Paths ----------

/// SVG path data into a tiny-skia path.
#[derive(Default)]
struct Builder {
    pb: sk::PathBuilder,
    open: bool,
}

impl PathSink for Builder {
    fn begin(&mut self, p: Pt) {
        self.pb.move_to(p.0, p.1);
        self.open = true;
    }
    fn end(&mut self, closed: bool) {
        if self.open && closed {
            self.pb.close();
        }
        self.open = false;
    }
    fn line(&mut self, p: Pt) {
        self.pb.line_to(p.0, p.1);
    }
    fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt) {
        self.pb.cubic_to(c1.0, c1.1, c2.0, c2.1, p.0, p.1);
    }
    fn quad(&mut self, c1: Pt, p: Pt) {
        self.pb.quad_to(c1.0, c1.1, p.0, p.1);
    }
    fn arc(&mut self, from: Pt, to: Pt, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool) {
        arc_to(&mut self.pb, from, to, rx, ry, rotation, large, sweep);
    }
}

/// An SVG elliptical arc as cubic Béziers (SVG 1.1, appendix F.6).
#[allow(clippy::too_many_arguments)]
fn arc_to(pb: &mut sk::PathBuilder, from: Pt, to: Pt, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool) {
    let (x1, y1, x2, y2) = (from.0 as f64, from.1 as f64, to.0 as f64, to.1 as f64);
    let (mut rx, mut ry) = ((rx as f64).abs(), (ry as f64).abs());
    if rx < 1e-9 || ry < 1e-9 || (x1 == x2 && y1 == y2) {
        pb.line_to(to.0, to.1);
        return;
    }
    let phi = (rotation as f64).to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let (cxp, cyp) = (coef * rx * y1p / ry, -coef * ry * x1p / rx);
    let (cx, cy) = (cos * cxp - sin * cyp + (x1 + x2) / 2.0, sin * cxp + cos * cyp + (y1 + y2) / 2.0);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let a = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
        if a.is_nan() { 0.0 } else { a }
    };
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    let segments = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / segments as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |t: f64| {
        let (ct, st) = (t.cos(), t.sin());
        (cx + rx * ct * cos - ry * st * sin, cy + rx * ct * sin + ry * st * cos)
    };
    let deriv = |t: f64| {
        let (ct, st) = (t.cos(), t.sin());
        (-rx * st * cos - ry * ct * sin, -rx * st * sin + ry * ct * cos)
    };
    let mut t = theta;
    for i in 0..segments {
        let t2 = t + step;
        let (p1, d1) = (point(t), deriv(t));
        let (p2, d2) = (if i + 1 == segments { (x2, y2) } else { point(t2) }, deriv(t2));
        pb.cubic_to((p1.0 + k * d1.0) as f32, (p1.1 + k * d1.1) as f32, (p2.0 - k * d2.0) as f32, (p2.1 - k * d2.1) as f32, p2.0 as f32, p2.1 as f32);
        t = t2;
    }
}

/// A rectangle with round corners (`radius` 0: square).
fn rounded(r: Rect, radius: f32) -> Option<sk::Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    if rad <= 0.0 {
        return Some(sk::PathBuilder::from_rect(sk::Rect::from_xywh(r.x, r.y, r.w, r.h)?));
    }
    let k = rad * 0.552_284_8;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut pb = sk::PathBuilder::new();
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

// ---------- Renderer ----------

/// Whether something with these bounds needs the clip.
enum Clip {
    Skip,
    Free,
    Mask,
}

/// The performance overlay's window (override-redirect and click-through, so above every window
/// and never in the way), its surface, its X11 id, and whether it shows.
struct Overlay {
    window: Rc<Window>,
    id: isize,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    size: (u32, u32),
    shown: bool,
}

pub struct Gfx {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    surface_size: (u32, u32),
    px: sk::Pixmap,
    pub width: f32,
    pub height: f32,
    /// Axis-aligned clips, each already intersected with the one below, on whole pixels.
    clips: Vec<Rect>,
    /// Masks for the clips last used (the same few rectangles every frame).
    masks: Vec<(Rect, sk::Mask)>,
    text: Text,
    /// Glyph coverage brightened as DirectWrite's contrast does (`text::contrast_table`).
    contrast: [u8; 256],
    icons: HashMap<(&'static str, bool), Option<sk::Path>>,
    images: HashMap<ImageKey, Slot>,
    /// Each decoded image's most vivid colour (kept when the images go: a few bytes each).
    tones: HashMap<PathBuf, Color>,
    image_bytes: u64,
    requests: Sender<ImageKey>,
    decoded: Receiver<Decoded>,
    frame: u64,
    pub now: f64,
    overlay: Option<Overlay>,
    /// The screen's frame while the overlay is drawn.
    saved: Option<sk::Pixmap>,
}

/// Decoded pixels kept: five screens' worth, at least 24 MB (as on Windows).
fn image_budget(width: f32, height: f32) -> u64 {
    ((width * height * 4.0 * 5.0) as u64).max(24 * 1024 * 1024)
}

impl Gfx {
    /// `font` is Figtree as TrueType.
    pub fn new(window: Rc<Window>, font: &'static [u8]) -> Result<Gfx, String> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let px = sk::Pixmap::new(size.width.max(1), size.height.max(1)).ok_or("no frame")?;
        let (requests, rx) = channel::<ImageKey>();
        let (tx, decoded) = channel::<Decoded>();
        std::thread::Builder::new().name("images".into()).spawn(move || decoder_thread(rx, tx)).map_err(|e| e.to_string())?;
        Ok(Gfx {
            window,
            surface,
            surface_size: (0, 0),
            width: px.width() as f32,
            height: px.height() as f32,
            px,
            clips: Vec::new(),
            masks: Vec::new(),
            text: Text::new(font),
            contrast: crate::linux::text::contrast_table(),
            icons: HashMap::new(),
            images: HashMap::new(),
            tones: HashMap::new(),
            image_bytes: 0,
            requests,
            decoded,
            frame: 0,
            now: 0.0,
            overlay: None,
            saved: None,
        })
    }

    /// The performance overlay's window, created hidden by `linux::app`.
    pub fn set_overlay(&mut self, window: Rc<Window>) {
        let id = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xlib(h)) => h.window as isize,
            Ok(RawWindowHandle::Xcb(h)) => h.window.get() as isize,
            _ => 0,
        };
        self.overlay = Some(Overlay { window, id, surface: None, size: (0, 0), shown: false });
    }

    /// Starts drawing the overlay: `w` x `h` pixels on `background`, opaque (the surface shows
    /// no transparency). False when there is no overlay window.
    pub fn overlay_begin(&mut self, w: f32, h: f32, background: Color) -> bool {
        if self.overlay.is_none() || self.saved.is_some() {
            return false;
        }
        let Some(mut px) = sk::Pixmap::new(w.ceil().max(1.0) as u32, h.ceil().max(1.0) as u32) else { return false };
        px.fill(sk_color(background.alpha(1.0)));
        self.saved = Some(std::mem::replace(&mut self.px, px));
        self.clips.clear();
        true
    }

    /// Shows what was drawn in the overlay window, `margin` pixels in from the top left of the
    /// screen Sofa is on, above every window.
    pub fn overlay_end(&mut self, margin: f32) {
        let Some(screen) = self.saved.take() else { return };
        let px = std::mem::replace(&mut self.px, screen);
        let origin = self.window.current_monitor().map(|m| m.position()).unwrap_or_default();
        let Some(o) = self.overlay.as_mut() else { return };
        let (w, h) = (px.width(), px.height());
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else { return };
        if o.size != (w, h) {
            let _ = o.window.request_inner_size(PhysicalSize::new(w, h));
            o.size = (w, h);
        }
        o.window.set_outer_position(PhysicalPosition::new(origin.x + margin as i32, origin.y + margin as i32));
        if o.surface.is_none() {
            o.surface = softbuffer::Context::new(o.window.clone()).and_then(|c| softbuffer::Surface::new(&c, o.window.clone())).ok();
        }
        let Some(surface) = o.surface.as_mut() else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        // SAFETY: the surface's u32 pixels seen as their bytes (same memory, and u8 needs no
        // alignment); both are w x h pixels of four bytes, BGRA in memory.
        let dst = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 4) };
        if dst.len() == px.data().len() {
            dst.copy_from_slice(px.data());
        }
        let _ = buf.present();
        if !o.shown {
            o.window.set_visible(true);
            o.shown = true;
        }
        self.overlay_raise();
    }

    /// Puts the overlay back above every window.
    pub fn overlay_raise(&self) {
        if let Some(o) = &self.overlay
            && o.shown
            && o.id != 0
        {
            crate::linux::x11::raise(o.id);
        }
    }

    pub fn overlay_hide(&mut self) {
        if let Some(o) = self.overlay.as_mut()
            && o.shown
        {
            o.window.set_visible(false);
            o.shown = false;
        }
    }

    pub fn overlay_shown(&self) -> bool {
        self.overlay.as_ref().is_some_and(|o| o.shown)
    }

    /// The window changed size: the next frame is drawn at the new size.
    pub fn resize(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        if (w, h) != (self.px.width(), self.px.height()) {
            self.px = sk::Pixmap::new(w, h).ok_or("no frame")?;
            self.masks.clear();
        }
        self.width = w as f32;
        self.height = h as f32;
        self.text.forget(self.frame, true);
        Ok(())
    }

    pub fn begin(&mut self, now: f64) {
        self.now = now;
        self.frame += 1;
        self.clips.clear();
        let size = self.window.inner_size();
        if (size.width.max(1), size.height.max(1)) != (self.px.width(), self.px.height()) {
            let _ = self.resize();
        }
    }

    /// Shows the frame.
    pub fn end(&mut self) -> Result<(), String> {
        let (w, h) = (self.px.width(), self.px.height());
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else { return Ok(()) };
        if self.surface_size != (w, h) {
            self.surface.resize(nw, nh).map_err(|e| e.to_string())?;
            self.surface_size = (w, h);
        }
        let mut buf = self.surface.buffer_mut().map_err(|e| e.to_string())?;
        let src = self.px.data();
        // SAFETY: the surface's u32 pixels seen as their bytes (same memory, and u8 needs no
        // alignment); both are w x h pixels of four bytes, BGRA in memory.
        let dst = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 4) };
        if dst.len() == src.len() {
            dst.copy_from_slice(src);
        }
        buf.present().map_err(|e| e.to_string())?;
        // Text layouts not drawn for a while are dropped.
        if self.frame.is_multiple_of(300) {
            self.text.forget(self.frame, false);
        }
        Ok(())
    }

    /// The frame as RGBA (QA screenshots).
    pub fn snapshot(&self) -> (u32, u32, Vec<u8>) {
        let mut out = self.px.data().to_vec();
        for p in out.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        (self.px.width(), self.px.height(), out)
    }

    pub fn clear(&mut self, c: Color) {
        self.px.fill(sk_color(c));
    }

    fn clip_for(&self, b: Rect) -> Clip {
        let Some(c) = self.clips.last() else { return Clip::Free };
        if b.right() <= c.x || b.x >= c.right() || b.bottom() <= c.y || b.y >= c.bottom() || c.w <= 0.0 || c.h <= 0.0 {
            Clip::Skip
        } else if b.x >= c.x && b.y >= c.y && b.right() <= c.right() && b.bottom() <= c.bottom() {
            Clip::Free
        } else {
            Clip::Mask
        }
    }

    /// The mask of the current clip (built once per rectangle).
    fn mask(&mut self) -> Option<usize> {
        let c = *self.clips.last()?;
        if let Some(i) = self.masks.iter().position(|(r, _)| *r == c) {
            return Some(i);
        }
        let mut m = sk::Mask::new(self.px.width(), self.px.height())?;
        let path = sk::PathBuilder::from_rect(sk::Rect::from_xywh(c.x, c.y, c.w, c.h)?);
        m.fill_path(&path, sk::FillRule::Winding, false, sk::Transform::identity());
        if self.masks.len() >= 3 {
            self.masks.remove(0);
        }
        self.masks.push((c, m));
        Some(self.masks.len() - 1)
    }

    /// Draws `path` with `paint` within the clip.
    fn fill_path(&mut self, path: &sk::Path, paint: &sk::Paint, bounds: Rect, tf: sk::Transform) {
        match self.clip_for(bounds) {
            Clip::Skip => {}
            Clip::Free => self.px.fill_path(path, paint, sk::FillRule::Winding, tf, None),
            Clip::Mask => {
                if let Some(i) = self.mask() {
                    self.px.fill_path(path, paint, sk::FillRule::Winding, tf, Some(&self.masks[i].1));
                }
            }
        }
    }

    pub fn fill(&mut self, r: Rect, c: Color, radius: f32) {
        if c.3 <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let p = paint(c);
        if radius <= 0.0 {
            // Square: the clip is a rectangle too, so the two simply intersect.
            let r = match self.clips.last() {
                Some(c) => {
                    let (x0, y0) = (r.x.max(c.x), r.y.max(c.y));
                    let (x1, y1) = (r.right().min(c.right()), r.bottom().min(c.bottom()));
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                }
                None => r,
            };
            if let Some(rect) = sk::Rect::from_xywh(r.x, r.y, r.w, r.h).filter(|_| r.w > 0.0 && r.h > 0.0) {
                self.px.fill_rect(rect, &p, sk::Transform::identity(), None);
            }
            return;
        }
        if let Some(path) = rounded(r, radius) {
            self.fill_path(&path, &p, r, sk::Transform::identity());
        }
    }

    pub fn clip(&mut self, r: Rect) {
        // Whole pixels, as Direct2D's aliased clip.
        let (x0, y0, x1, y1) = (r.x.round(), r.y.round(), r.right().round(), r.bottom().round());
        let mut c = Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
        if let Some(p) = self.clips.last() {
            let (x0, y0) = (c.x.max(p.x), c.y.max(p.y));
            let (x1, y1) = (c.right().min(p.right()), c.bottom().min(p.bottom()));
            c = Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
        }
        self.clips.push(c);
    }

    pub fn unclip(&mut self) {
        self.clips.pop();
    }

    pub fn stroke(&mut self, r: Rect, c: Color, radius: f32, width: f32) {
        if c.3 <= 0.0 {
            return;
        }
        let Some(path) = rounded(r, radius) else { return };
        let stroke = sk::Stroke { width, ..sk::Stroke::default() };
        let b = r.inset(-width);
        let p = paint(c);
        match self.clip_for(b) {
            Clip::Skip => {}
            Clip::Free => self.px.stroke_path(&path, &p, &stroke, sk::Transform::identity(), None),
            Clip::Mask => {
                if let Some(i) = self.mask() {
                    self.px.stroke_path(&path, &p, &stroke, sk::Transform::identity(), Some(&self.masks[i].1));
                }
            }
        }
    }

    /// A linear gradient over `r` (vertical when `vertical`), with stops at fractions: used
    /// only as a scrim to keep text readable over art.
    pub fn gradient(&mut self, r: Rect, stops: &[(f32, Color)], vertical: bool) {
        let s: Vec<sk::GradientStop> = stops.iter().map(|(p, c)| sk::GradientStop::new(*p, sk_color(*c))).collect();
        let (p0, p1) = if vertical { (sk::Point::from_xy(r.x, r.y), sk::Point::from_xy(r.x, r.bottom())) } else { (sk::Point::from_xy(r.x, r.y), sk::Point::from_xy(r.right(), r.y)) };
        let Some(shader) = sk::LinearGradient::new(p0, p1, s, sk::SpreadMode::Pad, sk::Transform::identity()) else { return };
        let p = sk::Paint { shader, anti_alias: true, ..sk::Paint::default() };
        if let Some(path) = sk::Rect::from_xywh(r.x, r.y, r.w, r.h).map(sk::PathBuilder::from_rect) {
            self.fill_path(&path, &p, r, sk::Transform::identity());
        }
    }

    // ---------- Text ----------

    /// Width and height of a single line of text.
    pub fn measure(&mut self, text: &str, size: f32, weight: u32) -> (f32, f32) {
        let l = self.text.layout(text, size, weight, 100_000.0, 0, Align::Left, 0.0, self.frame);
        (l.width, l.height)
    }

    /// Height of wrapped text.
    pub fn text_height(&mut self, text: &str, size: f32, weight: u32, width: f32, lines: u32) -> f32 {
        self.text.layout(text, size, weight, width, lines, Align::Left, 0.0, self.frame).height
    }

    /// Draws text with its top-left at (x, y), within `width`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, s: &str, x: f32, y: f32, width: f32, size: f32, weight: u32, c: Color, align: Align, lines: u32, tracking: f32) {
        if s.is_empty() || c.3 <= 0.0 {
            return;
        }
        let layout = self.text.layout(s, size, weight, width, lines, align, tracking, self.frame);
        let (pw, ph) = (self.px.width() as i32, self.px.height() as i32);
        let (cx0, cy0, cx1, cy1) = match self.clips.last() {
            Some(c) => ((c.x as i32).max(0), (c.y as i32).max(0), (c.right() as i32).min(pw), (c.bottom() as i32).min(ph)),
            None => (0, 0, pw, ph),
        };
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        // The colour premultiplied, in the frame's BGRA order.
        let a = (c.3.clamp(0.0, 1.0) * 255.0).round() as u32;
        let ch = |v: f32| mul((v.clamp(0.0, 1.0) * 255.0).round() as u32, a);
        let col = [ch(c.2), ch(c.1), ch(c.0), a];
        let stride = pw as usize * 4;
        let contrast = self.contrast;
        for line in &layout.lines {
            let base_y = (y + line.baseline).round() as i32;
            for g in &line.glyphs {
                let pen = x + line.x + g.x;
                let whole = pen.floor();
                let fx = (((pen - whole) * 4.0).round() as u8).min(3);
                let gy = base_y + (g.y.round() as i32);
                let Some(img) = self.text.glyph(g.face, g.id, size, weight, fx) else { continue };
                let (left, top) = (whole as i32 + img.left, gy - img.top);
                let (x0, y0) = (left.max(cx0), top.max(cy0));
                let (x1, y1) = ((left + img.w as i32).min(cx1), (top + img.h as i32).min(cy1));
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let data = self.px.data_mut();
                for py in y0..y1 {
                    let row = (py - top) as usize * img.w as usize;
                    let out = py as usize * stride;
                    for px in x0..x1 {
                        let i = out + px as usize * 4;
                        if img.color {
                            let j = (row + (px - left) as usize) * 4;
                            let (r, g2, b, sa) = (u32::from(img.data[j]), u32::from(img.data[j + 1]), u32::from(img.data[j + 2]), mul(u32::from(img.data[j + 3]), a));
                            if sa == 0 {
                                continue;
                            }
                            let inv = 255 - sa;
                            data[i] = (mul(b, sa) + mul(u32::from(data[i]), inv)).min(255) as u8;
                            data[i + 1] = (mul(g2, sa) + mul(u32::from(data[i + 1]), inv)).min(255) as u8;
                            data[i + 2] = (mul(r, sa) + mul(u32::from(data[i + 2]), inv)).min(255) as u8;
                            data[i + 3] = (sa + mul(u32::from(data[i + 3]), inv)).min(255) as u8;
                        } else {
                            let m = u32::from(contrast[usize::from(img.data[row + (px - left) as usize])]);
                            if m == 0 {
                                continue;
                            }
                            let sa = mul(col[3], m);
                            let inv = 255 - sa;
                            for k in 0..3 {
                                data[i + k] = (mul(col[k], m) + mul(u32::from(data[i + k]), inv)).min(255) as u8;
                            }
                            data[i + 3] = (sa + mul(u32::from(data[i + 3]), inv)).min(255) as u8;
                        }
                    }
                }
            }
        }
    }

    // ---------- Icons ----------

    /// Draws a 24-unit icon into `r`: stroked (Lucide) or filled (brand marks).
    pub fn icon(&mut self, name: &'static str, svg: &str, r: Rect, c: Color, filled: bool, stroke: f32) {
        if c.3 <= 0.0 {
            return;
        }
        self.icons.entry((name, filled)).or_insert_with(|| {
            let mut b = Builder::default();
            walk_path(&icon_path_data(svg), &mut b);
            b.end(false);
            b.pb.finish()
        });
        let s = r.w.min(r.h) / 24.0;
        let tf = sk::Transform::from_row(s, 0.0, 0.0, s, r.x + (r.w - 24.0 * s) / 2.0, r.y + (r.h - 24.0 * s) / 2.0);
        let decision = self.clip_for(r.inset(-stroke * s));
        let mask = if matches!(decision, Clip::Mask) { self.mask() } else { None };
        if matches!(decision, Clip::Skip) || (matches!(decision, Clip::Mask) && mask.is_none()) {
            return;
        }
        let Some(Some(path)) = self.icons.get(&(name, filled)) else { return };
        let m = mask.map(|i| &self.masks[i].1);
        let p = paint(c);
        if filled {
            self.px.fill_path(path, &p, sk::FillRule::Winding, tf, m);
        } else {
            let st = sk::Stroke { width: stroke, line_cap: sk::LineCap::Round, line_join: sk::LineJoin::Round, ..sk::Stroke::default() };
            self.px.stroke_path(path, &p, &st, tf, m);
        }
    }

    // ---------- Images ----------

    /// Takes images the worker finished. Returns true when any arrived (the frame should be
    /// drawn again).
    pub fn take_decoded(&mut self) -> bool {
        let mut any = false;
        while let Ok(d) = self.decoded.try_recv() {
            any = true;
            if let Some(t) = d.tone {
                self.tones.insert(d.key.path.clone(), t);
            }
            let slot = match d.pixels.and_then(|(w, h, px)| sk::IntSize::from_wh(w, h).and_then(|s| sk::Pixmap::from_vec(px, s)).map(|p| (w, h, p))) {
                Some((w, h, px)) => {
                    self.image_bytes += u64::from(w) * u64::from(h) * 4;
                    Slot::Ready { px, w, h, ready_at: self.now, used: self.frame }
                }
                None => Slot::Failed,
            };
            self.images.insert(d.key, slot);
        }
        if self.image_bytes > image_budget(self.width, self.height) {
            self.evict();
        }
        any
    }

    fn evict(&mut self) {
        let mut ready: Vec<(u64, ImageKey, u64)> = self
            .images
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Ready { w, h, used, .. } => Some((*used, k.clone(), u64::from(*w) * u64::from(*h) * 4)),
                _ => None,
            })
            .collect();
        ready.sort_by_key(|(used, ..)| *used);
        for (used, key, bytes) in ready {
            if self.image_bytes <= image_budget(self.width, self.height) * 3 / 4 || used + 2 >= self.frame {
                break;
            }
            self.images.remove(&key);
            self.image_bytes -= bytes;
        }
    }

    /// Draws an image into `r` (requesting it when not decoded yet). Returns its fade-in
    /// progress (0 while loading, 1 once shown for 250 ms) or None when it cannot be loaded.
    pub fn image(&mut self, path: &Path, r: Rect, fit: Fit, radius: f32, opacity: f32) -> Option<f32> {
        self.image_sized(path, r, (r.w, r.h), fit, radius, opacity)
    }

    /// Like `image`, decoded at `base` size whatever size it is drawn at, so art that zooms
    /// (a focused tile, the hero settling) is decoded once.
    #[allow(clippy::too_many_arguments)]
    pub fn image_sized(&mut self, path: &Path, r: Rect, base: (f32, f32), fit: Fit, radius: f32, opacity: f32) -> Option<f32> {
        if r.w < 1.0 || r.h < 1.0 || base.0 < 1.0 || base.1 < 1.0 {
            return Some(0.0);
        }
        let cover = matches!(fit, Fit::Cover | Fit::CoverAt(_));
        let key = ImageKey { path: path.to_path_buf(), w: base.0.ceil() as u32, h: base.1.ceil() as u32, cover };
        // Close sizes share one decode (focus scaling changes the size by a few percent).
        let key = ImageKey { w: key.w.div_ceil(16) * 16, h: key.h.div_ceil(16) * 16, ..key };
        let (frame, now) = (self.frame, self.now);
        let (bw, bh, fade) = match self.images.get_mut(&key) {
            None => {
                self.images.insert(key.clone(), Slot::Loading);
                let _ = self.requests.send(key);
                return Some(0.0);
            }
            Some(Slot::Loading) => return Some(0.0),
            Some(Slot::Failed) => return None,
            Some(Slot::Ready { w, h, ready_at, used, .. }) => {
                *used = frame;
                (*w as f32, *h as f32, (((now - *ready_at) / 250.0) as f32).clamp(0.0, 1.0))
            }
        };
        // Source rectangle for cover, or destination for contain.
        let (src, dst) = match fit {
            Fit::Cover | Fit::CoverAt(_) => {
                let focal = if let Fit::CoverAt(f) = fit { f } else { 0.5 };
                let s = (r.w / bw).max(r.h / bh);
                let (sw, sh) = (r.w / s, r.h / s);
                (Rect::new((bw - sw) / 2.0, (bh - sh) * focal, sw, sh), r)
            }
            Fit::Contain | Fit::ContainLeftBottom => {
                let s = (r.w / bw).min(r.h / bh);
                let (dw, dh) = (bw * s, bh * s);
                let at = if fit == Fit::Contain { Rect::new(r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0, dw, dh) } else { Rect::new(r.x, r.bottom() - dh, dw, dh) };
                (Rect::new(0.0, 0.0, bw, bh), at)
            }
        };
        let alpha = opacity * fade;
        if alpha <= 0.0 {
            return Some(fade);
        }
        let (sx, sy) = (dst.w / src.w, dst.h / src.h);
        let tf = sk::Transform::from_row(sx, 0.0, 0.0, sy, dst.x - src.x * sx, dst.y - src.y * sy);
        let decision = self.clip_for(dst);
        // A square image and a square clip meet in a rectangle; round corners need the mask.
        let mask = if matches!(decision, Clip::Mask) && radius > 0.0 { self.mask() } else { None };
        let clip = self.clips.last().copied();
        let Some(Slot::Ready { px, .. }) = self.images.get(&key) else { return Some(fade) };
        let quality = if (sx - 1.0).abs() < 0.002 && (sy - 1.0).abs() < 0.002 { sk::FilterQuality::Nearest } else { sk::FilterQuality::Bilinear };
        let shader = sk::Pattern::new(px.as_ref(), sk::SpreadMode::Pad, quality, alpha, tf);
        let p = sk::Paint { shader, anti_alias: true, ..sk::Paint::default() };
        match decision {
            Clip::Skip => {}
            Clip::Free => {
                if let Some(path) = rounded(dst, radius) {
                    self.px.fill_path(&path, &p, sk::FillRule::Winding, sk::Transform::identity(), None);
                }
            }
            Clip::Mask if radius <= 0.0 => {
                let Some(c) = clip else { return Some(fade) };
                let (x0, y0) = (dst.x.max(c.x), dst.y.max(c.y));
                let (x1, y1) = (dst.right().min(c.right()), dst.bottom().min(c.bottom()));
                if let Some(rect) = sk::Rect::from_xywh(x0, y0, x1 - x0, y1 - y0).filter(|_| x1 > x0 && y1 > y0) {
                    self.px.fill_rect(rect, &p, sk::Transform::identity(), None);
                }
            }
            Clip::Mask => {
                if let (Some(i), Some(path)) = (mask, rounded(dst, radius)) {
                    self.px.fill_path(&path, &p, sk::FillRule::Winding, sk::Transform::identity(), Some(&self.masks[i].1));
                }
            }
        }
        Some(fade)
    }

    /// Gives memory back while Sofa waits behind a game: decoded art, text layouts, glyphs and
    /// clip masks go. Everything is rebuilt on the next frame.
    pub fn release(&mut self) {
        self.images.clear();
        self.images.shrink_to_fit();
        self.image_bytes = 0;
        self.masks.clear();
        self.icons.clear();
        self.text.forget(self.frame, true);
    }

    /// The most vivid colour of an image that was drawn, once it has been decoded.
    pub fn tone(&self, path: &Path) -> Option<Color> {
        self.tones.get(path).copied()
    }

    /// Forgets images that could not be loaded, so files written since are tried again.
    pub fn retry_failed(&mut self) {
        self.images.retain(|_, s| !matches!(s, Slot::Failed));
    }

    /// Whether an image finished loading in the last 250 ms (so frames should keep coming).
    pub fn fading(&self) -> bool {
        self.images.values().any(|s| matches!(s, Slot::Ready { ready_at, .. } if self.now - ready_at < 260.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pts(Vec<(f32, f32)>);

    impl Pts {
        fn of(path: &sk::Path) -> Pts {
            Pts(path.points().iter().map(|p| (p.x, p.y)).collect())
        }
    }

    #[test]
    fn arcs_end_where_svg_says() {
        let mut pb = sk::PathBuilder::new();
        pb.move_to(0.0, 12.0);
        arc_to(&mut pb, (0.0, 12.0), (24.0, 12.0), 12.0, 12.0, 0.0, true, true);
        let path = pb.finish().expect("a half circle");
        let pts = Pts::of(&path);
        let last = *pts.0.last().expect("points");
        assert!((last.0 - 24.0).abs() < 1e-3 && (last.1 - 12.0).abs() < 1e-3, "{last:?}");
        // Clockwise from the left: the arc passes over the top (y near 0).
        assert!(pts.0.iter().any(|p| p.1 < 1.0), "{:?}", pts.0);
    }

    #[test]
    fn rounded_rects_stay_inside_their_box() {
        let path = rounded(Rect::new(10.0, 10.0, 100.0, 40.0), 50.0).expect("path");
        let b = path.bounds();
        assert!(b.left() >= 9.99 && b.right() <= 110.01 && b.top() >= 9.99 && b.bottom() <= 50.01);
    }

    #[test]
    fn decoding_premultiplies_into_bgra() {
        let dir = std::env::temp_dir().join(format!("sofa-gfx-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("red.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 128])).save(&file).expect("png");
        let (w, h, px) = decode(&ImageKey { path: file.clone(), w: 8, h: 4, cover: false }).expect("decoded");
        assert_eq!((w, h), (8, 4));
        assert_eq!(&px[..4], &[0, 0, 128, 128]);
        let (w, h, _) = decode(&ImageKey { path: file, w: 4, h: 4, cover: false }).expect("smaller");
        assert_eq!((w, h), (4, 2), "contain shrinks to fit, keeping the shape");
        let _ = std::fs::remove_dir_all(dir);
    }
}
