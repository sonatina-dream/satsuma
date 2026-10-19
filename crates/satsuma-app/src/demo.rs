//! The README demo: `satsuma render-demo <work-dir> <docs-dir>` (built with `--features demo`).
//!
//! Nothing is captured from the screen. The scene is drawn: a plain desktop with invented
//! sample files, the app's own wheel, progress and editor views rendered offscreen, and a
//! scripted pointer. Sample files are written to `<work-dir>`; frames go to ffmpeg as a
//! lossless `<work-dir>/master.mkv`, and screenshots to `<docs-dir>/screenshots`.

use crate::app::AppDelegate;
use crate::draw::{accent, attrs, attrs_wrapped, rect, text_centered, text_in};
use crate::editor::{EditorWindow, Pointer};
use crate::hud::{self, HudModel, HudView};
use crate::system::ns;
use crate::wheel::{WheelModel, WheelView};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSApplication, NSBezierPath,
    NSBitmapImageFileType, NSBitmapImageRep, NSCalibratedRGBColorSpace, NSColor, NSColorSpace,
    NSCompositingOperation, NSCursor, NSFont, NSFontWeightMedium, NSFontWeightSemibold, NSGradient,
    NSGradientDrawingOptions, NSGraphicsContext, NSImage, NSImageInterpolation, NSRectFillUsingOperation,
    NSShadow, NSStringDrawing, NSTextAlignment, NSView,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSPoint, NSRect, NSSize};
use satsuma_core::geometry::{WheelGeometry, WHEEL_VIEW_SIZE};
use satsuma_core::naming::{conversion_output_name, tool_output_name};
use satsuma_core::wheel::{hub_caption, wheel_items};
use satsuma_core::{Format, Tool, WheelItem, WheelMode};
use satsuma_engine::{capability, Env, Operation};
use std::f64::consts::PI;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

type R = Result<(), String>;

/// The scene in points; frames are rendered at 2x.
const SW: f64 = 1280.0;
const SH: f64 = 800.0;
const SCALE: f64 = 2.0;
const FPS: f64 = 30.0;
const ICON: f64 = 64.0;
const PHOTO: (f64, f64) = (1800.0, 1200.0);

pub fn run(args: &[String]) -> i32 {
    let (Some(work), Some(docs)) = (args.first(), args.get(1)) else {
        eprintln!("Usage: satsuma render-demo <work-dir> <docs-dir>");
        return 2;
    };
    let mtm = crate::system::main_thread();
    let app = NSApplication::sharedApplication(mtm);
    app.setAppearance(Some(&aqua()));
    let result = Director::new(mtm, Path::new(work), Path::new(docs)).and_then(|mut d| {
        d.play()?;
        d.finish()
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

// ---------------------------------------------------------------- drawing helpers

fn aqua() -> Retained<NSAppearance> {
    NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua }).expect("the Aqua appearance")
}

fn rgb(hex: u32, a: f64) -> Retained<NSColor> {
    let c = |shift: u32| ((hex >> shift) & 0xff) as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(c(16), c(8), c(0), a)
}

fn gradient(colors: &[u32]) -> Retained<NSGradient> {
    let colors: Vec<_> = colors.iter().map(|c| rgb(*c, 1.0)).collect();
    NSGradient::initWithColors(NSGradient::alloc(), &NSArray::from_retained_slice(&colors)).expect("a gradient")
}

fn disc(c: NSPoint, r: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithOvalInRect(rect(c.x - r, c.y - r, r * 2.0, r * 2.0))
}

fn rounded(r: NSRect, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, radius, radius)
}

fn zero() -> NSRect {
    rect(0.0, 0.0, 0.0, 0.0)
}

fn ease(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn lerp(a: NSPoint, b: NSPoint, t: f64) -> NSPoint {
    NSPoint::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// A transparent bitmap of `size` points at `scale`, tagged sRGB.
fn bitmap(size: NSSize, scale: f64) -> Result<Retained<NSBitmapImageRep>, String> {
    let (w, h) = ((size.width * scale).round() as isize, (size.height * scale).round() as isize);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            w,
            h,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            w * 4,
            32,
        )
    }
    .ok_or("Could not create a bitmap.")?;
    let rep = rep.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace()).ok_or("Could not tag the bitmap.")?;
    rep.setSize(size);
    Ok(rep)
}

/// Runs `draw` with `rep` as the current graphics context, after clearing it.
fn paint(rep: &NSBitmapImageRep, draw: impl FnOnce() -> R) -> R {
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(rep).ok_or("No graphics context.")?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    ctx.setImageInterpolation(NSImageInterpolation::High);
    NSColor::clearColor().set();
    NSRectFillUsingOperation(rect(0.0, 0.0, rep.size().width, rep.size().height), NSCompositingOperation::Copy);
    let result = draw();
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    result
}

fn write_png(rep: &NSBitmapImageRep, out: &Path) -> R {
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
        .ok_or("Could not encode the PNG.")?;
    std::fs::write(out, data.to_vec()).map_err(|e| format!("{}: {e}", out.display()))
}

fn snapshot(view: &NSView) -> Result<Retained<NSBitmapImageRep>, String> {
    let b = view.bounds();
    let rep = bitmap(b.size, SCALE)?;
    paint(&rep, || Ok(()))?;
    view.cacheDisplayInRect_toBitmapImageRep(b, &rep);
    Ok(rep)
}

fn draw_rep(rep: &NSBitmapImageRep, dst: NSRect, alpha: f64) {
    unsafe { rep.drawInRect_fromRect_operation_fraction_respectFlipped_hints(dst, zero(), NSCompositingOperation::SourceOver, alpha, true, None) };
}

fn draw_image(img: &NSImage, dst: NSRect, src: NSRect, alpha: f64) {
    img.drawInRect_fromRect_operation_fraction(dst, src, NSCompositingOperation::SourceOver, alpha);
}

