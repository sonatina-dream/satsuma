//! Editor models for the Crop Image and Add Background windows (SPEC §6.1, §6.2).
//!
//! Rectangles are in image pixels with a top-left origin.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn from_edges(l: f64, t: f64, r: f64, b: f64) -> Self {
        Rect::new(l, t, r - l, b - t)
    }

    /// Whole pixels, kept inside `bounds`.
    pub fn rounded(&self, bounds: (f64, f64)) -> Rect {
        let x = self.x.round().clamp(0.0, bounds.0 - 1.0);
        let y = self.y.round().clamp(0.0, bounds.1 - 1.0);
        let w = self.w.round().clamp(1.0, bounds.0 - x);
        let h = self.h.round().clamp(1.0, bounds.1 - y);
        Rect::new(x, y, w, h)
    }
}

// ------------------------------------------------------------------ crop

/// Aspect-ratio presets shown in the Crop window, in order.
pub const CROP_ASPECTS: [(&str, Option<(u32, u32)>); 6] = [
    ("Free", None),
    ("1:1", Some((1, 1))),
    ("16:9", Some((16, 9))),
    ("9:16", Some((9, 16))),
    ("4:3", Some((4, 3))),
    ("3:4", Some((3, 4))),
];

pub fn ratio_value(r: (u32, u32)) -> f64 {
    r.0 as f64 / r.1.max(1) as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Move,
}

/// What is under point `p` (image pixels): a handle within `tolerance`, the inside, or nothing.
pub fn hit_handle(r: Rect, p: (f64, f64), tolerance: f64) -> Option<Handle> {
    let (x, y) = p;
    let near = |a: f64, b: f64| (a - b).abs() <= tolerance;
    let within_x = x >= r.x - tolerance && x <= r.right() + tolerance;
    let within_y = y >= r.y - tolerance && y <= r.bottom() + tolerance;
    if !(within_x && within_y) {
        return None;
    }
    let (l, rr, t, b) = (near(x, r.x), near(x, r.right()), near(y, r.y), near(y, r.bottom()));
    Some(match (l, rr, t, b) {
        (true, _, true, _) => Handle::TopLeft,
        (_, true, true, _) => Handle::TopRight,
        (true, _, _, true) => Handle::BottomLeft,
        (_, true, _, true) => Handle::BottomRight,
        (true, ..) => Handle::Left,
        (_, true, ..) => Handle::Right,
        (_, _, true, _) => Handle::Top,
        (_, _, _, true) => Handle::Bottom,
        _ => Handle::Move,
    })
}

/// The largest rectangle with `aspect` (w/h) that fits in the image, centred where `current`
/// is centred (moved inward if it would stick out).
pub fn fit_aspect(current: Rect, aspect: f64, bounds: (f64, f64)) -> Rect {
    let (bw, bh) = bounds;
    let (mut w, mut h) = (bw, bw / aspect);
    if h > bh {
        h = bh;
        w = bh * aspect;
    }
    let (cx, cy) = current.center();
    let x = (cx - w / 2.0).clamp(0.0, bw - w);
    let y = (cy - h / 2.0).clamp(0.0, bh - h);
    Rect::new(x, y, w, h)
}

/// Resizes around the centre to exactly `w`×`h` (clamped to the image), for typed sizes.
pub fn set_size(current: Rect, w: f64, h: f64, bounds: (f64, f64)) -> Rect {
    let w = w.clamp(1.0, bounds.0);
    let h = h.clamp(1.0, bounds.1);
    let (cx, cy) = current.center();
    Rect::new((cx - w / 2.0).clamp(0.0, bounds.0 - w), (cy - h / 2.0).clamp(0.0, bounds.1 - h), w, h)
}

