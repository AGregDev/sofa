//! Drawing: Direct2D on a DXGI flip-model swapchain, DirectWrite text in Sofa's Figtree, WIC
//! image decoding on a worker thread (decoded at the size drawn, kept under a memory budget),
//! and Lucide icons built from their SVG paths.

use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

pub const WM_IMAGE: u32 = 0x8000 + 1; // WM_APP + 1: a decoded image is ready

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    /// Scaled around its centre.
    pub fn scaled(&self, s: f32) -> Rect {
        let (w, h) = (self.w * s, self.h * s);
        Rect::new(self.x - (w - self.w) / 2.0, self.y - (h - self.h) / 2.0, w, h)
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn hex(v: u32) -> Color {
        Color(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
    }
    pub const fn alpha(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }
    /// '#rrggbb' or 'hsl(h s% l%)' (the Electron app's colour formats).
    pub fn parse(s: &str) -> Option<Color> {
        if let Some(h) = s.strip_prefix('#') {
            return u32::from_str_radix(h, 16).ok().map(Color::hex);
        }
        let inner = s.strip_prefix("hsl(")?.strip_suffix(')')?;
        let n: Vec<f32> = inner.split([' ', ',']).filter(|p| !p.is_empty()).filter_map(|p| p.trim_end_matches('%').parse().ok()).collect();
        Some(Color::hsl(*n.first()?, *n.get(1)? / 100.0, *n.get(2)? / 100.0))
    }
    /// Hue in degrees, saturation and lightness from 0 to 1.
    pub fn hsl(h: f32, s: f32, l: f32) -> Color {
        let h = h / 360.0;
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f32| {
            t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Color(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0), 1.0)
    }
    /// As 8-bit red, green and blue.
    pub fn rgb8(self) -> (u8, u8, u8) {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (c(self.0), c(self.1), c(self.2))
    }
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Fit {
    Cover,
    /// Cover, cropped around a vertical focal point (CSS `object-position: center 30%`).
    CoverAt(f32),
    Contain,
    /// Contain, anchored bottom-left (game logos over the hero art).
    ContainLeftBottom,
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

/// The art's most vivid colour (Sofa 1.x's `vibrantColor`): hues on a 12 x 12 grid, averaged
/// with saturated, bright pixels counting most, as a mid-light colour. None for grey art.
/// `px` is premultiplied BGRA, `w` x `h`.
pub(crate) fn tone(px: &[u8], w: u32, h: u32) -> Option<Color> {
    if w == 0 || h == 0 || px.len() < (w * h * 4) as usize {
        return None;
    }
    let (mut x, mut y, mut sum) = (0f32, 0f32, 0f32);
    for gy in 0..12u32 {
        for gx in 0..12u32 {
            let (px_x, px_y) = ((gx * 2 + 1) * w / 24, (gy * 2 + 1) * h / 24);
            let i = ((px_y * w + px_x) * 4) as usize;
            let a = f32::from(px[i + 3]) / 255.0;
            if a <= 0.0 {
                continue;
            }
            let (b, g, r) = (f32::from(px[i]) / 255.0 / a, f32::from(px[i + 1]) / 255.0 / a, f32::from(px[i + 2]) / 255.0 / a);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
            let weight = a * sat * sat * max + 0.0001;
            if max - min < 1e-4 {
                // Grey has no hue: it only weakens the others.
                sum += weight;
                continue;
            }
            let hue = if max == r {
                ((g - b) / (max - min)) % 6.0
            } else if max == g {
                (b - r) / (max - min) + 2.0
            } else {
                (r - g) / (max - min) + 4.0
            } * std::f32::consts::PI
                / 3.0;
            x += hue.cos() * weight;
            y += hue.sin() * weight;
            sum += weight;
        }
    }
    let strength = (x.hypot(y) / sum.max(1e-6)).min(1.0);
    (strength >= 0.15).then(|| Color::hsl((y.atan2(x).to_degrees() + 360.0) % 360.0, 0.85, 0.58))
}

enum Slot {
    Loading,
    Ready { bmp: ID2D1Bitmap1, w: u32, h: u32, ready_at: f64, used: u64 },
    Failed,
}

/// Decodes images with WIC at the size they are drawn, off the UI thread.
fn decoder_thread(rx: Receiver<ImageKey>, tx: Sender<Decoded>, hwnd: isize) {
    // SAFETY: COM initialised once for this thread, before any WIC call.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: plain COM object creation.
    let wic: Option<IWICImagingFactory> = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.ok();
    for key in rx {
        let pixels = wic.as_ref().and_then(|wic| decode(wic, &key).ok());
        let tone = pixels.as_ref().and_then(|(w, h, px)| tone(px, *w, *h));
        if tx.send(Decoded { key, pixels, tone }).is_err() {
            break;
        }
        // SAFETY: posting to our own window; a closed window just drops the message.
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_IMAGE, WPARAM(0), LPARAM(0));
        }
    }
}

fn decode(wic: &IWICImagingFactory, key: &ImageKey) -> Result<(u32, u32, Vec<u8>)> {
    // SAFETY: WIC calls on objects this function owns; buffers sized from the reported sizes.
    unsafe {
        let path = HSTRING::from(key.path.as_os_str());
        let decoder = wic.CreateDecoderFromFilename(&path, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut sw, mut sh) = (0u32, 0u32);
        frame.GetSize(&mut sw, &mut sh)?;
        if sw == 0 || sh == 0 {
            return Err(Error::from(E_FAIL));
        }
        // Never upscale: draw-time scaling handles small sources.
        let s = if key.cover { (key.w as f32 / sw as f32).max(key.h as f32 / sh as f32) } else { (key.w as f32 / sw as f32).min(key.h as f32 / sh as f32) };
        let s = s.min(1.0);
        let (w, h) = (((sw as f32 * s).round() as u32).max(1), ((sh as f32 * s).round() as u32).max(1));
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant)?;
        let conv = wic.CreateFormatConverter()?;
        conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        let stride = w * 4;
        let mut buf = vec![0u8; (stride * h) as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut buf)?;
        Ok((w, h, buf))
    }
}