/// Draws whatever `f` draws with a soft drop shadow.
fn shadowed(blur: f64, dy: f64, alpha: f64, f: impl FnOnce()) {
    NSGraphicsContext::saveGraphicsState_class();
    let s = NSShadow::new();
    s.setShadowColor(Some(&rgb(0x000000, alpha)));
    s.setShadowBlurRadius(blur);
    s.setShadowOffset(NSSize::new(0.0, -dy));
    s.set();
    f();
    NSGraphicsContext::restoreGraphicsState_class();
}

fn font(size: f64, weight: f64) -> Retained<NSFont> {
    NSFont::systemFontOfSize_weight(size, weight)
}

fn fit_in(size: NSSize, bounds: NSRect) -> NSRect {
    let s = (bounds.size.width / size.width).min(bounds.size.height / size.height);
    let (w, h) = (size.width * s, size.height * s);
    rect(bounds.origin.x + (bounds.size.width - w) / 2.0, bounds.origin.y + (bounds.size.height - h) / 2.0, w, h)
}

// ---------------------------------------------------------------- sample files

/// An invented sunset over the sea, drawn rather than photographed.
fn paint_photo() -> R {
    let (w, h) = PHOTO;
    let horizon = h * 0.36;
    gradient(&[0xF7B27A, 0xEE8D7E, 0x9C6FA8, 0x3B4A86, 0x1F2A5A]).drawInRect_angle(rect(0.0, horizon, w, h - horizon), 90.0);
    let sun = NSPoint::new(w * 0.6, horizon + h * 0.06);
    let glow = NSArray::from_retained_slice(&[rgb(0xFFDDA8, 0.75), rgb(0xFFB98A, 0.25), rgb(0xFFB98A, 0.0)]);
    if let Some(g) = NSGradient::initWithColors(NSGradient::alloc(), &glow) {
        g.drawFromCenter_radius_toCenter_radius_options(sun, 0.0, sun, w * 0.24, NSGradientDrawingOptions::empty());
    }
    rgb(0xFFF1D0, 1.0).setFill();
    disc(sun, w * 0.045).fill();
    gradient(&[0x141C38, 0x2A3766, 0x6B5F8E]).drawInRect_angle(rect(0.0, 0.0, w, horizon), 90.0);
    for k in 0..16 {
        let y = horizon - 12.0 - k as f64 * h * 0.02;
        let half = (w * (0.09 - k as f64 * 0.004)).max(w * 0.012);
        rgb(0xFFD3A0, (0.6 - k as f64 * 0.035).max(0.05)).setFill();
        rounded(rect(sun.x - half, y - 4.0, half * 2.0, 8.0), 4.0).fill();
    }
    let p = |x: f64, y: f64| NSPoint::new(x, y);
    let left = NSBezierPath::bezierPath();
    left.moveToPoint(p(0.0, horizon - 2.0));
    left.lineToPoint(p(0.0, horizon + h * 0.17));
    left.curveToPoint_controlPoint1_controlPoint2(p(w * 0.16, horizon + h * 0.05), p(w * 0.06, horizon + h * 0.17), p(w * 0.1, horizon + h * 0.07));
    left.curveToPoint_controlPoint1_controlPoint2(p(w * 0.34, horizon - 2.0), p(w * 0.22, horizon + h * 0.03), p(w * 0.28, horizon));
    left.closePath();
    rgb(0x241C3E, 1.0).setFill();
    left.fill();
    let right = NSBezierPath::bezierPath();
    right.moveToPoint(p(w, horizon - 2.0));
    right.lineToPoint(p(w, horizon + h * 0.1));
    right.curveToPoint_controlPoint1_controlPoint2(p(w * 0.8, horizon - 2.0), p(w * 0.92, horizon + h * 0.1), p(w * 0.86, horizon + h * 0.01));
    right.closePath();
    rgb(0x2E2450, 1.0).setFill();
    right.fill();
    rgb(0x1E1A33, 0.85).setStroke();
    for (x, y, s) in [(0.30, 0.80, 1.0), (0.345, 0.83, 0.8), (0.27, 0.845, 0.65)] {
        let (x, y) = (w * x, h * y);
        let bird = NSBezierPath::bezierPath();
        bird.moveToPoint(p(x - 22.0 * s, y + 6.0 * s));
        bird.curveToPoint_controlPoint1_controlPoint2(p(x, y), p(x - 14.0 * s, y + 10.0 * s), p(x - 5.0 * s, y + 6.0 * s));
        bird.curveToPoint_controlPoint1_controlPoint2(p(x + 22.0 * s, y + 6.0 * s), p(x + 5.0 * s, y + 6.0 * s), p(x + 14.0 * s, y + 10.0 * s));
        bird.setLineWidth(3.5 * s);
        bird.stroke();
    }
    Ok(())
}

fn write_samples(dir: &Path) -> R {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let rep = bitmap(NSSize::new(PHOTO.0, PHOTO.1), 1.0)?;
    paint(&rep, paint_photo)?;
    let jpeg = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &NSDictionary::new()) }
        .ok_or("Could not encode the JPEG.")?;
    std::fs::write(dir.join("Beach.jpg"), jpeg.to_vec()).map_err(|e| e.to_string())?;

    // Three seconds of a soft 220 Hz tone, 44.1 kHz mono.
    let rate = 44_100u32;
    let samples: Vec<i16> = (0..rate * 3)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let fade = (t * 4.0).min(1.0).min((3.0 - t) * 4.0);
            ((2.0 * PI * 220.0 * t).sin() * 6000.0 * fade) as i16
        })
        .collect();
    let mut wav = Vec::new();
    let data_len = samples.len() as u32 * 2;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | (1 << 16), rate, rate * 2, 2 | (16 << 16)] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(dir.join("Podcast.wav"), wav).map_err(|e| e.to_string())?;

    let text = "BT /F1 24 Tf 72 720 Td (Quarterly report) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
    for o in offsets {
        pdf.push_str(&format!("{o:010} 00000 n \n"));
    }
    pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
    std::fs::write(dir.join("Report.pdf"), pdf).map_err(|e| e.to_string())?;

    std::fs::write(dir.join("Notes.md"), "# Notes\n\n- Pack the camera\n- Book the ferry\n").map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- the scene

