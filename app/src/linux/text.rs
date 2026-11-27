//! Text on Linux: Figtree, and the fonts fontconfig suggests for characters Figtree lacks,
//! shaped and rasterized with swash, laid out the way DirectWrite lays out Sofa's text on
//! Windows: one line trimmed with an ellipsis, or wrapped at spaces into at most `lines` lines of
//! 1.2 em with an ellipsis on the last one.
//!
//! Sofa's look was made with DirectWrite, which draws Figtree (a variable font) its own way:
//! measured on Windows 11 (widths at three sizes, `dw_weight`), Sofa's 400 is the weight axis at
//! 350, its 500 and 600 are 300 with DirectWrite's bold simulation, its 700 is 650 with it; the
//! first baseline of a line sits 1.0791 em down and a line is 1.3301 em high. The same is done
//! here, so text has the same weight, width and place on both systems.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::shape::ShapeContext;
use swash::zeno::{Format, Vector};
use swash::{CacheKey, FontRef};

use crate::draw::Align;

/// The weight axis value and bold simulation DirectWrite uses for a weight Sofa asks for.
fn dw_weight(weight: u32) -> (f32, bool) {
    match weight {
        0..=349 => (300.0, false),
        350..=449 => (350.0, false),
        450..=649 => (300.0, true),
        650..=749 => (650.0, true),
        _ => (900.0, true),
    }
}

/// DirectWrite's bold simulation: outlines widened, and each advance lengthened, by this much
/// of the size.
const BOLD_EM: f32 = 0.0197;
/// DirectWrite's single line: baseline and height, in ems of the size.
const DW_BASELINE: f32 = 1.0791;
const DW_HEIGHT: f32 = 1.3301;
/// Partial coverage brightened as DirectWrite's grayscale contrast does (`c * (k + 1) / (c * k
/// + 1)`), so thin strokes read as they do on Windows.
const CONTRAST: f32 = 0.8;

/// Coverage after DirectWrite-like contrast, for each 8-bit value.
pub fn contrast_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = (c * (CONTRAST + 1.0) / (c * CONTRAST + 1.0) * 255.0).round() as u8;
    }
    t
}

enum Data {
    Static(&'static [u8]),
    Mapped(memmap2::Mmap),
}

struct Face {
    data: Data,
    offset: u32,
    key: CacheKey,
    /// The font has a weight axis (Figtree does; fallback fonts usually do not).
    wght: bool,
}

impl Face {
    fn new(data: Data, index: usize) -> Option<Face> {
        let bytes: &[u8] = match &data {
            Data::Static(b) => b,
            Data::Mapped(m) => m,
        };
        let f = FontRef::from_index(bytes, index)?;
        let wght = f.variations().any(|v| v.tag() == swash::tag_from_bytes(b"wght"));
        let (offset, key) = (f.offset, f.key);
        Some(Face { data, offset, key, wght })
    }
    fn font(&self) -> FontRef<'_> {
        let data: &[u8] = match &self.data {
            Data::Static(b) => b,
            Data::Mapped(m) => m,
        };
        FontRef { data, offset: self.offset, key: self.key }
    }
    fn covers(&self, c: char) -> bool {
        self.font().charmap().map(c) != 0
    }
}

/// A glyph as rasterized: coverage (or colour, for emoji) and where it sits from the pen.
pub struct Glyph {
    pub left: i32,
    pub top: i32,
    pub w: u32,
    pub h: u32,
    pub color: bool,
    pub data: Vec<u8>,
}

/// A glyph placed on a line: its font, id and pen position from the line's start.
#[derive(Clone, Copy)]
pub struct Placed {
    pub face: u16,
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

pub struct Line {
    pub glyphs: Vec<Placed>,
    /// From the layout's left edge (alignment).
    pub x: f32,
    /// From the layout's top.
    pub baseline: f32,
}

pub struct Layout {
    pub lines: Vec<Line>,
    /// The widest line, trailing spaces included (DirectWrite's widthIncludingTrailingWhitespace).
    pub width: f32,
    pub height: f32,
}

/// A shaped character cluster: its source bytes, advance and glyphs (`glyphs` indexes `Shaped::g`).
#[derive(Clone)]
struct Cl {
    adv: f32,
    space: bool,
    newline: bool,
    glyphs: std::ops::Range<usize>,
}

struct Shaped {
    cl: Vec<Cl>,
    /// Glyphs with offsets from their cluster's pen position.
    g: Vec<Placed>,
}

type LayoutKey = (String, u32, u32, u32, u32, u8);
type GlyphKey = (u16, u16, u32, u16, u8);

pub struct Text {
    faces: Vec<Face>,
    /// Which face draws a character Figtree lacks (None: no font has it).
    fallback: HashMap<char, Option<usize>>,
    /// Font files already asked for, as path and index.
    loaded: HashSet<(String, usize)>,
    shaper: ShapeContext,
    scaler: ScaleContext,
    glyphs: HashMap<GlyphKey, Option<Glyph>>,
    layouts: HashMap<LayoutKey, (Rc<Layout>, u64)>,
}

impl Text {
    pub fn new(figtree: &'static [u8]) -> Text {
        let faces: Vec<Face> = Face::new(Data::Static(figtree), 0).into_iter().collect();
        Text { faces, fallback: HashMap::new(), loaded: HashSet::new(), shaper: ShapeContext::new(), scaler: ScaleContext::new(), glyphs: HashMap::new(), layouts: HashMap::new() }
    }