// ---------- Icons: SVG path data to Direct2D geometry ----------

struct PathBuilder {
    sink: ID2D1GeometrySink,
    open: bool,
    filled: bool,
}

impl PathBuilder {
    fn begin(&mut self, p: Vector2) {
        self.end(false);
        // SAFETY: the sink is open until `close`.
        unsafe { self.sink.BeginFigure(p, if self.filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW }) };
        self.open = true;
    }
    fn end(&mut self, closed: bool) {
        if self.open {
            // SAFETY: a figure is open.
            unsafe { self.sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN }) };
            self.open = false;
        }
    }
}

fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// Reads SVG numbers, including `1.5.5` (two numbers) and `-12.77.04`.
struct Nums<'a> {
    s: &'a [u8],
    i: usize,
}

impl Nums<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b',' || self.s[self.i] == b'\n' || self.s[self.i] == b'\t') {
            self.i += 1;
        }
    }
    fn more(&mut self) -> bool {
        self.skip();
        self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'-' | b'+' | b'.'))
    }
    fn num(&mut self) -> f32 {
        self.skip();
        let start = self.i;
        if self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else if (c == b'e' || c == b'E') && self.i > start {
                self.i += 1;
                if self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
    }
    fn flag(&mut self) -> bool {
        self.skip();
        let f = self.i < self.s.len() && self.s[self.i] == b'1';
        self.i += 1;
        f
    }
}