enum Look {
    /// A photo thumbnail; `src` is the part of the image shown.
    Thumb(Retained<NSImage>, NSRect),
    /// A drawn document icon (not the Mac's own, which depends on the apps installed).
    Doc(Retained<NSBitmapImageRep>),
}

/// A plain page with a coloured badge naming the file type.
fn doc_icon(ext: &str, badge: u32) -> Result<Retained<NSBitmapImageRep>, String> {
    let rep = bitmap(NSSize::new(ICON, ICON), SCALE)?;
    paint(&rep, || {
        let (x0, y0, w, h, fold) = (9.0, 3.0, 46.0, 58.0, 13.0);
        let page = NSBezierPath::bezierPath();
        page.moveToPoint(NSPoint::new(x0, y0));
        page.lineToPoint(NSPoint::new(x0 + w, y0));
        page.lineToPoint(NSPoint::new(x0 + w, y0 + h - fold));
        page.lineToPoint(NSPoint::new(x0 + w - fold, y0 + h));
        page.lineToPoint(NSPoint::new(x0, y0 + h));
        page.closePath();
        shadowed(2.0, 0.5, 0.3, || {
            rgb(0xFDFDFD, 1.0).setFill();
            page.fill();
        });
        rgb(0x000000, 0.18).setStroke();
        page.setLineWidth(0.5);
        page.stroke();
        let corner = NSBezierPath::bezierPath();
        corner.moveToPoint(NSPoint::new(x0 + w - fold, y0 + h));
        corner.lineToPoint(NSPoint::new(x0 + w - fold, y0 + h - fold));
        corner.lineToPoint(NSPoint::new(x0 + w, y0 + h - fold));
        corner.closePath();
        rgb(0xE2E2E6, 1.0).setFill();
        corner.fill();
        rgb(0xC9C9CF, 1.0).setFill();
        for (k, len) in [30.0, 26.0, 30.0, 18.0].into_iter().enumerate() {
            NSBezierPath::fillRect(rect(x0 + 8.0, y0 + h - 22.0 - k as f64 * 6.0, len, 2.0));
        }
        rgb(badge, 1.0).setFill();
        rounded(rect(x0 + 5.0, y0 + 7.0, w - 10.0, 14.0), 3.5).fill();
        let a = attrs(&font(9.0, unsafe { objc2_app_kit::NSFontWeightBold }), &rgb(0xFFFFFF, 1.0), None);
        text_centered(&ext.to_uppercase(), NSPoint::new(x0 + w / 2.0, y0 + 14.0), &a);
        Ok(())
    })?;
    Ok(rep)
}

struct DeskFile {
    name: String,
    look: Look,
    pos: NSPoint,
    born: Option<usize>,
}

struct Wheel {
    view: Retained<WheelView>,
    center: NSPoint,
    appear: f64,
}

struct Hud {
    view: Retained<HudView>,
    origin: NSPoint,
    alpha: f64,
}

struct Editor {
    win: EditorWindow,
    frame: NSRect,
    title: &'static str,
    appear: f64,
}

struct Drag {
    file: usize,
    /// Icon centre minus pointer.
    offset: NSPoint,
    /// Where the drag image is when it is not following the pointer (sliding back).
    at: Option<NSPoint>,
}

struct Director {
    mtm: MainThreadMarker,
    env: Env,
    delegate: Retained<AppDelegate>,
    samples: PathBuf,
    shots: PathBuf,
    photo: Retained<NSImage>,
    wallpaper: Retained<NSBitmapImageRep>,
    frame_rep: Retained<NSBitmapImageRep>,
    encoder: Child,
    frame: usize,
    files: Vec<DeskFile>,
    cursor: NSPoint,
    show_cursor: bool,
    selected: Option<usize>,
    drag: Option<Drag>,
    wheel: Option<Wheel>,
    hud: Option<Hud>,
    editor: Option<Editor>,
    crop_drag: bool,
    key: Option<(&'static str, &'static str, usize)>,
    caption: Option<(&'static str, usize)>,
    ripples: Vec<(NSPoint, usize)>,
}

impl Director {
    fn new(mtm: MainThreadMarker, work: &Path, docs: &Path) -> Result<Self, String> {
        let samples = work.join("Desktop");
        write_samples(&samples)?;
        let shots = docs.join("screenshots");
        std::fs::create_dir_all(&shots).map_err(|e| e.to_string())?;
        let photo = NSImage::initWithContentsOfFile(NSImage::alloc(), &ns(&samples.join("Beach.jpg").to_string_lossy()))
            .ok_or("Could not load the sample photo.")?;
        let wallpaper = bitmap(NSSize::new(SW, SH), SCALE)?;
        paint(&wallpaper, || {
            paint_wallpaper();
            Ok(())
        })?;
        let master = work.join("master.mkv");
        let encoder = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{}x{}", (SW * SCALE) as i64, (SH * SCALE) as i64), "-framerate", &FPS.to_string()])
            .args(["-i", "-", "-c:v", "libx264rgb", "-preset", "ultrafast", "-crf", "0"])
            .arg(&master)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't start ffmpeg: {e}"))?;

        let full = rect(0.0, 0.0, PHOTO.0, PHOTO.1);
        let column = |row: usize| NSPoint::new(SW - 90.0, SH - 110.0 - row as f64 * 120.0);
        let files = vec![
            DeskFile { name: "Beach.jpg".into(), look: Look::Thumb(photo.clone(), full), pos: column(0), born: None },
            DeskFile { name: "Podcast.wav".into(), look: Look::Doc(doc_icon("wav", 0x8E5BD9)?), pos: column(1), born: None },
            DeskFile { name: "Report.pdf".into(), look: Look::Doc(doc_icon("pdf", 0xE0483B)?), pos: column(2), born: None },
            DeskFile { name: "Notes.md".into(), look: Look::Doc(doc_icon("md", 0x4A6FA5)?), pos: column(3), born: None },
        ];
        Ok(Director {
            mtm,
            env: Env::discover(),
            delegate: AppDelegate::new(mtm),
            samples,
            shots,
            photo,
            wallpaper,
            frame_rep: bitmap(NSSize::new(SW, SH), SCALE)?,
            encoder,
            frame: 0,
            files,
            cursor: NSPoint::new(520.0, 250.0),
            show_cursor: true,
            selected: None,
            drag: None,
            wheel: None,
            hud: None,
            editor: None,
            crop_drag: false,
            key: None,
            caption: None,
            ripples: Vec::new(),
        })
    }