/// Drags `handle` of `start` by (dx, dy) image pixels, keeping `aspect` if set, at least
/// `min` pixels on each side, and inside the image.
pub fn drag(start: Rect, handle: Handle, dx: f64, dy: f64, aspect: Option<f64>, bounds: (f64, f64), min: f64) -> Rect {
    let (bw, bh) = bounds;
    let min = min.min(bw).min(bh);
    if handle == Handle::Move {
        let x = (start.x + dx).clamp(0.0, bw - start.w);
        let y = (start.y + dy).clamp(0.0, bh - start.h);
        return Rect::new(x, y, start.w, start.h);
    }
    let (mut l, mut t, mut r, mut b) = (start.x, start.y, start.right(), start.bottom());
    use Handle::*;
    let moves_left = matches!(handle, TopLeft | Left | BottomLeft);
    let moves_right = matches!(handle, TopRight | Right | BottomRight);
    let moves_top = matches!(handle, TopLeft | Top | TopRight);
    let moves_bottom = matches!(handle, BottomLeft | Bottom | BottomRight);
    if moves_left {
        l = (l + dx).clamp(0.0, r - min);
    }
    if moves_right {
        r = (r + dx).clamp(l + min, bw);
    }
    if moves_top {
        t = (t + dy).clamp(0.0, b - min);
    }
    if moves_bottom {
        b = (b + dy).clamp(t + min, bh);
    }
    let Some(a) = aspect else {
        return Rect::from_edges(l, t, r, b);
    };

    let (cx, cy) = start.center();
    let horizontal_edge = matches!(handle, Left | Right);
    let vertical_edge = matches!(handle, Top | Bottom);
    if horizontal_edge {
        // Width drives; height follows around the vertical centre.
        let mut w = r - l;
        let avail_h = 2.0 * cy.min(bh - cy);
        let avail_w = if moves_left { start.right() } else { bw - start.x };
        w = w.min(avail_h * a).min(avail_w).max(min);
        let h = w / a;
        let (l2, r2) = if moves_left { (start.right() - w, start.right()) } else { (start.x, start.x + w) };
        return Rect::new(l2, cy - h / 2.0, r2 - l2, h);
    }
    if vertical_edge {
        let mut h = b - t;
        let avail_w = 2.0 * cx.min(bw - cx);
        let avail_h = if moves_top { start.bottom() } else { bh - start.y };
        h = h.min(avail_w / a).min(avail_h).max(min);
        let w = h * a;
        let (t2, b2) = if moves_top { (start.bottom() - h, start.bottom()) } else { (start.y, start.y + h) };
        return Rect::new(cx - w / 2.0, t2, w, b2 - t2);
    }
    // Corner: anchor the opposite corner, grow to cover the drag, then fit the space.
    let anchor_x = if moves_left { start.right() } else { start.x };
    let anchor_y = if moves_top { start.bottom() } else { start.y };
    let (mut w, mut h) = (r - l, b - t);
    if w / h > a {
        h = w / a;
    } else {
        w = h * a;
    }
    let avail_w = if moves_left { anchor_x } else { bw - anchor_x };
    let avail_h = if moves_top { anchor_y } else { bh - anchor_y };
    if w > avail_w {
        w = avail_w;
        h = w / a;
    }
    if h > avail_h {
        h = avail_h;
        w = h * a;
    }
    let x = if moves_left { anchor_x - w } else { anchor_x };
    let y = if moves_top { anchor_y - h } else { anchor_y };
    Rect::new(x, y, w, h)
}

/// A crop to apply: a rectangle as fractions of the image (so it also fits other images in a
/// selection), and an optional exact output size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropSpec {
    pub rect: Rect,
    pub output: Option<(u32, u32)>,
}

impl CropSpec {
    pub fn from_pixels(r: Rect, image: (f64, f64), output: Option<(u32, u32)>) -> Self {
        CropSpec {
            rect: Rect::new(r.x / image.0, r.y / image.1, r.w / image.0, r.h / image.1),
            output,
        }
    }

    /// Centred crop with an aspect ratio (for the command line).
    pub fn centered(aspect: Option<(u32, u32)>, image: (f64, f64)) -> Self {
        let full = Rect::new(0.0, 0.0, image.0, image.1);
        let r = match aspect {
            Some(a) => fit_aspect(full, ratio_value(a), image),
            None => full,
        };
        Self::from_pixels(r, image, None)
    }

    pub fn pixels(&self, image: (f64, f64)) -> Rect {
        Rect::new(self.rect.x * image.0, self.rect.y * image.1, self.rect.w * image.0, self.rect.h * image.1)
            .rounded(image)
    }
}

// ------------------------------------------------------------------ add background

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientPreset {
    pub name: &'static str,
    /// Colour stops from top-left to bottom-right.
    pub stops: [(u8, u8, u8); 3],
}

pub const GRADIENTS: [GradientPreset; 9] = [
    GradientPreset { name: "Sunset", stops: [(255, 94, 98), (255, 140, 66), (255, 200, 87)] },
    GradientPreset { name: "Citrus", stops: [(247, 151, 30), (255, 196, 0), (255, 236, 140)] },
    GradientPreset { name: "Dream", stops: [(251, 194, 235), (190, 180, 250), (166, 193, 238)] },
    GradientPreset { name: "Frost", stops: [(240, 245, 252), (214, 228, 244), (196, 214, 238)] },
    GradientPreset { name: "Orchid", stops: [(218, 34, 255), (173, 60, 240), (120, 70, 230)] },
    GradientPreset { name: "Ember", stops: [(241, 39, 17), (245, 110, 30), (245, 175, 25)] },
    GradientPreset { name: "Lagoon", stops: [(67, 198, 172), (40, 120, 160), (25, 22, 84)] },
    GradientPreset { name: "Aqua", stops: [(128, 208, 199), (60, 150, 190), (19, 84, 122)] },
    GradientPreset { name: "Meadow", stops: [(168, 224, 99), (110, 190, 80), (60, 140, 70)] },
];