fn add_path_data(b: &mut PathBuilder, d: &str) {
    let mut n = Nums { s: d.as_bytes(), i: 0 };
    let (mut cur, mut start, mut last_ctrl) = (v(0.0, 0.0), v(0.0, 0.0), None::<Vector2>);
    let mut cmd = b'M';
    loop {
        n.skip();
        if n.i >= n.s.len() {
            break;
        }
        let c = n.s[n.i];
        if c == b'|' {
            // A new <path> element: its first moveto is relative to the origin, not to where
            // the previous element ended.
            cur = v(0.0, 0.0);
            start = cur;
            last_ctrl = None;
            cmd = b'M';
            n.i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            cmd = c;
            n.i += 1;
        } else if !n.more() {
            n.i += 1;
            continue;
        }
        let rel = cmd.is_ascii_lowercase();
        let at = |x: f32, y: f32, cur: Vector2| if rel { v(cur.X + x, cur.Y + y) } else { v(x, y) };
        // SAFETY (all sink calls below): the sink is open until `close`.
        unsafe {
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    let p = at(n.num(), n.num(), cur);
                    b.begin(p);
                    cur = p;
                    start = p;
                    cmd = if rel { b'l' } else { b'L' }; // further pairs are line-tos
                    last_ctrl = None;
                }
                b'L' => {
                    let p = at(n.num(), n.num(), cur);
                    b.sink.AddLine(p);
                    cur = p;
                    last_ctrl = None;
                }
                b'H' => {
                    let x = n.num();
                    cur = v(if rel { cur.X + x } else { x }, cur.Y);
                    b.sink.AddLine(cur);
                    last_ctrl = None;
                }
                b'V' => {
                    let y = n.num();
                    cur = v(cur.X, if rel { cur.Y + y } else { y });
                    b.sink.AddLine(cur);
                    last_ctrl = None;
                }
                b'C' | b'S' => {
                    let c1 = if cmd.eq_ignore_ascii_case(&b'C') {
                        at(n.num(), n.num(), cur)
                    } else {
                        last_ctrl.map_or(cur, |lc| v(2.0 * cur.X - lc.X, 2.0 * cur.Y - lc.Y))
                    };
                    let c2 = at(n.num(), n.num(), cur);
                    let p = at(n.num(), n.num(), cur);
                    b.sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: c1, point2: c2, point3: p });
                    last_ctrl = Some(c2);
                    cur = p;
                }
                b'Q' => {
                    let c1 = at(n.num(), n.num(), cur);
                    let p = at(n.num(), n.num(), cur);
                    b.sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: c1, point2: p });
                    last_ctrl = Some(c1);
                    cur = p;
                }
                b'A' => {
                    let (rx, ry, rot) = (n.num(), n.num(), n.num());
                    let (large, sweep) = (n.flag(), n.flag());
                    let p = at(n.num(), n.num(), cur);
                    b.sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: p,
                        size: D2D_SIZE_F { width: rx, height: ry },
                        rotationAngle: rot,
                        sweepDirection: if sweep { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    cur = p;
                    last_ctrl = None;
                }
                b'Z' => {
                    b.end(true);
                    cur = start;
                    last_ctrl = None;
                }
                _ => {
                    n.i += 1;
                }
            }
        }
    }
}