    fn age(&self, born: usize) -> f64 {
        (self.frame.saturating_sub(born)) as f64 / FPS
    }

    // ------------------------------------------------------------ rendering

    fn draw(&self) -> R {
        draw_rep(&self.wallpaper, rect(0.0, 0.0, SW, SH), 1.0);

        for (i, f) in self.files.iter().enumerate() {
            let (alpha, scale) = match f.born {
                Some(b) => {
                    let t = (self.age(b) / 0.3).min(1.0);
                    (t, 0.6 + 0.4 * (1.0 - (1.0 - t).powi(3)) + 0.06 * (PI * t).sin())
                }
                None => (1.0, 1.0),
            };
            let dragging = self.drag.as_ref().is_some_and(|d| d.file == i);
            self.draw_file(f, f.pos, if dragging { 0.45 * alpha } else { alpha }, scale, self.selected == Some(i));
        }

        if let Some(e) = &self.editor {
            self.draw_editor(e)?;
        }

        if let Some(w) = &self.wheel {
            let g = WheelGeometry::default();
            let s = 0.86 + 0.14 * w.appear;
            shadowed(26.0, 10.0, 0.35 * w.appear, || {
                rgb(0xF2F2F6, 0.74 * w.appear).setFill();
                disc(w.center, g.backdrop * s).fill();
            });
            let size = WHEEL_VIEW_SIZE * s;
            draw_rep(&*snapshot(&w.view)?, rect(w.center.x - size / 2.0, w.center.y - size / 2.0, size, size), w.appear);
        }

        if let Some(h) = &self.hud {
            let r = rect(h.origin.x, h.origin.y, hud::WIDTH, hud::HEIGHT);
            shadowed(22.0, 8.0, 0.3 * h.alpha, || {
                rgb(0xF5F5F7, 0.96 * h.alpha).setFill();
                rounded(r, 14.0).fill();
            });
            draw_rep(&*snapshot(&h.view)?, r, h.alpha);
        }

        if let Some(d) = &self.drag {
            let f = &self.files[d.file];
            let at = d.at.unwrap_or(NSPoint::new(self.cursor.x + d.offset.x, self.cursor.y + d.offset.y));
            self.draw_file(f, at, 0.7, 1.0, false);
        }

        for (p, born) in &self.ripples {
            let t = self.age(*born) / 0.4;
            if t < 1.0 {
                let ring = disc(*p, 8.0 + 22.0 * t);
                accent().colorWithAlphaComponent(0.7 * (1.0 - t)).setStroke();
                ring.setLineWidth(3.0);
                ring.stroke();
            }
        }

        if self.show_cursor {
            let cursor = NSCursor::arrowCursor();
            let img = cursor.image();
            let (hot, s) = (cursor.hotSpot(), img.size());
            let k = 1.2;
            img.drawInRect(rect(self.cursor.x - hot.x * k, self.cursor.y - (s.height - hot.y) * k, s.width * k, s.height * k));
        }

        if let Some((glyph, label, born)) = self.key {
            let t = self.age(born);
            let alpha = (t / 0.1).min(1.0) * (1.0 - ((t - 1.1) / 0.3).clamp(0.0, 1.0));
            if alpha > 0.0 {
                let r = rect(SW / 2.0 - 110.0, 34.0, 220.0, 66.0);
                shadowed(18.0, 5.0, 0.3 * alpha, || {
                    rgb(0x1C1C1E, 0.84 * alpha).setFill();
                    rounded(r, 16.0).fill();
                });
                let white = rgb(0xFFFFFF, alpha);
                text_centered(glyph, NSPoint::new(r.origin.x + 56.0, r.origin.y + 33.0), &attrs(&font(30.0, unsafe { NSFontWeightSemibold }), &white, None));
                text_centered(label, NSPoint::new(r.origin.x + 136.0, r.origin.y + 33.0), &attrs(&font(21.0, unsafe { NSFontWeightMedium }), &white, None));
            }
        }

        if let Some((text, born)) = self.caption {
            let alpha = (self.age(born) / 0.25).min(1.0);
            let a = attrs(&font(20.0, unsafe { NSFontWeightSemibold }), &rgb(0x1D1D1F, alpha), None);
            let size = unsafe { ns(text).sizeWithAttributes(Some(&a)) };
            let r = rect(SW / 2.0 - size.width / 2.0 - 24.0, SH - 96.0, size.width + 48.0, 46.0);
            shadowed(18.0, 5.0, 0.25 * alpha, || {
                rgb(0xFFFFFF, 0.93 * alpha).setFill();
                rounded(r, 23.0).fill();
            });
            text_centered(text, NSPoint::new(SW / 2.0, r.origin.y + 23.0), &a);
        }
        Ok(())
    }

