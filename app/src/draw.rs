//! What both renderers share (`gfx.rs` on Windows, `linux/gfx.rs` on Linux): rectangles,
//! colours, how art fits its box, the art's tone, and Lucide's SVG paths read into figures.

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

// ---------- Icons: SVG path data ----------

pub type Pt = (f32, f32);

/// Where SVG path data goes: the renderer's own path builder.
pub trait PathSink {
    /// Starts a figure at `p`, leaving any open figure open.
    fn begin(&mut self, p: Pt);
    /// Ends the open figure, if one is open.
    fn end(&mut self, closed: bool);
    fn line(&mut self, p: Pt);
    fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt);
    fn quad(&mut self, c1: Pt, p: Pt);
    /// An elliptical arc from `from` to `to`, as SVG's `A` command describes it.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, from: Pt, to: Pt, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool);
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

/// Walks SVG path data (with `|` between `<path>` elements) into `b`.
pub fn walk_path(d: &str, b: &mut impl PathSink) {
    let mut n = Nums { s: d.as_bytes(), i: 0 };
    let (mut cur, mut start, mut last_ctrl): (Pt, Pt, Option<Pt>) = ((0.0, 0.0), (0.0, 0.0), None);
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
            cur = (0.0, 0.0);
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
        let at = |x: f32, y: f32, cur: Pt| if rel { (cur.0 + x, cur.1 + y) } else { (x, y) };
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
                b.line(p);
                cur = p;
                last_ctrl = None;
            }
            b'H' => {
                let x = n.num();
                cur = (if rel { cur.0 + x } else { x }, cur.1);
                b.line(cur);
                last_ctrl = None;
            }
            b'V' => {
                let y = n.num();
                cur = (cur.0, if rel { cur.1 + y } else { y });
                b.line(cur);
                last_ctrl = None;
            }
            b'C' | b'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&b'C') { at(n.num(), n.num(), cur) } else { last_ctrl.map_or(cur, |lc| (2.0 * cur.0 - lc.0, 2.0 * cur.1 - lc.1)) };
                let c2 = at(n.num(), n.num(), cur);
                let p = at(n.num(), n.num(), cur);
                b.cubic(c1, c2, p);
                last_ctrl = Some(c2);
                cur = p;
            }
            b'Q' => {
                let c1 = at(n.num(), n.num(), cur);
                let p = at(n.num(), n.num(), cur);
                b.quad(c1, p);
                last_ctrl = Some(c1);
                cur = p;
            }
            b'A' => {
                let (rx, ry, rot) = (n.num(), n.num(), n.num());
                let (large, sweep) = (n.flag(), n.flag());
                let p = at(n.num(), n.num(), cur);
                b.arc(cur, p, rx, ry, rot, large, sweep);
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

/// Converts Lucide's SVG elements (`<path d>`, `<circle>`, `<rect>`, `<line>`, `<polyline>`,
/// `<polygon>`) to path data.
pub fn svg_to_path_data(svg: &str) -> String {
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

/// An icon's path data: Lucide's SVG elements converted, brand marks as they are.
pub fn icon_path_data(svg: &str) -> String {
    if svg.trim_start().starts_with('<') { svg_to_path_data(svg) } else { svg.to_owned() }
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

    /// Records what the walker emits.
    #[derive(Default)]
    struct Log(Vec<String>);

    impl PathSink for Log {
        fn begin(&mut self, p: Pt) {
            self.0.push(format!("M{} {}", p.0, p.1));
        }
        fn end(&mut self, closed: bool) {
            self.0.push(if closed { "Z".into() } else { "E".into() });
        }
        fn line(&mut self, p: Pt) {
            self.0.push(format!("L{} {}", p.0, p.1));
        }
        fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt) {
            self.0.push(format!("C{} {} {} {} {} {}", c1.0, c1.1, c2.0, c2.1, p.0, p.1));
        }
        fn quad(&mut self, c1: Pt, p: Pt) {
            self.0.push(format!("Q{} {} {} {}", c1.0, c1.1, p.0, p.1));
        }
        fn arc(&mut self, from: Pt, to: Pt, rx: f32, ry: f32, _: f32, large: bool, sweep: bool) {
            self.0.push(format!("A{} {} {rx} {ry} {} {} {} {}", from.0, from.1, u8::from(large), u8::from(sweep), to.0, to.1));
        }
    }

    #[test]
    fn path_data_walks_relative_commands_and_elements() {
        let mut log = Log::default();
        walk_path("| M2 3 l1 1 2 0 h-1 v2 z | m1 1 a1 1 0 0 1 2 0 s1 1 2 2", &mut log);
        assert_eq!(log.0, ["M2 3", "L3 4", "L5 4", "L4 4", "L4 6", "Z", "M1 1", "A1 1 1 1 0 1 3 1", "C3 1 4 2 5 3"]);
    }

    #[test]
    fn numbers_without_separators_are_read_apart() {
        let mut log = Log::default();
        walk_path("M1.5.5L-12.77.04", &mut log);
        assert_eq!(log.0, ["M1.5 0.5", "L-12.77 0.04"]);
    }
}