/// Converts Lucide's SVG elements (`<path d>`, `<circle>`, `<rect>`, `<line>`, `<polyline>`,
/// `<polygon>`) to path data.
fn svg_to_path_data(svg: &str) -> String {
    let attr = |tag: &str, name: &str| -> f32 {
        let pat = format!(" {name}=\"");
        tag.find(&pat).and_then(|i| tag[i + pat.len()..].split('"').next()).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    };
    let mut d = String::new();
    for tag in svg.split('<').filter(|t| !t.trim().is_empty()) {
        let name = tag.split([' ', '/', '>']).next().unwrap_or("");
        match name {
            "path" => {
                if let Some(i) = tag.find(" d=\"") {
                    d.push_str("| ");
                    d.push_str(tag[i + 4..].split('"').next().unwrap_or(""));
                    d.push(' ');
                }
            }
            "circle" => {
                let (cx, cy, r) = (attr(tag, "cx"), attr(tag, "cy"), attr(tag, "r"));
                d.push_str(&format!("M{} {} A{r} {r} 0 1 1 {} {} A{r} {r} 0 1 1 {} {} Z ", cx - r, cy, cx + r, cy, cx - r, cy));
            }
            "rect" => {
                let (x, y, w, h) = (attr(tag, "x"), attr(tag, "y"), attr(tag, "width"), attr(tag, "height"));
                let r = attr(tag, "rx").max(attr(tag, "ry")).min(w / 2.0).min(h / 2.0);
                if r > 0.0 {
                    d.push_str(&format!(
                        "M{} {y} H{} A{r} {r} 0 0 1 {} {} V{} A{r} {r} 0 0 1 {} {} H{} A{r} {r} 0 0 1 {x} {} V{} A{r} {r} 0 0 1 {} {y} Z ",
                        x + r, x + w - r, x + w, y + r, y + h - r, x + w - r, y + h, x + r, y + h - r, y + r, x + r
                    ));
                } else {
                    d.push_str(&format!("M{x} {y} H{} V{} H{x} Z ", x + w, y + h));
                }
            }
            "line" => d.push_str(&format!("M{} {} L{} {} ", attr(tag, "x1"), attr(tag, "y1"), attr(tag, "x2"), attr(tag, "y2"))),
            "polyline" | "polygon" => {
                if let Some(i) = tag.find(" points=\"") {
                    let pts = tag[i + 9..].split('"').next().unwrap_or("");
                    d.push('M');
                    d.push_str(pts);
                    if name == "polygon" {
                        d.push_str(" Z");
                    }
                    d.push(' ');
                }
            }
            _ => {}
        }
    }
    d
}

// ---------- Renderer ----------

/// Text, size x10, weight, width, line count, alignment (+8 with letter spacing).
type LayoutKey = (String, u32, u32, u32, u32, u8);