    fn draw_file(&self, f: &DeskFile, c: NSPoint, alpha: f64, scale: f64, selected: bool) {
        let size = ICON * scale;
        let frame = rect(c.x - size / 2.0, c.y - size / 2.0, size, size);
        if selected {
            rgb(0x000000, 0.28 * alpha).setFill();
            rounded(rect(frame.origin.x - 6.0, frame.origin.y - 6.0, size + 12.0, size + 12.0), 7.0).fill();
        }
        match &f.look {
            Look::Thumb(img, src) => {
                let r = fit_in(src.size, rect(frame.origin.x + 3.0, frame.origin.y + 3.0, size - 6.0, size - 6.0));
                shadowed(4.0, 1.5, 0.4 * alpha, || {
                    rgb(0xFFFFFF, alpha).setFill();
                    NSBezierPath::fillRect(rect(r.origin.x - 2.0, r.origin.y - 2.0, r.size.width + 4.0, r.size.height + 4.0));
                });
                draw_image(img, r, *src, alpha);
            }
            Look::Doc(rep) => draw_rep(rep, frame, alpha),
        }
        let label = rect(c.x - 64.0, frame.origin.y - 38.0, 128.0, 34.0);
        let a = attrs_wrapped(&font(12.0, unsafe { NSFontWeightMedium }), &rgb(0xFFFFFF, alpha), Some(NSTextAlignment::Center));
        shadowed(2.5, 1.0, 0.7 * alpha, || text_in(&f.name, label, &a));
    }

    fn draw_editor(&self, e: &Editor) -> R {
        let content = e.win.window.contentView().ok_or("No content view.")?;
        let (w, h) = (e.frame.size.width, e.frame.size.height);
        let win = bitmap(e.frame.size, SCALE)?;
        paint(&win, || {
            let b = rect(0.0, 0.0, w, h);
            rounded(b, 12.0).addClip();
            rgb(0xEFEFF1, 1.0).setFill();
            NSBezierPath::fillRect(b);
            for sub in content.subviews().iter() {
                if !sub.isHidden() {
                    draw_rep(&*snapshot(&sub)?, sub.frame(), 1.0);
                }
            }
            let a = attrs(&font(13.0, unsafe { NSFontWeightSemibold }), &rgb(0x3A3A3C, 1.0), None);
            text_centered(e.title, NSPoint::new(w / 2.0, h - 15.0), &a);
            for (k, color) in [0xFF5F57u32, 0xD6D6D9, 0xD6D6D9].into_iter().enumerate() {
                let light = disc(NSPoint::new(20.0 + k as f64 * 20.0, h - 15.0), 6.0);
                rgb(color, 1.0).setFill();
                light.fill();
                rgb(0x000000, 0.12).setStroke();
                light.setLineWidth(0.5);
                light.stroke();
            }
            rgb(0x000000, 0.16).setStroke();
            let border = rounded(rect(0.25, 0.25, w - 0.5, h - 0.5), 12.0);
            border.setLineWidth(0.5);
            border.stroke();
            Ok(())
        })?;
        let s = 0.96 + 0.04 * e.appear;
        let (dw, dh) = (w * s, h * s);
        let dst = rect(e.frame.origin.x + (w - dw) / 2.0, e.frame.origin.y + (h - dh) / 2.0, dw, dh);
        shadowed(44.0, 20.0, 0.45 * e.appear, || draw_rep(&win, dst, e.appear));
        Ok(())
    }

    /// Renders the current state into the frame bitmap without recording it.
    fn render(&self) -> R {
        paint(&self.frame_rep, || self.draw())
    }

    fn emit(&mut self) -> R {
        self.update_wheel_selection();
        self.render()?;
        let len = (self.frame_rep.bytesPerRow() * self.frame_rep.pixelsHigh()) as usize;
        let bytes = unsafe { std::slice::from_raw_parts(self.frame_rep.bitmapData(), len) };
        self.encoder
            .stdin
            .as_mut()
            .ok_or("ffmpeg has no input.")?
            .write_all(bytes)
            .map_err(|e| format!("Couldn't write a frame to ffmpeg: {e}"))?;
        self.frame += 1;
        self.ripples.retain(|(_, born)| (self.frame - born) as f64 / FPS < 0.5);
        Ok(())
    }

    /// Saves the last rendered frame, cropped to `region` (points), as a screenshot.
    fn shot(&mut self, name: &str, region: NSRect) -> R {
        let (caption, key) = (self.caption.take(), self.key.take());
        let rendered = self.render();
        (self.caption, self.key) = (caption, key);
        rendered?;
        let rep = bitmap(region.size, SCALE)?;
        paint(&rep, || {
            draw_rep(&self.frame_rep, rect(-region.origin.x, -region.origin.y, SW, SH), 1.0);
            Ok(())
        })?;
        write_png(&rep, &self.shots.join(format!("{name}.png")))
    }

    fn finish(mut self) -> R {
        drop(self.encoder.stdin.take());
        let status = self.encoder.wait().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg failed: {status}"))
        }
    }

    // ------------------------------------------------------------ timeline primitives

    fn hold(&mut self, secs: f64) -> R {
        for _ in 0..frames(secs) {
            self.emit()?;
        }
        Ok(())
    }

    fn tween(&mut self, secs: f64, mut f: impl FnMut(&mut Self, f64)) -> R {
        let n = frames(secs);
        for i in 1..=n {
            f(self, i as f64 / n as f64);
            self.emit()?;
        }
        Ok(())
    }

    /// Moves the pointer along a slight arc, as a hand would.
    fn move_to(&mut self, to: NSPoint, secs: f64) -> R {
        let from = self.cursor;
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let d = dx.hypot(dy);
        let bend = (d * 0.08).min(36.0);
        let (nx, ny) = if d > 0.0 { (-dy / d, dx / d) } else { (0.0, 0.0) };
        self.tween(secs, |s, t| {
            let e = ease(t);
            let arc = (PI * e).sin() * bend;
            s.cursor = NSPoint::new(from.x + dx * e + nx * arc, from.y + dy * e + ny * arc);
            if s.crop_drag {
                s.crop_input(Pointer::Drag);
            }
        })
    }

    fn click(&mut self) {
        self.ripples.push((self.cursor, self.frame));
    }