pub const SOLIDS: [(&str, (u8, u8, u8)); 10] = [
    ("White", (255, 255, 255)),
    ("Yellow", (250, 204, 21)),
    ("Lime", (132, 204, 22)),
    ("Green", (34, 160, 90)),
    ("Teal", (20, 150, 140)),
    ("Cyan", (34, 200, 220)),
    ("Blue", (37, 99, 235)),
    ("Navy", (23, 37, 84)),
    ("Purple", (126, 58, 220)),
    ("Black", (12, 12, 14)),
];

/// Canvas ratios offered by Add Background; `None` is Auto (padding on every side).
pub const FRAME_RATIOS: [(&str, Option<(u32, u32)>); 4] =
    [("Auto", None), ("1:1", Some((1, 1))), ("16:9", Some((16, 9))), ("4:3", Some((4, 3)))];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Background {
    Gradient(usize),
    Solid { r: u8, g: u8, b: u8 },
    Image(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameSpec {
    pub background: Background,
    /// Space around the photo, in output pixels.
    pub padding: f64,
    /// Corner radius of the photo, in pixels.
    pub corners: f64,
    /// Shadow blur radius, in pixels (0 = none).
    pub shadow: f64,
    pub ratio: Option<(u32, u32)>,
}

impl FrameSpec {
    /// Starting values scaled to the photo, like the original's defaults.
    pub fn defaults_for(image: (f64, f64)) -> Self {
        let long = image.0.max(image.1);
        let short = image.0.min(image.1);
        FrameSpec {
            background: Background::Gradient(0),
            padding: (long * 0.08).round(),
            corners: (short * 0.03).round(),
            shadow: (long * 0.012).round().max(4.0),
            ratio: None,
        }
    }

    /// Slider maxima for an image: (padding, corners, shadow).
    pub fn limits_for(image: (f64, f64)) -> (f64, f64, f64) {
        let long = image.0.max(image.1);
        let short = image.0.min(image.1);
        ((long * 0.5).round(), (short * 0.5).round(), (long * 0.06).round().max(20.0))
    }
}

/// Canvas size and where the photo goes, for an image of `image` pixels.
pub fn frame_layout(image: (f64, f64), padding: f64, ratio: Option<(u32, u32)>) -> ((f64, f64), Rect) {
    let (iw, ih) = image;
    let p = padding.max(0.0);
    let (mut cw, mut ch) = (iw + 2.0 * p, ih + 2.0 * p);
    if let Some(r) = ratio {
        let a = ratio_value(r);
        if cw / ch > a {
            ch = cw / a;
        } else {
            cw = ch * a;
        }
    }
    let (cw, ch) = (cw.round(), ch.round());
    let rect = Rect::new(((cw - iw) / 2.0).round(), ((ch - ih) / 2.0).round(), iw, ih);
    ((cw, ch), rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMG: (f64, f64) = (2550.0, 3300.0);

    fn close(a: Rect, b: Rect) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.w - b.w).abs() < 1e-6 && (a.h - b.h).abs() < 1e-6
    }

    #[test]
    fn handles_are_found_at_edges_and_corners() {
        let r = Rect::new(100.0, 100.0, 400.0, 300.0);
        assert_eq!(hit_handle(r, (100.0, 100.0), 10.0), Some(Handle::TopLeft));
        assert_eq!(hit_handle(r, (500.0, 400.0), 10.0), Some(Handle::BottomRight));
        assert_eq!(hit_handle(r, (300.0, 95.0), 10.0), Some(Handle::Top));
        assert_eq!(hit_handle(r, (505.0, 250.0), 10.0), Some(Handle::Right));
        assert_eq!(hit_handle(r, (300.0, 250.0), 10.0), Some(Handle::Move));
        assert_eq!(hit_handle(r, (50.0, 50.0), 10.0), None);
    }

    #[test]
    fn free_drag_of_bottom_edge_matches_the_demo() {
        // E10: the bottom edge of a 2550×3300 page is dragged up to 1677 px tall.
        let full = Rect::new(0.0, 0.0, IMG.0, IMG.1);
        let r = drag(full, Handle::Bottom, 0.0, 1677.0 - 3300.0, None, IMG, 20.0);
        assert!(close(r, Rect::new(0.0, 0.0, 2550.0, 1677.0)));
    }

    #[test]
    fn drags_stay_inside_and_above_minimum() {
        let r = Rect::new(100.0, 100.0, 400.0, 300.0);
        let out = drag(r, Handle::TopLeft, -500.0, -500.0, None, IMG, 20.0);
        assert!(close(out, Rect::new(0.0, 0.0, 500.0, 400.0)));
        let tiny = drag(r, Handle::Right, -1000.0, 0.0, None, IMG, 20.0);
        assert_eq!(tiny.w, 20.0);
        let moved = drag(r, Handle::Move, 10_000.0, -10_000.0, None, IMG, 20.0);
        assert!(close(moved, Rect::new(IMG.0 - 400.0, 0.0, 400.0, 300.0)));
    }

    #[test]
    fn aspect_presets_fit_the_image() {
        let full = Rect::new(0.0, 0.0, IMG.0, IMG.1);
        // E10: choosing 1:1 gives 2550 × 2550.
        let sq = fit_aspect(full, 1.0, IMG);
        assert_eq!((sq.w, sq.h), (2550.0, 2550.0));
        assert_eq!(sq.y, 375.0);
        let wide = fit_aspect(full, 16.0 / 9.0, IMG);
        assert!((wide.w - 2550.0).abs() < 1e-9 && (wide.h - 2550.0 * 9.0 / 16.0).abs() < 1e-9);
        let tall = fit_aspect(full, 9.0 / 16.0, IMG);
        assert!((tall.h - 3300.0).abs() < 1e-9 && tall.w < 2550.0);
    }

    #[test]
    fn aspect_drags_keep_the_ratio() {
        let start = Rect::new(500.0, 500.0, 1000.0, 1000.0);
        for handle in [Handle::TopLeft, Handle::TopRight, Handle::BottomLeft, Handle::BottomRight, Handle::Left, Handle::Right, Handle::Top, Handle::Bottom] {
            for (dx, dy) in [(300.0, 120.0), (-400.0, -900.0), (2000.0, 2000.0), (-2000.0, 50.0)] {
                let r = drag(start, handle, dx, dy, Some(1.0), IMG, 20.0);
                assert!((r.w - r.h).abs() < 1e-6, "{handle:?} {dx},{dy} -> {r:?}");
                assert!(r.x >= -1e-6 && r.y >= -1e-6 && r.right() <= IMG.0 + 1e-6 && r.bottom() <= IMG.1 + 1e-6, "{handle:?} {r:?}");
            }
        }
    }

    #[test]
    fn typed_sizes_and_specs() {
        let r = set_size(Rect::new(0.0, 0.0, 2550.0, 3300.0), 1000.0, 500.0, IMG);
        assert_eq!((r.x, r.y, r.w, r.h), (775.0, 1400.0, 1000.0, 500.0));
        let spec = CropSpec::from_pixels(r, IMG, None);
        assert_eq!(spec.pixels(IMG), r);
        // The same fractions apply to an image of another size.
        let other = spec.pixels((1275.0, 1650.0));
        assert_eq!((other.w, other.h), (500.0, 250.0));
        let c = CropSpec::centered(Some((1, 1)), (400.0, 200.0));
        assert_eq!(c.pixels((400.0, 200.0)), Rect::new(100.0, 0.0, 200.0, 200.0));
    }

    #[test]
    fn frame_layouts() {
        let ((w, h), r) = frame_layout((1000.0, 500.0), 100.0, None);
        assert_eq!((w, h), (1200.0, 700.0));
        assert_eq!((r.x, r.y), (100.0, 100.0));
        let ((w, h), r) = frame_layout((1000.0, 500.0), 100.0, Some((1, 1)));
        assert_eq!((w, h), (1200.0, 1200.0));
        assert_eq!((r.x, r.y), (100.0, 350.0));
        let ((w, h), _) = frame_layout((500.0, 1000.0), 50.0, Some((16, 9)));
        assert_eq!(h, 1100.0);
        assert!((w - (1100.0f64 * 16.0 / 9.0).round()).abs() < 1e-9);
    }

    #[test]
    fn frame_defaults_scale_with_the_photo() {
        let d = FrameSpec::defaults_for((2550.0, 1677.0));
        assert_eq!(d.padding, 204.0);
        assert_eq!(d.corners, 50.0);
        assert_eq!(d.background, Background::Gradient(0));
        let (p, c, _) = FrameSpec::limits_for((2550.0, 1677.0));
        assert!(p >= d.padding && c >= d.corners);
    }
}

// ------------------------------------------------------------------ edit photo

/// Slider groups in the Edit Photo window ("Fine controls for light, color, and detail", E2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Light,
    Color,
    Detail,
    Effects,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::Light, Section::Color, Section::Detail, Section::Effects];

    pub fn name(self) -> &'static str {
        match self {
            Section::Light => "Light",
            Section::Color => "Color",
            Section::Detail => "Detail",
            Section::Effects => "Effects",
        }
    }
}