    /// Drops what was not drawn for a while (`all`: everything, giving memory back).
    pub fn forget(&mut self, frame: u64, all: bool) {
        if all {
            self.layouts.clear();
            self.glyphs.clear();
            self.layouts.shrink_to_fit();
            self.glyphs.shrink_to_fit();
        } else {
            self.layouts.retain(|_, (_, used)| frame - *used < 600);
            if self.glyphs.len() > 4000 {
                self.glyphs.clear();
            }
        }
    }

    /// The face that draws `c`: Figtree, a fallback loaded before, or one fontconfig suggests.
    fn face_for(&mut self, c: char) -> usize {
        if c.is_control() || c.is_whitespace() || self.faces.first().is_none_or(|f| f.covers(c)) {
            return 0;
        }
        if let Some(r) = self.fallback.get(&c) {
            return r.unwrap_or(0);
        }
        let mut found = (1..self.faces.len()).find(|&i| self.faces[i].covers(c));
        if found.is_none()
            && let Some((path, index)) = fc_match(c)
            && self.loaded.insert((path.clone(), index))
            && let Ok(file) = std::fs::File::open(&path)
            // SAFETY: a read-only map of a font file; fonts are not rewritten while in use.
            && let Ok(map) = unsafe { memmap2::Mmap::map(&file) }
            && let Some(face) = Face::new(Data::Mapped(map), index)
            && face.covers(c)
        {
            self.faces.push(face);
            found = Some(self.faces.len() - 1);
        }
        self.fallback.insert(c, found);
        found.unwrap_or(0)
    }

    fn shape(&mut self, text: &str, size: f32, weight: u32, tracking: f32) -> Shaped {
        // DirectWrite's bold simulation lengthens every glyph's advance.
        let (wght, simulated) = dw_weight(weight);
        let bold = if simulated { BOLD_EM * size } else { 0.0 };
        // Runs of one face each.
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        for (i, c) in text.char_indices() {
            let f = if c.is_whitespace() || c.is_control() { runs.last().map_or(0, |r| r.2) } else { self.face_for(c) };
            match runs.last_mut() {
                Some(r) if r.2 == f => r.1 = i + c.len_utf8(),
                _ => runs.push((i, i + c.len_utf8(), f)),
            }
        }
        let mut out = Shaped { cl: Vec::new(), g: Vec::new() };
        for (start, end, face) in runs {
            let run = &text[start..end];
            let f = &self.faces[face];
            let mut shaper = self.shaper.builder(f.font()).size(size);
            if f.wght {
                shaper = shaper.variations([("wght", wght)]);
            }
            let mut shaper = shaper.build();
            shaper.add_str(run);
            let (cl, g) = (&mut out.cl, &mut out.g);
            shaper.shape_with(|c| {
                let s = (c.source.start as usize).min(run.len());
                let e = (c.source.end as usize).clamp(s, run.len());
                let src = &run[s..e];
                let first = g.len();
                let mut adv = 0.0;
                for gl in c.glyphs {
                    g.push(Placed { face: face as u16, id: gl.id, x: adv + gl.x, y: gl.y });
                    adv += gl.advance + bold;
                }
                let newline = src.contains('\n');
                if !newline && !src.is_empty() {
                    adv += tracking * size * src.chars().count() as f32;
                }
                cl.push(Cl { adv: if newline { 0.0 } else { adv }, space: !src.is_empty() && src.chars().all(|ch| ch.is_whitespace() && ch != '\n'), newline, glyphs: first..g.len() });
            });
        }
        out
    }

    /// The ellipsis in Figtree at this size: its glyph and advance.
    fn ellipsis(&mut self, size: f32, weight: u32) -> Shaped {
        self.shape("\u{2026}", size, weight, 0.0)
    }