    fn caption(&mut self, text: &'static str) {
        self.caption = Some((text, self.frame));
    }

    fn tap(&mut self, glyph: &'static str, label: &'static str) {
        self.key = Some((glyph, label, self.frame));
    }

    // ------------------------------------------------------------ desktop and drags

    fn file_index(&self, name: &str) -> usize {
        self.files.iter().position(|f| f.name == name).expect("a sample file")
    }

    /// Points at a file, presses and starts dragging it to `to`.
    fn drag_file(&mut self, name: &str, to: NSPoint) -> R {
        let i = self.file_index(name);
        let pos = self.files[i].pos;
        self.move_to(NSPoint::new(pos.x + 20.0, pos.y - 20.0), 0.85)?;
        self.click();
        self.selected = Some(i);
        self.hold(0.15)?;
        self.drag = Some(Drag { file: i, offset: NSPoint::new(pos.x - self.cursor.x, pos.y - self.cursor.y), at: None });
        self.move_to(to, 1.0)
    }

    fn wheel_model(&self, mode: WheelMode, file: &str) -> WheelModel {
        let ext = Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or("");
        let formats = vec![Format::from_id(ext)];
        let items = wheel_items(mode, &formats, &|item| capability::item_available(&self.env, item, &formats));
        let bytes = std::fs::metadata(self.samples.join(file)).ok().map(|m| m.len());
        let mut model = WheelModel::new(mode, items);
        model.hub = hub_caption(1, bytes);
        model
    }

    fn open_wheel(&mut self, mode: WheelMode, file: &str) -> R {
        let view = WheelView::new(self.mtm, self.wheel_model(mode, file));
        view.setAppearance(Some(&aqua()));
        self.wheel = Some(Wheel { view, center: self.cursor, appear: 0.0 });
        self.tween(0.16, |s, t| {
            if let Some(w) = s.wheel.as_mut() {
                w.appear = ease(t);
            }
        })
    }

    /// Replaces the petals in place, as tapping another key does.
    fn switch_wheel(&mut self, mode: WheelMode, file: &str) {
        let model = self.wheel_model(mode, file);
        if let Some(w) = &self.wheel {
            w.view.set_model(model);
        }
    }

    fn update_wheel_selection(&mut self) {
        let Some(w) = &self.wheel else { return };
        if w.appear < 1.0 {
            return;
        }
        let mut m = w.view.model();
        let hit = WheelGeometry::default().hit_test(self.cursor.x - w.center.x, self.cursor.y - w.center.y, m.items.len());
        if m.selected != hit {
            m.selected = hit;
            w.view.set_model(m);
        }
    }

    /// Where the pointer should go to land on the petal titled `title`.
    fn petal(&self, title: &str) -> NSPoint {
        let w = self.wheel.as_ref().expect("an open wheel");
        let items = w.view.model().items;
        let i = items.iter().position(|it| it.title() == title).unwrap_or_else(|| panic!("no {title} petal"));
        let lp = WheelGeometry::default().label_position(i, items.len());
        NSPoint::new(w.center.x + lp.x * 1.15, w.center.y + lp.y * 1.15)
    }

    fn close_wheel(&mut self) -> R {
        self.tween(0.14, |s, t| {
            if let Some(w) = s.wheel.as_mut() {
                w.appear = 1.0 - t;
            }
        })?;
        self.wheel = None;
        Ok(())
    }

    /// Releases over the highlighted petal.
    fn drop_on_wheel(&mut self) -> Result<Option<WheelItem>, String> {
        let item = self.wheel.as_ref().and_then(|w| {
            let m = w.view.model();
            m.selected.and_then(|i| m.items.get(i).copied())
        });
        self.drag = None;
        self.selected = None;
        self.close_wheel()?;
        Ok(item)
    }

    /// Releases in the centre: the wheel closes and the drag image slides home.
    fn cancel_drag(&mut self) -> R {
        self.close_wheel()?;
        let Some(d) = &self.drag else { return Ok(()) };
        let from = NSPoint::new(self.cursor.x + d.offset.x, self.cursor.y + d.offset.y);
        let home = self.files[d.file].pos;
        self.tween(0.3, |s, t| {
            if let Some(d) = s.drag.as_mut() {
                d.at = Some(lerp(from, home, ease(t)));
            }
        })?;
        self.drag = None;
        self.selected = None;
        Ok(())
    }

    // ------------------------------------------------------------ progress

    /// The progress HUD for one file: bar, then "Saved …", and the result appearing.
    fn job(&mut self, title: String, near: NSPoint, result: DeskFile) -> R {
        let view = HudView::new(self.mtm);
        view.setAppearance(Some(&aqua()));
        let model = |subtitle: String, fraction: f64| HudModel { title: title.clone(), subtitle, fraction: Some(fraction), error: false };
        *view.ivars().borrow_mut() = model("Preparing…".into(), 0.0);
        self.hud = Some(Hud { view, origin: NSPoint::new(near.x - hud::WIDTH / 2.0, near.y - 150.0 - hud::HEIGHT), alpha: 0.0 });
        self.tween(0.12, |s, t| {
            if let Some(h) = s.hud.as_mut() {
                h.alpha = t;
            }
        })?;
        self.tween(0.9, |s, t| {
            if let Some(h) = &s.hud {
                *h.view.ivars().borrow_mut() = model("0 of 1 saved".into(), ease(t) * 0.98);
            }
        })?;
        if let Some(h) = &self.hud {
            *h.view.ivars().borrow_mut() = model(format!("Saved {}", result.name), 1.0);
        }
        self.files.push(DeskFile { born: Some(self.frame), ..result });
        Ok(())
    }

    fn hide_hud(&mut self) -> R {
        self.tween(0.2, |s, t| {
            if let Some(h) = s.hud.as_mut() {
                h.alpha = 1.0 - t;
            }
        })?;
        self.hud = None;
        Ok(())
    }

    // ------------------------------------------------------------ editors