pub struct Gfx {
    hwnd: HWND,
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory5,
    fonts: Option<IDWriteFontCollection>,
    family: HSTRING,
    swapchain: IDXGISwapChain1,
    device: ID2D1Device,
    d3d_context: ID3D11DeviceContext,
    dxgi: IDXGIDevice3,
    dc: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle1,
    pub width: f32,
    pub height: f32,
    formats: HashMap<(u32, u32), IDWriteTextFormat>,
    layouts: HashMap<LayoutKey, (IDWriteTextLayout, u64)>,
    icons: HashMap<(&'static str, bool), ID2D1PathGeometry1>,
    images: HashMap<ImageKey, Slot>,
    /// Each decoded image's most vivid colour (kept when the images go: a few bytes each).
    tones: HashMap<PathBuf, Color>,
    image_bytes: u64,
    requests: Sender<ImageKey>,
    decoded: Receiver<Decoded>,
    frame: u64,
    pub now: f64,
}

/// Decoded pixels kept as bitmaps: five screens' worth, at least 24 MB. What was drawn least
/// recently goes first (art for tiles that are off screen, heroes no longer focused).
fn image_budget(width: f32, height: f32) -> u64 {
    ((width * height * 4.0 * 5.0) as u64).max(24 * 1024 * 1024)
}

impl Gfx {
    pub fn new(hwnd: HWND, font_woff2: &[u8]) -> Result<Gfx> {
        // SAFETY: Direct2D/DirectWrite/D3D11/DXGI creation with valid arguments; every
        // interface returned is owned by the struct.
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let dwrite: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut d3d: Option<ID3D11Device> = None;
            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
            let mut created = Err(Error::from(E_FAIL));
            for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
                // Only the UI thread uses the device: single-threaded skips Direct3D's locking.
                created = D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED, Some(&levels), D3D11_SDK_VERSION, Some(&mut d3d), None, None);
                if created.is_ok() {
                    break;
                }
            }
            created?;
            let d3d = d3d.ok_or_else(|| Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice3 = d3d.cast()?;
            let d3d_context = d3d.GetImmediateContext()?;
            let device = factory.CreateDevice(&dxgi)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let adapter = dxgi.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: 0,
                Height: 0,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                Flags: 0,
            };
            let swapchain = dxgi_factory.CreateSwapChainForHwnd(&d3d, hwnd, &desc, None, None)?;
            let _ = dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            dc.SetUnitMode(D2D1_UNIT_MODE_PIXELS);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = dc.CreateSolidColorBrush(&Color::hex(0xffffff).d2d(), None)?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                    transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
                },
                None,
            )?;
            let fonts = load_font(&dwrite, font_woff2).ok();
            let family = HSTRING::from(if fonts.is_some() { "Figtree" } else { "Segoe UI" });
            let (requests, rx) = channel::<ImageKey>();
            let (tx, decoded) = channel::<Decoded>();
            let h = hwnd.0 as isize;
            std::thread::Builder::new().name("images".into()).spawn(move || decoder_thread(rx, tx, h)).map_err(|_| Error::from(E_FAIL))?;
            let mut g = Gfx {
                hwnd,
                factory,
                dwrite,
                fonts,
                family,
                swapchain,
                device,
                d3d_context,
                dxgi,
                dc,
                brush,
                round,
                width: 0.0,
                height: 0.0,
                formats: HashMap::new(),
                layouts: HashMap::new(),
                icons: HashMap::new(),
                images: HashMap::new(),
                tones: HashMap::new(),
                image_bytes: 0,
                requests,
                decoded,
                frame: 0,
                now: 0.0,
            };
            g.attach_target()?;
            Ok(g)
        }
    }

    fn attach_target(&mut self) -> Result<()> {
        // SAFETY: the back buffer belongs to our swapchain; the bitmap wraps it for drawing.
        unsafe {
            let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let target = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&target);
            let size = target.GetPixelSize();
            self.width = size.width as f32;
            self.height = size.height as f32;
        }
        Ok(())
    }

    pub fn resize(&mut self) -> Result<()> {
        // SAFETY: the target is released before the buffers are resized.
        unsafe {
            self.dc.SetTarget(None);
            self.swapchain.ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.layouts.clear();
        self.attach_target()
    }

    pub fn begin(&mut self, now: f64) {
        self.now = now;
        self.frame += 1;
        // SAFETY: matched by `end`.
        unsafe {
            self.dc.BeginDraw();
            self.dc.SetTransform(&Matrix3x2::identity());
        }
    }

    pub fn end(&mut self) -> Result<()> {
        // SAFETY: matched with `begin`; Present waits for the next vertical blank.
        unsafe {
            self.dc.EndDraw(None, None)?;
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        // Text layouts not drawn for a while are dropped.
        let frame = self.frame;
        if frame.is_multiple_of(300) {
            self.layouts.retain(|_, (_, used)| frame - *used < 600);
        }
        Ok(())
    }

    pub fn clear(&self, c: Color) {
        // SAFETY: inside BeginDraw/EndDraw.
        unsafe { self.dc.Clear(Some(&c.d2d())) };
    }

    fn color(&self, c: Color) -> &ID2D1SolidColorBrush {
        // SAFETY: a plain setter on our brush.
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    pub fn fill(&self, r: Rect, c: Color, radius: f32) {
        // SAFETY: inside BeginDraw/EndDraw.
        unsafe {
            if radius > 0.0 {
                self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, self.color(c));
            } else {
                self.dc.FillRectangle(&r.d2d(), self.color(c));
            }
        }
    }

    pub fn clip(&self, r: Rect) {
        // SAFETY: inside BeginDraw/EndDraw; matched by `unclip`.
        unsafe { self.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    pub fn unclip(&self) {
        // SAFETY: matches a `clip`.
        unsafe { self.dc.PopAxisAlignedClip() };
    }


    pub fn stroke(&self, r: Rect, c: Color, radius: f32, width: f32) {
        // SAFETY: inside BeginDraw/EndDraw.
        unsafe { self.dc.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, self.color(c), width, None) };
    }

    /// A linear gradient from `a` to `b` over `r` (vertical when `vertical`), with stops at
    /// fractions: used only as a scrim to keep text readable over art.
    pub fn gradient(&self, r: Rect, stops: &[(f32, Color)], vertical: bool) {
        let s: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(p, c)| D2D1_GRADIENT_STOP { position: *p, color: c.d2d() }).collect();
        // SAFETY: inside BeginDraw/EndDraw; brush resources live for this call.
        unsafe {
            let Ok(coll) = self.dc.CreateGradientStopCollection(&s, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT) else { return };
            let (p0, p1) = if vertical { (v(r.x, r.y), v(r.x, r.bottom())) } else { (v(r.x, r.y), v(r.right(), r.y)) };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: p0, endPoint: p1 };
            if let Ok(b) = self.dc.CreateLinearGradientBrush(&props, None, &coll) {
                self.dc.FillRectangle(&r.d2d(), &b);
            }
        }
    }

    // ---------- Text ----------

    fn format(&mut self, size: f32, weight: u32) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0).round() as u32, weight);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        // SAFETY: DirectWrite object creation with valid arguments.
        let f = unsafe {
            self.dwrite.CreateTextFormat(
                &self.family,
                self.fonts.as_ref(),
                DWRITE_FONT_WEIGHT(weight as i32),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )
        }
        .ok()?;
        self.formats.insert(key, f.clone());
        Some(f)
    }

    /// A cached text layout: `lines` 0 means one line, no wrap; otherwise wrap and cut to that
    /// many lines with an ellipsis. `tracking` is letter spacing in ems.
    #[allow(clippy::too_many_arguments)]
    fn layout(&mut self, text: &str, size: f32, weight: u32, width: f32, lines: u32, align: Align, tracking: f32) -> Option<IDWriteTextLayout> {
        let key = (text.to_owned(), (size * 10.0) as u32, weight, width as u32, lines, align as u8 + if tracking != 0.0 { 8 } else { 0 });
        if let Some((l, used)) = self.layouts.get_mut(&key) {
            *used = self.frame;
            return Some(l.clone());
        }
        let format = self.format(size, weight)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // Wrapped text gets a fixed line height (1.2 em), so `lines` lines always fit.
        let line_h = size * 1.2;
        let max_h = if lines == 0 { size * 2.0 } else { line_h * lines as f32 + 1.0 };
        // SAFETY: DirectWrite calls on objects created here.
        let layout = unsafe {
            let l = self.dwrite.CreateTextLayout(&wide, &format, width.max(1.0), max_h).ok()?;
            let _ = l.SetWordWrapping(if lines == 0 { DWRITE_WORD_WRAPPING_NO_WRAP } else { DWRITE_WORD_WRAPPING_WRAP });
            if lines > 0 {
                let _ = l.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.8);
            }
            let _ = l.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if let Ok(sign) = self.dwrite.CreateEllipsisTrimmingSign(&format) {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = l.SetTrimming(&trim, &sign);
            }
            if tracking != 0.0
                && let Ok(l1) = l.cast::<IDWriteTextLayout1>()
            {
                let _ = l1.SetCharacterSpacing(0.0, tracking * size, 0.0, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 });
            }
            l
        };
        self.layouts.insert(key, (layout.clone(), self.frame));
        Some(layout)
    }

    /// Width and height of a single line of text.
    pub fn measure(&mut self, text: &str, size: f32, weight: u32) -> (f32, f32) {
        let Some(l) = self.layout(text, size, weight, 100_000.0, 0, Align::Left, 0.0) else { return (0.0, size) };
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: writes into a local struct.
        let _ = unsafe { l.GetMetrics(&mut m) };
        (m.widthIncludingTrailingWhitespace, m.height)
    }

    /// Height of wrapped text.
    pub fn text_height(&mut self, text: &str, size: f32, weight: u32, width: f32, lines: u32) -> f32 {
        let Some(l) = self.layout(text, size, weight, width, lines, Align::Left, 0.0) else { return size };
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: writes into a local struct.
        let _ = unsafe { l.GetMetrics(&mut m) };
        m.height
    }

    /// Draws text with its top-left at (x, y), within `width`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, s: &str, x: f32, y: f32, width: f32, size: f32, weight: u32, c: Color, align: Align, lines: u32, tracking: f32) {
        if s.is_empty() || c.3 <= 0.0 {
            return;
        }
        let Some(l) = self.layout(s, size, weight, width, lines, align, tracking) else { return };
        // SAFETY: inside BeginDraw/EndDraw.
        unsafe { self.dc.DrawTextLayout(v(x, y), &l, self.color(c), D2D1_DRAW_TEXT_OPTIONS_NONE) };
    }

    // ---------- Icons ----------

    fn icon_geometry(&mut self, name: &'static str, svg: &str, filled: bool) -> Option<ID2D1PathGeometry1> {
        if let Some(g) = self.icons.get(&(name, filled)) {
            return Some(g.clone());
        }
        // SAFETY: geometry built and closed here.
        let geom = unsafe {
            let geom = self.factory.CreatePathGeometry().ok()?;
            let sink = geom.Open().ok()?;
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            let mut b = PathBuilder { sink, open: false, filled };
            let data = if svg.trim_start().starts_with('<') { svg_to_path_data(svg) } else { svg.to_owned() };
            add_path_data(&mut b, &data);
            b.end(false);
            b.sink.Close().ok()?;
            geom
        };
        self.icons.insert((name, filled), geom.clone());
        Some(geom)
    }

    /// Draws a 24-unit icon into `r`: stroked (Lucide) or filled (brand marks).
    pub fn icon(&mut self, name: &'static str, svg: &str, r: Rect, c: Color, filled: bool, stroke: f32) {
        let Some(g) = self.icon_geometry(name, svg, filled) else { return };
        let s = r.w.min(r.h) / 24.0;
        let m = Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: r.x + (r.w - 24.0 * s) / 2.0, M32: r.y + (r.h - 24.0 * s) / 2.0 };
        // SAFETY: inside BeginDraw/EndDraw; the transform is restored after.
        unsafe {
            self.dc.SetTransform(&m);
            if filled {
                self.dc.FillGeometry(&g, self.color(c), None);
            } else {
                self.dc.DrawGeometry(&g, self.color(c), stroke, &self.round);
            }
            self.dc.SetTransform(&Matrix3x2::identity());
        }
    }

    // ---------- Images ----------

    /// Takes images the worker finished and turns them into bitmaps. Returns true when any
    /// arrived (the frame should be drawn again).
    pub fn take_decoded(&mut self) -> bool {
        let mut any = false;
        while let Ok(d) = self.decoded.try_recv() {
            any = true;
            if let Some(t) = d.tone {
                self.tones.insert(d.key.path.clone(), t);
            }
            let slot = match d.pixels {
                Some((w, h, px)) => {
                    let props = D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                        colorContext: ManuallyDrop::new(None),
                    };
                    // SAFETY: `px` holds `h` rows of `w * 4` bytes.
                    match unsafe { self.dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(px.as_ptr().cast()), w * 4, &props) } {
                        Ok(bmp) => {
                            self.image_bytes += u64::from(w) * u64::from(h) * 4;
                            Slot::Ready { bmp, w, h, ready_at: self.now, used: self.frame }
                        }
                        Err(_) => Slot::Failed,
                    }
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
        let frame = self.frame;
        let now = self.now;
        match self.images.get_mut(&key) {
            None => {
                self.images.insert(key.clone(), Slot::Loading);
                let _ = self.requests.send(key);
                Some(0.0)
            }
            Some(Slot::Loading) => Some(0.0),
            Some(Slot::Failed) => None,
            Some(Slot::Ready { bmp, w, h, ready_at, used }) => {
                *used = frame;
                let fade = (((now - *ready_at) / 250.0) as f32).clamp(0.0, 1.0);
                let (bw, bh) = (*w as f32, *h as f32);
                let bmp = bmp.clone();
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
                let alpha = opacity * (0.0 + fade);
                // SAFETY: inside BeginDraw/EndDraw.
                unsafe {
                    if radius > 0.0 {
                        let props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                            extendModeX: D2D1_EXTEND_MODE_CLAMP,
                            extendModeY: D2D1_EXTEND_MODE_CLAMP,
                            interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
                        };
                        if let Ok(brush) = self.dc.CreateBitmapBrush(&bmp, Some(&props), None) {
                            let s = dst.w / src.w;
                            brush.SetTransform(&Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: dst.h / src.h, M31: dst.x - src.x * s, M32: dst.y - src.y * (dst.h / src.h) });
                            brush.SetOpacity(alpha);
                            self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: dst.d2d(), radiusX: radius, radiusY: radius }, &brush);
                        }
                    } else {
                        self.dc.DrawBitmap(&bmp, Some(&dst.d2d()), alpha, D2D1_INTERPOLATION_MODE_LINEAR, Some(&src.d2d()), None);
                    }
                }
                Some(fade)
            }
        }
    }

    /// Gives memory back while Sofa waits behind a game: decoded art and text layouts go,
    /// Direct2D drops its caches and the driver trims (IDXGIDevice3::Trim, what Windows asks
    /// of apps going to the background). Everything is rebuilt on the next frame.
    pub fn release(&mut self) {
        self.images.clear();
        self.image_bytes = 0;
        self.layouts.clear();
        // SAFETY: plain calls on our own devices, outside BeginDraw/EndDraw.
        unsafe {
            self.device.ClearResources(0);
            self.d3d_context.ClearState();
            self.dxgi.Trim();
        }
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

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

/// Loads Figtree (a WOFF2 variable font) into a DirectWrite font collection.
fn load_font(dwrite: &IDWriteFactory5, woff2: &[u8]) -> Result<IDWriteFontCollection> {
    // SAFETY: DirectWrite unpacks the WOFF2 bytes into a stream we copy out of; the in-memory
    // loader copies the data (no owner object), so nothing borrowed outlives this call.
    unsafe {
        let stream = dwrite.UnpackFontFile(DWRITE_CONTAINER_TYPE_WOFF2, woff2.as_ptr().cast(), woff2.len() as u32)?;
        let size = stream.GetFileSize()?;
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut ctx: *mut std::ffi::c_void = std::ptr::null_mut();
        stream.ReadFileFragment(&mut ptr, 0, size, &mut ctx)?;
        let bytes = std::slice::from_raw_parts(ptr as *const u8, size as usize).to_vec();
        stream.ReleaseFileFragment(ctx);
        let loader = dwrite.CreateInMemoryFontFileLoader()?;
        dwrite.RegisterFontFileLoader(&loader)?;
        let file = loader.CreateInMemoryFontFileReference(dwrite, bytes.as_ptr().cast(), bytes.len() as u32, None)?;
        let builder = dwrite.CreateFontSetBuilder()?;
        builder.AddFontFile(&file)?;
        let set = builder.CreateFontSet()?;
        dwrite.CreateFontCollectionFromFontSet(&set)?.cast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(b: u8, g: u8, r: u8) -> Vec<u8> {
        [b, g, r, 255].repeat(24 * 24)
    }

    #[test]
    fn tone_finds_the_hue_of_vivid_art() {
        let (r, g, b) = tone(&solid(0, 0, 220), 24, 24).expect("red art has a tone").rgb8();
        assert!(r > 200 && g < 90 && b < 90, "{r} {g} {b}");
        let (r, g, b) = tone(&solid(200, 40, 20), 24, 24).expect("blue art has a tone").rgb8();
        assert!(b > 200 && r < 120, "{r} {g} {b}");
    }

    #[test]
    fn grey_art_has_no_tone() {
        assert!(tone(&solid(120, 120, 120), 24, 24).is_none());
        assert!(tone(&[], 0, 0).is_none());
    }

    #[test]
    fn hsl_matches_the_electron_colours() {
        assert_eq!(Color::parse("hsl(0 100% 50%)").map(Color::rgb8), Some((255, 0, 0)));
        assert_eq!(Color::hsl(120.0, 1.0, 0.25).rgb8(), (0, 128, 0));
    }
}