pub struct AdjustmentInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub section: Section,
    pub min: f64,
    pub max: f64,
}

const fn adj(key: &'static str, label: &'static str, section: Section, min: f64) -> AdjustmentInfo {
    AdjustmentInfo { key, label, section, min, max: 100.0 }
}

/// Every adjustment, in the order of Tangerine's `ImageAdjustments` fields (E11).
pub const ADJUSTMENTS: [AdjustmentInfo; 20] = [
    adj("exposure", "Exposure", Section::Light, -100.0),
    adj("brightness", "Brightness", Section::Light, -100.0),
    adj("contrast", "Contrast", Section::Light, -100.0),
    adj("highlights", "Highlights", Section::Light, -100.0),
    adj("shadows", "Shadows", Section::Light, -100.0),
    adj("whites", "Whites", Section::Light, -100.0),
    adj("blacks", "Blacks", Section::Light, -100.0),
    adj("temperature", "Temperature", Section::Color, -100.0),
    adj("tint", "Tint", Section::Color, -100.0),
    adj("saturation", "Saturation", Section::Color, -100.0),
    adj("vibrance", "Vibrance", Section::Color, -100.0),
    adj("gamma", "Gamma", Section::Light, -100.0),
    adj("clarity", "Clarity", Section::Detail, 0.0),
    adj("texture", "Texture", Section::Detail, 0.0),
    adj("sharpness", "Sharpness", Section::Detail, 0.0),
    adj("dehaze", "Dehaze", Section::Detail, 0.0),
    adj("fade", "Fade", Section::Effects, 0.0),
    adj("vignette", "Vignette", Section::Effects, -100.0),
    adj("grain", "Grain", Section::Effects, 0.0),
    adj("noiseReduction", "Noise Reduction", Section::Detail, 0.0),
];