    /// A cached layout. `lines` 0: one line, trimmed to `width` with an ellipsis; otherwise
    /// wrapped and cut to that many lines. `tracking` is letter spacing in ems.
    #[allow(clippy::too_many_arguments)]
    pub fn layout(&mut self, text: &str, size: f32, weight: u32, width: f32, lines: u32, align: Align, tracking: f32, frame: u64) -> Rc<Layout> {
        let key = (text.to_owned(), (size * 10.0) as u32, weight, width as u32, lines, align as u8 + if tracking != 0.0 { 8 } else { 0 });
        if let Some((l, used)) = self.layouts.get_mut(&key) {
            *used = frame;
            return l.clone();
        }
        let shaped = self.shape(text, size, weight, tracking);
        let ell = self.ellipsis(size, weight);
        let ell_adv: f32 = ell.cl.iter().map(|c| c.adv).sum();
        let cl = &shaped.cl;
        // Lines as cluster ranges.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        if lines == 0 {
            ranges.push((0, cl.len()));
        } else {
            let mut i = 0;
            while i < cl.len() {
                let start = i;
                let mut w = 0.0;
                let mut brk: Option<usize> = None;
                let mut end = cl.len();
                let mut next = cl.len();
                while i < cl.len() {
                    let c = &cl[i];
                    if c.newline {
                        end = i;
                        next = i + 1;
                        break;
                    }
                    if !c.space && w + c.adv > width && i > start {
                        // Wrap at the last space; a word longer than the line breaks anywhere.
                        let at = brk.unwrap_or(i);
                        end = at;
                        next = at;
                        break;
                    }
                    w += c.adv;
                    if c.space {
                        brk = Some(i + 1);
                    }
                    i += 1;
                }
                if i >= cl.len() {
                    end = cl.len();
                    next = cl.len();
                }
                ranges.push((start, end));
                i = next;
                if end == cl.len() && next == cl.len() {
                    break;
                }
            }
            if ranges.is_empty() {
                ranges.push((0, 0));
            }
        }
        let max = if lines == 0 { 1 } else { lines as usize };
        let cut = ranges.len() > max;
        ranges.truncate(max);
        let line_h = size * 1.2;
        let mut out = Layout { lines: Vec::new(), width: 0.0, height: if lines == 0 { size * DW_HEIGHT } else { line_h * ranges.len() as f32 } };
        let count = ranges.len();
        for (n, (s, mut e)) in ranges.into_iter().enumerate() {
            let full: f32 = cl[s..e].iter().map(|c| c.adv).sum();
            let last = n + 1 == count;
            let trim = (lines == 0 && full > width + 0.01) || (last && cut);
            if trim {
                // Character trimming: as much as fits with the ellipsis after it.
                let mut w = 0.0;
                let mut k = s;
                while k < e && w + cl[k].adv + ell_adv <= width {
                    w += cl[k].adv;
                    k += 1;
                }
                e = k;
                while e > s && cl[e - 1].space {
                    e -= 1;
                }
            }
            let mut glyphs = Vec::new();
            let mut pen = 0.0;
            for c in &cl[s..e] {
                for g in &shaped.g[c.glyphs.clone()] {
                    glyphs.push(Placed { x: pen + g.x, ..*g });
                }
                pen += c.adv;
            }
            let with_spaces = pen;
            let mut visible = pen;
            for c in cl[s..e].iter().rev() {
                if !c.space {
                    break;
                }
                visible -= c.adv;
            }
            if trim {
                for c in &ell.cl {
                    for g in &ell.g[c.glyphs.clone()] {
                        glyphs.push(Placed { x: pen + g.x, ..*g });
                    }
                    pen += c.adv;
                }
                visible = visible.max(pen.min(with_spaces + ell_adv));
            }
            let room = if width >= 50_000.0 { visible } else { width };
            let x = match align {
                Align::Left => 0.0,
                Align::Center => (room - visible) / 2.0,
                Align::Right => room - visible,
            };
            let baseline = if lines == 0 { size * DW_BASELINE } else { n as f32 * line_h + line_h * 0.8 };
            out.width = out.width.max(if trim { pen } else { with_spaces });
            out.lines.push(Line { glyphs, x, baseline });
        }
        let l = Rc::new(out);
        self.layouts.insert(key, (l.clone(), frame));
        l
    }