    fn open_editor(&mut self, kind: &str, file: &str, animate: bool) -> R {
        let files = vec![self.samples.join(file)];
        let d = ProtocolObject::from_ref(&*self.delegate);
        let (win, title) = match kind {
            "crop" => (EditorWindow::crop(self.mtm, files, d, false)?, "Crop Image"),
            "edit" => (EditorWindow::adjust(self.mtm, files, d, false)?, "Edit Photo"),
            _ => (EditorWindow::frame(self.mtm, files, d, false)?, "Add Background"),
        };
        let size = win.window.contentView().ok_or("No content view.")?.bounds().size;
        let frame = rect(((SW - size.width) / 2.0).round(), ((SH - size.height) / 2.0 - 24.0).max(16.0).round(), size.width, size.height);
        self.editor = Some(Editor { win, frame, title, appear: if animate { 0.0 } else { 1.0 } });
        if animate {
            self.tween(0.22, |s, t| {
                if let Some(e) = s.editor.as_mut() {
                    e.appear = ease(t);
                }
            })?;
        }
        Ok(())
    }

    fn close_editor(&mut self) -> R {
        self.tween(0.18, |s, t| {
            if let Some(e) = s.editor.as_mut() {
                e.appear = 1.0 - t;
            }
        })?;
        self.editor = None;
        Ok(())
    }

    /// Converts a window point of the editor to the scene.
    fn in_editor(&self, p: NSPoint) -> NSPoint {
        let f = self.editor.as_ref().expect("an editor").frame;
        NSPoint::new(f.origin.x + p.x, f.origin.y + p.y)
    }

    fn crop_frame(&self) -> NSRect {
        let e = self.editor.as_ref().expect("an editor");
        let r = e.win.crop_frame().expect("a crop editor");
        rect(e.frame.origin.x + r.origin.x, e.frame.origin.y + r.origin.y, r.size.width, r.size.height)
    }

    fn crop_input(&mut self, phase: Pointer) {
        if let Some(e) = &self.editor {
            let p = NSPoint::new(self.cursor.x - e.frame.origin.x, self.cursor.y - e.frame.origin.y);
            e.win.crop_pointer(p, phase);
        }
    }

    /// Presses at the pointer, drags by (dx, dy) and releases.
    fn crop_drag(&mut self, dx: f64, dy: f64, secs: f64) -> R {
        self.click();
        self.crop_input(Pointer::Down);
        self.crop_drag = true;
        self.hold(0.1)?;
        let to = NSPoint::new(self.cursor.x + dx, self.cursor.y + dy);
        self.move_to(to, secs)?;
        self.crop_drag = false;
        self.crop_input(Pointer::Up);
        self.hold(0.15)
    }

    // ------------------------------------------------------------ the story

    fn play(&mut self) -> R {
        // Convert: drag, tap Shift, drop on a petal.
        self.hold(0.5)?;
        self.caption("Drag a file");
        self.drag_file("Beach.jpg", NSPoint::new(560.0, 400.0))?;
        self.caption("Tap Shift to convert");
        self.tap("⇧", "Shift");
        self.hold(0.25)?;
        self.open_wheel(WheelMode::Conversions, "Beach.jpg")?;
        self.hold(0.45)?;
        self.caption("Drop on a format");
        let png = self.petal("PNG");
        self.move_to(png, 0.6)?;
        self.hold(0.5)?;
        let wheel_center = self.wheel.as_ref().map(|w| w.center).unwrap_or(self.cursor);
        self.shot("convert", rect(wheel_center.x - 260.0, wheel_center.y - 205.0, 520.0, 410.0))?;
        self.drop_on_wheel()?;
        let name = conversion_output_name("Beach.jpg", Format::Png, &|_| false);
        let thumb = Look::Thumb(self.photo.clone(), rect(0.0, 0.0, PHOTO.0, PHOTO.1));
        let at = NSPoint::new(SW - 210.0, self.files[0].pos.y);
        self.job(Operation::Convert(Format::Png).title(), wheel_center, DeskFile { name, look: thumb, pos: at, born: None })?;
        self.caption("Saved next to the original");
        self.hold(1.3)?;
        self.hide_hud()?;

        // The same gesture for audio.
        self.caption("Audio, video, documents and more");
        self.drag_file("Podcast.wav", NSPoint::new(600.0, 420.0))?;
        self.tap("⇧", "Shift");
        self.hold(0.2)?;
        self.open_wheel(WheelMode::Conversions, "Podcast.wav")?;
        self.hold(0.35)?;
        let mp3 = self.petal("MP3");
        self.move_to(mp3, 0.55)?;
        self.hold(0.35)?;
        let wheel_center = self.wheel.as_ref().map(|w| w.center).unwrap_or(self.cursor);
        self.drop_on_wheel()?;
        let name = conversion_output_name("Podcast.wav", Format::Mp3, &|_| false);
        let at = NSPoint::new(SW - 210.0, self.files[1].pos.y);
        let look = Look::Doc(doc_icon("mp3", 0xD9468F)?);
        self.job(Operation::Convert(Format::Mp3).title(), wheel_center, DeskFile { name, look, pos: at, born: None })?;
        self.hold(1.1)?;
        self.hide_hud()?;

        // Tools: Command, then the Crop editor.
        self.caption("Tap Command for tools");
        self.drag_file("Beach.jpg", NSPoint::new(560.0, 430.0))?;
        self.tap("⌘", "Command");
        self.hold(0.25)?;
        self.open_wheel(WheelMode::Tools, "Beach.jpg")?;
        self.hold(0.6)?;
        let crop = self.petal("CROP");
        self.move_to(crop, 0.6)?;
        self.hold(0.45)?;
        let wheel_center = self.wheel.as_ref().map(|w| w.center).unwrap_or(self.cursor);
        self.shot("tools", rect(wheel_center.x - 260.0, wheel_center.y - 205.0, 520.0, 410.0))?;
        self.drop_on_wheel()?;
        self.caption("Crop with handles or a preset");
        self.open_editor("crop", "Beach.jpg", true)?;
        self.hold(0.3)?;
        let r = self.crop_frame();
        self.move_to(NSPoint::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height), 0.6)?;
        self.crop_drag(0.0, -70.0, 0.7)?;
        let r = self.crop_frame();
        self.move_to(NSPoint::new(r.origin.x + r.size.width, r.origin.y), 0.55)?;
        self.crop_drag(-70.0, 40.0, 0.6)?;
        // The "16:9" segment of the aspect control.
        self.move_to(self.in_editor(NSPoint::new(108.0 + 336.0 / 6.0 * 2.5, 109.0)), 0.6)?;
        self.click();
        if let Some(e) = &self.editor {
            e.win.select_segment(2);
        }
        self.hold(0.7)?;
        if let Some(e) = &self.editor {
            self.shot("crop", rect(e.frame.origin.x - 40.0, e.frame.origin.y - 40.0, e.frame.size.width + 80.0, e.frame.size.height + 80.0))?;
        }
        let src = self.editor.as_ref().and_then(|e| e.win.crop_rect()).map(|c| rect(c.x, PHOTO.1 - c.y - c.h, c.w, c.h));
        self.move_to(self.in_editor(NSPoint::new(460.0 - 16.0 - 55.0, 29.0)), 0.6)?;
        self.click();
        self.hold(0.12)?;
        let center = self.editor.as_ref().map(|e| NSPoint::new(e.frame.origin.x + e.frame.size.width / 2.0, e.frame.origin.y + e.frame.size.height / 2.0));
        self.close_editor()?;
        let name = tool_output_name("Beach.jpg", Tool::CropImage.info().label, Some(Format::Jpg), &|_| false);
        let thumb = Look::Thumb(self.photo.clone(), src.unwrap_or(rect(0.0, 0.0, PHOTO.0, PHOTO.1)));
        let at = NSPoint::new(SW - 330.0, self.files[0].pos.y);
        self.job(Tool::CropImage.info().verb.to_string(), center.unwrap_or(self.cursor), DeskFile { name, look: thumb, pos: at, born: None })?;
        self.hold(1.2)?;
        self.hide_hud()?;