/// Slider values, each 0 when untouched; ranges are in [`ADJUSTMENTS`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Adjustments(pub [f64; 20]);

impl Adjustments {
    pub fn index(key: &str) -> Option<usize> {
        ADJUSTMENTS.iter().position(|a| a.key == key)
    }

    pub fn get(&self, key: &str) -> f64 {
        Self::index(key).map_or(0.0, |i| self.0[i])
    }

    /// Sets a value, clamped to its range.
    pub fn set(&mut self, i: usize, value: f64) {
        if let Some(info) = ADJUSTMENTS.get(i) {
            self.0[i] = value.round().clamp(info.min, info.max);
        }
    }

    pub fn is_identity(&self) -> bool {
        self.0.iter().all(|v| *v == 0.0)
    }

    /// Indices of the sliders in a section, in display order.
    pub fn section(section: Section) -> Vec<usize> {
        (0..ADJUSTMENTS.len()).filter(|i| ADJUSTMENTS[*i].section == section).collect()
    }
}

#[cfg(test)]
mod adjustment_tests {
    use super::*;

    #[test]
    fn every_adjustment_is_in_one_section_and_has_a_sane_range() {
        let total: usize = Section::ALL.iter().map(|s| Adjustments::section(*s).len()).sum();
        assert_eq!(total, ADJUSTMENTS.len());
        for a in &ADJUSTMENTS {
            assert!(a.min < 0.0 + 1e-9 && a.max == 100.0 && (a.min == 0.0 || a.min == -100.0), "{}", a.key);
        }
        // Light holds the most sliders and must fit the window: 8 rows.
        assert_eq!(Adjustments::section(Section::Light).len(), 8);
    }

    #[test]
    fn values_clamp_and_identity() {
        let mut a = Adjustments::default();
        assert!(a.is_identity());
        let i = Adjustments::index("sharpness").unwrap();
        a.set(i, -40.0);
        assert_eq!(a.get("sharpness"), 0.0);
        a.set(Adjustments::index("exposure").unwrap(), 250.4);
        assert_eq!(a.get("exposure"), 100.0);
        assert!(!a.is_identity());
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Adjustments>(&json).unwrap(), a);
    }
}