    /// A glyph rasterized at `size` with its pen `fx` quarter pixels right of a whole pixel.
    pub fn glyph(&mut self, face: u16, id: u16, size: f32, weight: u32, fx: u8) -> Option<&Glyph> {
        let f = self.faces.get(face as usize)?;
        let (wght, bold) = dw_weight(weight);
        let key = (face, id, (size * 16.0).round() as u32, wght as u16 + u16::from(bold), fx);
        if !self.glyphs.contains_key(&key) {
            let mut builder = self.scaler.builder(f.font()).size(size).hint(false);
            if f.wght {
                builder = builder.variations([("wght", wght)]);
            }
            let mut scaler = builder.build();
            let image = Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline])
                .format(Format::Alpha)
                .offset(Vector::new(f32::from(fx) / 4.0, 0.0))
                .embolden(if bold { BOLD_EM * size } else { 0.0 })
                .render(&mut scaler, id);
            let g = image.filter(|i| i.placement.width > 0 && i.placement.height > 0).map(|i| Glyph {
                left: i.placement.left,
                top: i.placement.top,
                w: i.placement.width,
                h: i.placement.height,
                color: i.content == Content::Color,
                data: i.data,
            });
            self.glyphs.insert(key, g);
        }
        self.glyphs.get(&key)?.as_ref()
    }
}

/// A font file fontconfig suggests for a character: its path and index in the file.
fn fc_match(c: char) -> Option<(String, usize)> {
    let out = std::process::Command::new("fc-match").arg("--format=%{file}\n%{index}\n").arg(format!("sans-serif:charset={:x}", c as u32)).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.lines();
    let file = parts.next()?.trim().to_owned();
    let index = parts.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    (!file.is_empty()).then_some((file, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIGTREE: &[u8] = include_bytes!("../../assets/fonts/figtree.ttf");

    #[test]
    fn one_line_is_trimmed_with_an_ellipsis() {
        let mut t = Text::new(FIGTREE);
        let full = t.layout("Assassin's Creed IV Black Flag", 20.0, 600, 100_000.0, 0, Align::Left, 0.0, 0);
        assert!(full.width > 150.0, "{}", full.width);
        let cut = t.layout("Assassin's Creed IV Black Flag", 20.0, 600, 120.0, 0, Align::Left, 0.0, 0);
        assert!(cut.width <= 120.5, "{}", cut.width);
        assert!(cut.lines[0].glyphs.len() < full.lines[0].glyphs.len());
        assert!((full.height - 20.0 * DW_HEIGHT).abs() < 0.01, "a line is as high as DirectWrite's: {}", full.height);
    }

    #[test]
    fn wrapping_keeps_words_and_stops_at_the_line_count() {
        let mut t = Text::new(FIGTREE);
        let l = t.layout("Close it first, or keep it open and start another game as well.", 16.0, 400, 160.0, 2, Align::Left, 0.0, 0);
        assert_eq!(l.lines.len(), 2);
        assert!((l.height - 2.0 * 16.0 * 1.2).abs() < 0.01);
        let all = t.layout("one two", 16.0, 400, 1000.0, 3, Align::Left, 0.0, 0);
        assert_eq!(all.lines.len(), 1);
    }

    #[test]
    fn centred_text_sits_in_the_middle() {
        let mut t = Text::new(FIGTREE);
        let l = t.layout("Home", 20.0, 600, 400.0, 0, Align::Center, 0.0, 0);
        let w = t.layout("Home", 20.0, 600, 100_000.0, 0, Align::Left, 0.0, 0).width;
        assert!((l.lines[0].x - (400.0 - w) / 2.0).abs() < 0.5);
    }

    #[test]
    fn glyphs_rasterize() {
        let mut t = Text::new(FIGTREE);
        let l = t.layout("A", 32.0, 700, 100_000.0, 0, Align::Left, 0.0, 0);
        let g = l.lines[0].glyphs[0];
        let img = t.glyph(g.face, g.id, 32.0, 700, 0).expect("A has an outline");
        assert!(img.w > 10 && img.h > 10 && img.data.iter().any(|&b| b > 200));
    }

    #[test]
    fn widths_follow_directwrite() {
        // DirectWrite's widths of "Night Drive", measured on Windows 11.
        let mut t = Text::new(FIGTREE);
        let dw = [(13.333, 400, 67.34), (15.0, 400, 75.76), (46.667, 400, 235.70), (13.333, 600, 69.71), (18.333, 500, 95.85), (46.667, 600, 244.0), (13.333, 700, 72.76), (18.333, 700, 100.04), (46.667, 700, 254.66)];
        for (size, weight, dw) in dw {
            let w = t.layout("Night Drive", size, weight, 100_000.0, 0, Align::Left, 0.0, 0).width;
            assert!((w - dw).abs() / dw < 0.002, "{size} px, {weight}: {w} against {dw}");
        }
    }

    #[test]
    fn contrast_keeps_the_ends() {
        let t = contrast_table();
        assert_eq!((t[0], t[255]), (0, 255));
        assert!(t[128] > 128);
    }
}