        // Switching wheels, then cancelling in the centre.
        self.caption("Tap another key to switch wheels");
        self.drag_file("Report.pdf", NSPoint::new(600.0, 420.0))?;
        self.tap("⇧", "Shift");
        self.hold(0.2)?;
        self.open_wheel(WheelMode::Conversions, "Report.pdf")?;
        self.hold(0.8)?;
        self.tap("⌘", "Command");
        self.switch_wheel(WheelMode::Tools, "Report.pdf");
        self.hold(0.9)?;
        self.tap("⌃", "Control");
        self.switch_wheel(WheelMode::Actions, "Report.pdf");
        self.hold(0.9)?;
        let wheel_center = self.wheel.as_ref().map(|w| w.center).unwrap_or(self.cursor);
        let copy_path = self.petal("COPY PATH");
        self.move_to(copy_path, 0.5)?;
        self.hold(0.4)?;
        self.shot("actions", rect(wheel_center.x - 260.0, wheel_center.y - 205.0, 520.0, 410.0))?;
        self.caption("Drop in the centre to cancel");
        self.move_to(wheel_center, 0.5)?;
        self.hold(0.4)?;
        self.cancel_drag()?;

        self.caption("Everything runs on your Mac");
        self.move_to(NSPoint::new(700.0, 300.0), 0.8)?;
        self.hold(1.6)?;

        // Still screenshots of the other two editors.
        self.caption = None;
        self.show_cursor = false;
        self.open_editor("edit", "Beach.jpg", false)?;
        if let Some(e) = &self.editor {
            for (k, v) in [("exposure", 10.0), ("contrast", 15.0), ("temperature", 20.0), ("vibrance", 30.0)] {
                e.win.set_adjustment(k, v);
            }
        }
        self.editor_shot("edit")?;
        self.open_editor("frame", "Beach.jpg", false)?;
        if let Some(e) = &self.editor {
            e.win.select_segment(1);
        }
        self.editor_shot("background")
    }

    /// An editor window alone on the wallpaper, on a canvas sized for it.
    fn editor_shot(&mut self, name: &str) -> R {
        let e = self.editor.as_mut().ok_or("No editor.")?;
        let size = e.frame.size;
        e.frame = rect(40.0, 40.0, size.width, size.height);
        let canvas = NSSize::new(size.width + 80.0, size.height + 80.0);
        let rep = bitmap(canvas, SCALE)?;
        let e = self.editor.as_ref().ok_or("No editor.")?;
        paint(&rep, || {
            let s = (canvas.width / SW).max(canvas.height / SH);
            draw_rep(&self.wallpaper, rect((canvas.width - SW * s) / 2.0, (canvas.height - SH * s) / 2.0, SW * s, SH * s), 1.0);
            self.draw_editor(e)
        })?;
        write_png(&rep, &self.shots.join(format!("{name}.png")))
    }
}

fn frames(secs: f64) -> usize {
    (secs * FPS).round().max(1.0) as usize
}

fn paint_wallpaper() {
    let full = rect(0.0, 0.0, SW, SH);
    gradient(&[0x1E2650, 0x41397A, 0x8E5487, 0xD9796C]).drawInRect_angle(full, -38.0);
    for (x, y, r, color, a) in [(1100.0, 80.0, 520.0, 0xFFA36E, 0.38), (160.0, 720.0, 560.0, 0x5D7FE0, 0.3)] {
        let colors = NSArray::from_retained_slice(&[rgb(color, a), rgb(color, 0.0)]);
        if let Some(g) = NSGradient::initWithColors(NSGradient::alloc(), &colors) {
            let c = NSPoint::new(x, y);
            g.drawFromCenter_radius_toCenter_radius_options(c, 0.0, c, r, NSGradientDrawingOptions::empty());
        }
    }
}
