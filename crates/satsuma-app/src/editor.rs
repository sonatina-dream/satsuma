//! Tool editor windows: Crop Image and Add Background (SPEC §6.1, §6.2).
//!
//! Each window keeps its state in an `EditorTarget` object, which is also the target of
//! every control. "Apply" posts a [`UiEvent::EditorApply`] and closes the window; the
//! controller then runs the job like any other.

use crate::draw::{accent, rect};
use crate::events::{post, UiEvent};
use crate::system::{self, ns};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSBezierPath, NSButton, NSButtonType, NSColor, NSControlStateValueOn, NSColorSpace, NSColorWell,
    NSColorWellStyle, NSEvent, NSFont, NSFontWeightRegular, NSImage, NSSegmentSwitchTracking,
    NSSegmentedControl, NSSlider, NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindingRule, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use satsuma_core::edit::{
    drag, fit_aspect, hit_handle, ratio_value, set_size, Adjustments, Background, CropSpec, FrameSpec, Handle,
    Rect, Section, ADJUSTMENTS, CROP_ASPECTS, FRAME_RATIOS, GRADIENTS, SOLIDS,
};
use satsuma_engine::imaging::{self, Image};
use satsuma_engine::photo;
use satsuma_engine::Operation;
use std::cell::RefCell;
use std::path::PathBuf;

const W: f64 = 460.0;

fn nsimage(img: &Image) -> Retained<NSImage> {
    let (w, h) = imaging::size(img);
    NSImage::initWithCGImage_size(NSImage::alloc(), img, NSSize::new(w, h))
}

/// Aspect-fit `size` inside `bounds`.
fn fit(size: (f64, f64), bounds: NSRect) -> NSRect {
    let s = (bounds.size.width / size.0).min(bounds.size.height / size.1);
    let (w, h) = (size.0 * s, size.1 * s);
    rect(bounds.origin.x + (bounds.size.width - w) / 2.0, bounds.origin.y + (bounds.size.height - h) / 2.0, w, h)
}

// ------------------------------------------------------------------ crop preview

/// How far from an edge (view points) a press still grabs its handle. The image is inset by
/// this much so every handle's grab zone stays inside the view.
const GRAB: f64 = 12.0;

pub struct CropModel {
    image: Retained<NSImage>,
    size: (f64, f64),
    rect: Rect,
    aspect: Option<f64>,
    drag: Option<(Handle, Rect, NSPoint)>,
    on_change: Option<Box<dyn Fn(Rect)>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaCropView"]
    #[ivars = RefCell<Option<CropModel>>]
    pub struct CropView;

    impl CropView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        // The window is movable by its background; handle drags must not move it.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            self.begin_drag(p);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            self.continue_drag(p);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            self.end_drag();
        }
    }

    unsafe impl NSObjectProtocol for CropView {}
);

impl CropView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn image_frame(&self, size: (f64, f64)) -> NSRect {
        let b = self.bounds();
        fit(size, rect(b.origin.x + GRAB, b.origin.y + GRAB, b.size.width - 2.0 * GRAB, b.size.height - 2.0 * GRAB))
    }

    fn scale(&self, size: (f64, f64)) -> f64 {
        self.image_frame(size).size.width / size.0
    }

    fn to_view(&self, size: (f64, f64), r: Rect) -> NSRect {
        let f = self.image_frame(size);
        let s = f.size.width / size.0;
        rect(f.origin.x + r.x * s, f.origin.y + f.size.height - (r.y + r.h) * s, r.w * s, r.h * s)
    }

    fn to_image(&self, size: (f64, f64), p: NSPoint) -> (f64, f64) {
        let f = self.image_frame(size);
        let s = f.size.width / size.0;
        ((p.x - f.origin.x) / s, (f.origin.y + f.size.height - p.y) / s)
    }

    fn set_rect(&self, r: Rect, notify: bool) {
        let callback = {
            let mut guard = self.ivars().borrow_mut();
            let Some(m) = guard.as_mut() else { return };
            m.rect = r;
            if notify { m.on_change.take() } else { None }
        };
        if let Some(cb) = callback {
            cb(r);
            if let Some(m) = self.ivars().borrow_mut().as_mut() {
                m.on_change = Some(cb);
            }
        }
        self.setNeedsDisplay(true);
    }

    fn begin_drag(&self, p: NSPoint) {
        let mut guard = self.ivars().borrow_mut();
        let Some(m) = guard.as_mut() else { return };
        let q = self.to_image(m.size, p);
        let tolerance = GRAB / self.scale(m.size);
        m.drag = hit_handle(m.rect, q, tolerance).map(|h| (h, m.rect, p));
    }

    fn continue_drag(&self, p: NSPoint) {
        let next = {
            let guard = self.ivars().borrow();
            let Some(m) = guard.as_ref() else { return };
            let Some((handle, start, p0)) = m.drag else { return };
            let s = self.scale(m.size);
            let (dx, dy) = ((p.x - p0.x) / s, -(p.y - p0.y) / s);
            drag(start, handle, dx, dy, m.aspect, m.size, 16.0)
        };
        self.set_rect(next, true);
    }

    fn end_drag(&self) {
        if let Some(m) = self.ivars().borrow_mut().as_mut() {
            m.drag = None;
        }
        self.setNeedsDisplay(true);
    }

    fn draw(&self) {
        let guard = self.ivars().borrow();
        let Some(m) = guard.as_ref() else { return };
        let frame = self.image_frame(m.size);
        m.image.drawInRect(frame);
        let crop = self.to_view(m.size, m.rect);

        // Dim everything outside the crop.
        let shade = NSBezierPath::bezierPathWithRect(frame);
        shade.appendBezierPathWithRect(crop);
        shade.setWindingRule(NSWindingRule::EvenOdd);
        NSColor::blackColor().colorWithAlphaComponent(0.5).setFill();
        shade.fill();

        // Rule of thirds while dragging.
        if m.drag.is_some() {
            NSColor::whiteColor().colorWithAlphaComponent(0.6).setStroke();
            for k in 1..3 {
                let t = k as f64 / 3.0;
                let v = NSBezierPath::bezierPath();
                v.moveToPoint(NSPoint::new(crop.origin.x + crop.size.width * t, crop.origin.y));
                v.lineToPoint(NSPoint::new(crop.origin.x + crop.size.width * t, crop.origin.y + crop.size.height));
                v.moveToPoint(NSPoint::new(crop.origin.x, crop.origin.y + crop.size.height * t));
                v.lineToPoint(NSPoint::new(crop.origin.x + crop.size.width, crop.origin.y + crop.size.height * t));
                v.setLineWidth(0.5);
                v.stroke();
            }
        }

        let border = NSBezierPath::bezierPathWithRect(crop);
        accent().setStroke();
        border.setLineWidth(2.0);
        border.stroke();
        accent().setFill();
        let (x0, y0, x1, y1) = (crop.origin.x, crop.origin.y, crop.origin.x + crop.size.width, crop.origin.y + crop.size.height);
        let (xm, ym) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        for (x, y) in [(x0, y0), (xm, y0), (x1, y0), (x1, ym), (x1, y1), (xm, y1), (x0, y1), (x0, ym)] {
            NSBezierPath::bezierPathWithOvalInRect(rect(x - 5.0, y - 5.0, 10.0, 10.0)).fill();
        }
    }
}

// ------------------------------------------------------------------ image preview

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaImagePreview"]
    #[ivars = RefCell<Option<Retained<NSImage>>>]
    pub struct ImagePreview;

    impl ImagePreview {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if let Some(img) = self.ivars().borrow().as_ref() {
                let s = img.size();
                let f = fit((s.width, s.height), self.bounds());
                img.drawInRect(f);
            }
        }
    }

    unsafe impl NSObjectProtocol for ImagePreview {}
);

impl ImagePreview {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn set(&self, img: Retained<NSImage>) {
        *self.ivars().borrow_mut() = Some(img);
        self.setNeedsDisplay(true);
    }
}

// ------------------------------------------------------------------ swatches

pub enum SwatchKind {
    Gradients,
    Solids,
}

pub struct SwatchModel {
    kind: SwatchKind,
    selected: Option<usize>,
    on_pick: Option<Box<dyn Fn(usize)>>,
}

const SWATCH: f64 = 26.0;
const SWATCH_GAP: f64 = 8.0;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaSwatchStrip"]
    #[ivars = RefCell<SwatchModel>]
    pub struct SwatchStrip;

    impl SwatchStrip {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let i = (p.x / (SWATCH + SWATCH_GAP)).floor() as usize;
            if i < self.count() && p.x - i as f64 * (SWATCH + SWATCH_GAP) <= SWATCH {
                self.select(Some(i));
                let cb = self.ivars().borrow_mut().on_pick.take();
                if let Some(cb) = cb {
                    cb(i);
                    self.ivars().borrow_mut().on_pick = Some(cb);
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for SwatchStrip {}
);

impl SwatchStrip {
    fn new(mtm: MainThreadMarker, origin: NSPoint, kind: SwatchKind) -> Retained<Self> {
        let n = match kind {
            SwatchKind::Gradients => GRADIENTS.len(),
            SwatchKind::Solids => SOLIDS.len(),
        };
        let frame = rect(origin.x, origin.y, n as f64 * (SWATCH + SWATCH_GAP), SWATCH);
        let this = Self::alloc(mtm).set_ivars(RefCell::new(SwatchModel { kind, selected: None, on_pick: None }));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn count(&self) -> usize {
        match self.ivars().borrow().kind {
            SwatchKind::Gradients => GRADIENTS.len(),
            SwatchKind::Solids => SOLIDS.len(),
        }
    }

    fn select(&self, i: Option<usize>) {
        self.ivars().borrow_mut().selected = i;
        self.setNeedsDisplay(true);
    }

    fn draw(&self) {
        let m = self.ivars().borrow();
        let c = |rgb: (u8, u8, u8)| NSColor::colorWithSRGBRed_green_blue_alpha(rgb.0 as f64 / 255.0, rgb.1 as f64 / 255.0, rgb.2 as f64 / 255.0, 1.0);
        for i in 0..self.count() {
            let r = rect(i as f64 * (SWATCH + SWATCH_GAP), 0.0, SWATCH, SWATCH);
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, 7.0, 7.0);
            match m.kind {
                SwatchKind::Gradients => {
                    let g = GRADIENTS[i];
                    let colors = NSArray::from_retained_slice(&[c(g.stops[0]), c(g.stops[1]), c(g.stops[2])]);
                    if let Some(gradient) = objc2_app_kit::NSGradient::initWithColors(objc2_app_kit::NSGradient::alloc(), &colors) {
                        gradient.drawInBezierPath_angle(&path, -45.0);
                    }
                }
                SwatchKind::Solids => {
                    c(SOLIDS[i].1).setFill();
                    path.fill();
                }
            }
            NSColor::separatorColor().setStroke();
            path.setLineWidth(0.5);
            path.stroke();
            if m.selected == Some(i) {
                let ring = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(r.origin.x + 1.5, r.origin.y + 1.5, SWATCH - 3.0, SWATCH - 3.0), 6.0, 6.0);
                accent().setStroke();
                ring.setLineWidth(3.0);
                ring.stroke();
            }
        }
    }
}

// ------------------------------------------------------------------ editor state

struct CropState {
    files: Vec<PathBuf>,
    size: (f64, f64),
    view: Retained<CropView>,
    aspect: Retained<NSSegmentedControl>,
    width: Retained<NSTextField>,
    height: Retained<NSTextField>,
}

struct FrameState {
    files: Vec<PathBuf>,
    full_size: (f64, f64),
    /// Downscaled source for fast previews, and its scale relative to the original.
    preview_src: Image,
    preview_scale: f64,
    background_image: Option<Image>,
    spec: FrameSpec,
    preview: Retained<ImagePreview>,
    gradients: Retained<SwatchStrip>,
    solids: Retained<SwatchStrip>,
    sliders: [Retained<NSSlider>; 3],
    values: [Retained<NSTextField>; 3],
    ratio: Retained<NSSegmentedControl>,
}

struct AdjustState {
    files: Vec<PathBuf>,
    preview_src: Image,
    original: Retained<NSImage>,
    adjustments: Adjustments,
    preview: Retained<ImagePreview>,
    section: Retained<NSSegmentedControl>,
    /// One row per adjustment: label, slider, value.
    rows: Vec<(Retained<NSTextField>, Retained<NSSlider>, Retained<NSTextField>)>,
    show_original: Retained<NSButton>,
}

enum EditorState {
    Crop(CropState),
    Frame(FrameState),
    Adjust(AdjustState),
}

fn signed(v: f64) -> String {
    if v > 0.0 {
        format!("+{}", v as i64)
    } else if v < 0.0 {
        format!("−{}", (-v) as i64)
    } else {
        "0".into()
    }
}

/// Rows are laid out from the top of the slider area; sections with fewer sliders leave the
/// gap at the bottom.
const ADJUST_ROWS_TOP: f64 = 58.0 + 7.0 * 30.0;

pub struct EditorIvars {
    state: RefCell<Option<EditorState>>,
    window: RefCell<Option<Retained<NSWindow>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaEditorTarget"]
    #[ivars = EditorIvars]
    pub struct EditorTarget;

    impl EditorTarget {
        #[unsafe(method(aspectChanged:))]
        fn aspect_changed(&self, _sender: Option<&AnyObject>) {
            self.crop_aspect_changed();
        }

        #[unsafe(method(sizeEdited:))]
        fn size_edited(&self, _sender: Option<&AnyObject>) {
            self.crop_size_edited();
        }

        #[unsafe(method(sliderChanged:))]
        fn slider_changed(&self, _sender: Option<&AnyObject>) {
            self.frame_controls_changed();
        }

        #[unsafe(method(ratioChanged:))]
        fn ratio_changed(&self, _sender: Option<&AnyObject>) {
            self.frame_controls_changed();
        }

        #[unsafe(method(colorChanged:))]
        fn color_changed(&self, sender: &NSColorWell) {
            let c = sender.color().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
            if let Some(c) = c {
                let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                self.set_background(Background::Solid { r: to(c.redComponent()), g: to(c.greenComponent()), b: to(c.blueComponent()) });
            }
        }

        #[unsafe(method(chooseImage:))]
        fn choose_image(&self, _sender: Option<&AnyObject>) {
            let mtm = self.mtm();
            let panel = objc2_app_kit::NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            panel.setMessage(Some(&ns("Choose an image for the background.")));
            if panel.runModal() == 1 {
                if let Some(path) = panel.URLs().iter().next().and_then(|u| u.to_file_path()) {
                    match imaging::load(&path) {
                        Ok(img) => {
                            if let Some(EditorState::Frame(f)) = self.ivars().state.borrow_mut().as_mut() {
                                f.background_image = Some(img);
                            }
                            self.set_background(Background::Image(path));
                        }
                        Err(e) => system::alert(mtm, "Couldn't use that image", &e.to_string()),
                    }
                }
            }
        }

        #[unsafe(method(sectionChanged:))]
        fn section_changed(&self, _sender: Option<&AnyObject>) {
            self.show_section();
        }

        #[unsafe(method(adjustmentChanged:))]
        fn adjustment_changed(&self, sender: &NSSlider) {
            self.on_adjustment(sender);
        }

        #[unsafe(method(toggleOriginal:))]
        fn toggle_original(&self, _sender: Option<&AnyObject>) {
            self.render_adjust_preview();
        }

        #[unsafe(method(reset:))]
        fn reset(&self, _sender: Option<&AnyObject>) {
            self.reset_state();
        }

        #[unsafe(method(apply:))]
        fn apply(&self, _sender: Option<&AnyObject>) {
            self.finish();
        }
    }

    unsafe impl NSObjectProtocol for EditorTarget {}
);

impl EditorTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(EditorIvars { state: RefCell::new(None), window: RefCell::new(None) });
        unsafe { msg_send![super(this), init] }
    }

    fn window(&self) -> Option<Retained<NSWindow>> {
        self.ivars().window.borrow().clone()
    }

    // ---- crop

    fn crop_aspect_changed(&self) {
        let (view, size, idx) = match self.ivars().state.borrow().as_ref() {
            Some(EditorState::Crop(c)) => (c.view.clone(), c.size, c.aspect.selectedSegment()),
            _ => return,
        };
        let aspect = CROP_ASPECTS.get(idx.max(0) as usize).and_then(|a| a.1).map(ratio_value);
        let current = {
            let mut g = view.ivars().borrow_mut();
            let Some(m) = g.as_mut() else { return };
            m.aspect = aspect;
            m.rect
        };
        if let Some(a) = aspect {
            view.set_rect(fit_aspect(current, a, size), true);
        } else {
            view.setNeedsDisplay(true);
        }
    }

    fn crop_size_edited(&self) {
        let (view, size, w, h, seg) = match self.ivars().state.borrow().as_ref() {
            Some(EditorState::Crop(c)) => (c.view.clone(), c.size, c.width.integerValue(), c.height.integerValue(), c.aspect.clone()),
            _ => return,
        };
        let current = match view.ivars().borrow().as_ref() {
            Some(m) => m.rect,
            None => return,
        };
        // Typed sizes override the aspect preset.
        seg.setSelectedSegment(0);
        if let Some(m) = view.ivars().borrow_mut().as_mut() {
            m.aspect = None;
        }
        view.set_rect(set_size(current, w.max(1) as f64, h.max(1) as f64, size), true);
    }

    // ---- frame

    fn frame_controls_changed(&self) {
        if let Some(EditorState::Frame(f)) = self.ivars().state.borrow_mut().as_mut() {
            f.spec.padding = f.sliders[0].doubleValue().round();
            f.spec.corners = f.sliders[1].doubleValue().round();
            f.spec.shadow = f.sliders[2].doubleValue().round();
            for (label, v) in f.values.iter().zip([f.spec.padding, f.spec.corners, f.spec.shadow]) {
                label.setStringValue(&ns(&format!("{} px", v as i64)));
            }
            f.spec.ratio = FRAME_RATIOS.get(f.ratio.selectedSegment().max(0) as usize).and_then(|r| r.1);
        }
        self.render_frame_preview();
    }

    fn set_background(&self, bg: Background) {
        if let Some(EditorState::Frame(f)) = self.ivars().state.borrow_mut().as_mut() {
            match &bg {
                Background::Gradient(i) => {
                    f.gradients.select(Some(*i));
                    f.solids.select(None);
                }
                Background::Solid { r, g, b } => {
                    f.gradients.select(None);
                    let idx = SOLIDS.iter().position(|s| s.1 == (*r, *g, *b));
                    f.solids.select(idx);
                }
                Background::Image(_) => {
                    f.gradients.select(None);
                    f.solids.select(None);
                }
            }
            f.spec.background = bg;
        }
        self.render_frame_preview();
    }

    fn render_frame_preview(&self) {
        let rendered = match self.ivars().state.borrow().as_ref() {
            Some(EditorState::Frame(f)) => {
                let s = f.preview_scale;
                let scaled = FrameSpec { padding: f.spec.padding * s, corners: f.spec.corners * s, shadow: f.spec.shadow * s, ..f.spec.clone() };
                imaging::frame(&f.preview_src, &scaled, f.background_image.as_deref(), 1.0).ok().map(|img| (f.preview.clone(), img))
            }
            _ => None,
        };
        if let Some((view, img)) = rendered {
            view.set(nsimage(&img));
        }
    }

    // ---- edit photo

    fn on_adjustment(&self, slider: &NSSlider) {
        let i = slider.tag() as usize;
        if let Some(EditorState::Adjust(a)) = self.ivars().state.borrow_mut().as_mut() {
            a.adjustments.set(i, slider.doubleValue());
            let v = a.adjustments.0[i];
            a.rows[i].2.setStringValue(&ns(&signed(v)));
            a.show_original.setState(0);
        }
        self.render_adjust_preview();
    }

    fn show_section(&self) {
        if let Some(EditorState::Adjust(a)) = self.ivars().state.borrow().as_ref() {
            let section = Section::ALL[(a.section.selectedSegment().max(0) as usize).min(3)];
            let visible = Adjustments::section(section);
            for (i, (label, slider, value)) in a.rows.iter().enumerate() {
                let row = visible.iter().position(|v| *v == i);
                for view in [&**label as &NSView, &**slider, &**value] {
                    view.setHidden(row.is_none());
                }
                if let Some(k) = row {
                    let y = ADJUST_ROWS_TOP - k as f64 * 30.0;
                    label.setFrame(rect(16.0, y + 3.0, 100.0, 18.0));
                    slider.setFrame(rect(120.0, y, W - 120.0 - 80.0, 24.0));
                    value.setFrame(rect(W - 72.0, y + 3.0, 56.0, 18.0));
                }
            }
        }
    }

    fn render_adjust_preview(&self) {
        let job = match self.ivars().state.borrow().as_ref() {
            Some(EditorState::Adjust(a)) => {
                if a.show_original.state() == NSControlStateValueOn {
                    a.preview.set(a.original.clone());
                    None
                } else {
                    Some((a.preview.clone(), photo::apply(&a.preview_src, &a.adjustments)))
                }
            }
            _ => None,
        };
        if let Some((view, Ok(img))) = job {
            view.set(nsimage(&img));
        }
    }

    // ---- shared

    fn reset_state(&self) {
        let mut crop_view = None;
        if let Some(state) = self.ivars().state.borrow_mut().as_mut() {
            match state {
                EditorState::Crop(c) => {
                    c.aspect.setSelectedSegment(0);
                    crop_view = Some((c.view.clone(), c.size));
                }
                EditorState::Adjust(a) => {
                    a.adjustments = Adjustments::default();
                    for (_, slider, value) in &a.rows {
                        slider.setDoubleValue(0.0);
                        value.setStringValue(&ns("0"));
                    }
                    a.show_original.setState(0);
                }
                EditorState::Frame(f) => {
                    f.spec = FrameSpec::defaults_for(f.full_size);
                    f.background_image = None;
                    for (slider, v) in f.sliders.iter().zip([f.spec.padding, f.spec.corners, f.spec.shadow]) {
                        slider.setDoubleValue(v);
                    }
                    f.ratio.setSelectedSegment(0);
                    f.gradients.select(Some(0));
                    f.solids.select(None);
                }
            }
        }
        if let Some((view, size)) = crop_view {
            if let Some(m) = view.ivars().borrow_mut().as_mut() {
                m.aspect = None;
            }
            view.set_rect(Rect::new(0.0, 0.0, size.0, size.1), true);
        } else if matches!(self.ivars().state.borrow().as_ref(), Some(EditorState::Adjust(_))) {
            self.render_adjust_preview();
        } else {
            self.frame_controls_changed();
        }
    }

    fn finish(&self) {
        let job = match self.ivars().state.borrow().as_ref() {
            Some(EditorState::Crop(c)) => c.view.ivars().borrow().as_ref().map(|m| {
                let r = m.rect.rounded(c.size);
                (Operation::Crop(CropSpec::from_pixels(r, c.size, None)), c.files.clone())
            }),
            Some(EditorState::Frame(f)) => Some((Operation::Frame(f.spec.clone()), f.files.clone())),
            Some(EditorState::Adjust(a)) => Some((Operation::Adjust(a.adjustments), a.files.clone())),
            None => None,
        };
        let Some(window) = self.window() else { return };
        if let Some((op, files)) = job {
            let fr = window.frame();
            let at = NSPoint::new(fr.origin.x + fr.size.width / 2.0, fr.origin.y + fr.size.height / 2.0);
            post(UiEvent::EditorApply { op, files, at });
        }
        window.close();
    }
}

// ------------------------------------------------------------------ window building

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect, align: NSTextAlignment) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(text), mtm);
    l.setFrame(frame);
    l.setAlignment(align);
    l.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, unsafe { NSFontWeightRegular })));
    l
}

fn segmented(mtm: MainThreadMarker, labels: &[&str], frame: NSRect, target: &EditorTarget, action: Sel) -> Retained<NSSegmentedControl> {
    let strings: Vec<Retained<NSString>> = labels.iter().map(|l| ns(l)).collect();
    let array = NSArray::from_retained_slice(&strings);
    let seg = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(&array, NSSegmentSwitchTracking::SelectOne, Some(target), Some(action), mtm)
    };
    seg.setFrame(frame);
    seg.setSelectedSegment(0);
    seg
}

fn button(mtm: MainThreadMarker, title: &str, frame: NSRect, target: &EditorTarget, action: Sel, primary: bool) -> Retained<NSButton> {
    let b = unsafe { NSButton::buttonWithTitle_target_action(&ns(title), Some(target), Some(action), mtm) };
    b.setFrame(frame);
    b.setBezelStyle(NSBezelStyle::Push);
    if primary {
        b.setKeyEquivalent(&ns("\r"));
        b.setBezelColor(Some(&accent()));
    }
    b
}

fn window(mtm: MainThreadMarker, title: &str, height: f64, delegate: &ProtocolObject<dyn NSWindowDelegate>) -> (Retained<NSWindow>, Retained<NSVisualEffectView>) {
    let frame = rect(0.0, 0.0, W, height);
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::FullSizeContentView;
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&ns(title));
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Visible);
    window.setMovableByWindowBackground(true);
    window.setLevel(3); // floating, so it stays above Finder
    window.setDelegate(Some(delegate));
    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
    effect.setMaterial(NSVisualEffectMaterial::Popover);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::Active);
    window.setContentView(Some(&effect));
    (window, effect)
}

pub struct EditorWindow {
    pub window: Retained<NSWindow>,
    _target: Retained<EditorTarget>,
}

#[cfg(feature = "demo")]
#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Down,
    Drag,
    Up,
}

impl EditorWindow {
    pub fn window_number(&self) -> isize {
        self.window.windowNumber()
    }

    fn show(window: &NSWindow, mtm: MainThreadMarker) {
        system::activate(mtm);
        window.center();
        window.makeKeyAndOrderFront(None);
    }

    /// Crop Image: preview with handles, aspect presets, exact size, Reset and Apply.
    pub fn crop(mtm: MainThreadMarker, files: Vec<PathBuf>, delegate: &ProtocolObject<dyn NSWindowDelegate>, show: bool) -> Result<Self, String> {
        let first = files.first().ok_or("No image to crop.")?;
        let img = imaging::load(first).map_err(|e| e.to_string())?;
        let size = imaging::size(&img);
        let height = 640.0;
        let (window, content) = window(mtm, "Crop Image", height, delegate);
        let target = EditorTarget::new(mtm);
        let t: &EditorTarget = &target;

        content.addSubview(&button(mtm, "Reset", rect(16.0, 14.0, 96.0, 30.0), t, sel!(reset:), false));
        content.addSubview(&button(mtm, "Apply", rect(W - 16.0 - 110.0, 14.0, 110.0, 30.0), t, sel!(apply:), true));

        content.addSubview(&label(mtm, "W", rect(16.0, 63.0, 18.0, 18.0), NSTextAlignment::Left));
        let width = NSTextField::textFieldWithString(&ns(&format!("{}", size.0 as i64)), mtm);
        width.setFrame(rect(36.0, 60.0, 80.0, 24.0));
        content.addSubview(&label(mtm, "×", rect(120.0, 63.0, 16.0, 18.0), NSTextAlignment::Center));
        let heightf = NSTextField::textFieldWithString(&ns(&format!("{}", size.1 as i64)), mtm);
        heightf.setFrame(rect(140.0, 60.0, 80.0, 24.0));
        content.addSubview(&label(mtm, "px", rect(226.0, 63.0, 24.0, 18.0), NSTextAlignment::Left));
        for f in [&width, &heightf] {
            unsafe {
                f.setTarget(Some(t));
                f.setAction(Some(sel!(sizeEdited:)));
            }
            content.addSubview(f);
        }

        content.addSubview(&label(mtm, "Aspect ratio", rect(16.0, 101.0, 90.0, 18.0), NSTextAlignment::Left));
        let names: Vec<&str> = CROP_ASPECTS.iter().map(|a| a.0).collect();
        let aspect = segmented(mtm, &names, rect(108.0, 96.0, W - 124.0, 26.0), t, sel!(aspectChanged:));
        content.addSubview(&aspect);

        // Grown by GRAB - 8 on each side so the image keeps its size and position.
        let view = CropView::new(mtm, rect(12.0, 130.0, W - 24.0, height - 130.0 - 32.0));
        let (wf, hf) = (width.clone(), heightf.clone());
        *view.ivars().borrow_mut() = Some(CropModel {
            image: nsimage(&img),
            size,
            rect: Rect::new(0.0, 0.0, size.0, size.1),
            aspect: None,
            drag: None,
            on_change: Some(Box::new(move |r: Rect| {
                wf.setStringValue(&ns(&format!("{}", r.w.round() as i64)));
                hf.setStringValue(&ns(&format!("{}", r.h.round() as i64)));
            })),
        });
        content.addSubview(&view);

        *target.ivars().state.borrow_mut() = Some(EditorState::Crop(CropState { files, size, view, aspect, width, height: heightf }));
        *target.ivars().window.borrow_mut() = Some(window.clone());
        if show {
            Self::show(&window, mtm);
        }
        Ok(EditorWindow { window, _target: target })
    }

    /// Add Background: gradient/solid/image background, padding, corners, shadow and ratio.
    pub fn frame(mtm: MainThreadMarker, files: Vec<PathBuf>, delegate: &ProtocolObject<dyn NSWindowDelegate>, show: bool) -> Result<Self, String> {
        let first = files.first().ok_or("No image for the background.")?;
        let img = imaging::load(first).map_err(|e| e.to_string())?;
        let full_size = imaging::size(&img);
        let preview_scale = (900.0 / full_size.0.max(full_size.1)).min(1.0);
        let preview_src = imaging::crop(
            &img,
            Rect::new(0.0, 0.0, full_size.0, full_size.1),
            Some(((full_size.0 * preview_scale).round().max(1.0) as u32, (full_size.1 * preview_scale).round().max(1.0) as u32)),
        )
        .map_err(|e| e.to_string())?;
        let spec = FrameSpec::defaults_for(full_size);
        let limits = FrameSpec::limits_for(full_size);

        let height = 700.0;
        let (window, content) = window(mtm, "Add Background", height, delegate);
        let target = EditorTarget::new(mtm);
        let t: &EditorTarget = &target;

        content.addSubview(&button(mtm, "Reset", rect(16.0, 14.0, 96.0, 30.0), t, sel!(reset:), false));
        content.addSubview(&button(mtm, "Save with Background", rect(W - 16.0 - 190.0, 14.0, 190.0, 30.0), t, sel!(apply:), true));

        content.addSubview(&label(mtm, "Ratio", rect(16.0, 63.0, 90.0, 18.0), NSTextAlignment::Left));
        let ratio_names: Vec<&str> = FRAME_RATIOS.iter().map(|r| r.0).collect();
        let ratio = segmented(mtm, &ratio_names, rect(W - 16.0 - 230.0, 58.0, 230.0, 26.0), t, sel!(ratioChanged:));
        content.addSubview(&ratio);

        let rows = [("Shadow", spec.shadow, limits.2, 96.0), ("Corners", spec.corners, limits.1, 126.0), ("Padding", spec.padding, limits.0, 156.0)];
        let mut sliders = Vec::new();
        let mut values = Vec::new();
        for (name, value, max, y) in rows {
            content.addSubview(&label(mtm, name, rect(16.0, y + 3.0, 80.0, 18.0), NSTextAlignment::Left));
            let s = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(value, 0.0, max, Some(t), Some(sel!(sliderChanged:)), mtm) };
            s.setFrame(rect(100.0, y, W - 100.0 - 90.0, 24.0));
            content.addSubview(&s);
            let v = label(mtm, &format!("{} px", value as i64), rect(W - 84.0, y + 3.0, 68.0, 18.0), NSTextAlignment::Right);
            content.addSubview(&v);
            sliders.push(s);
            values.push(v);
        }
        // Display order is Padding, Corners, Shadow.
        let sliders: [Retained<NSSlider>; 3] = [sliders[2].clone(), sliders[1].clone(), sliders[0].clone()];
        let values: [Retained<NSTextField>; 3] = [values[2].clone(), values[1].clone(), values[0].clone()];

        content.addSubview(&label(mtm, "Background", rect(16.0, 250.0, 120.0, 18.0), NSTextAlignment::Left));
        let gradients = SwatchStrip::new(mtm, NSPoint::new(16.0, 216.0), SwatchKind::Gradients);
        let solids = SwatchStrip::new(mtm, NSPoint::new(16.0, 186.0), SwatchKind::Solids);
        content.addSubview(&gradients);
        content.addSubview(&solids);
        let image_button = unsafe { NSButton::buttonWithTitle_target_action(&ns("Image…"), Some(t), Some(sel!(chooseImage:)), mtm) };
        image_button.setFrame(rect(W - 16.0 - 84.0, 213.0, 84.0, 30.0));
        image_button.setToolTip(Some(&ns("Use another image as the background")));
        content.addSubview(&image_button);
        let well = NSColorWell::colorWellWithStyle(NSColorWellStyle::Minimal, mtm);
        well.setFrame(rect(W - 16.0 - 44.0, 186.0, 44.0, 26.0));
        unsafe {
            well.setTarget(Some(t));
            well.setAction(Some(sel!(colorChanged:)));
        }
        well.setToolTip(Some(&ns("Choose a custom background colour")));
        content.addSubview(&well);

        let preview = ImagePreview::new(mtm, rect(16.0, 282.0, W - 32.0, height - 282.0 - 36.0));
        content.addSubview(&preview);

        let weak_target = target.clone();
        gradients.ivars().borrow_mut().on_pick = Some(Box::new(move |i| weak_target.set_background(Background::Gradient(i))));
        let weak_target = target.clone();
        solids.ivars().borrow_mut().on_pick = Some(Box::new(move |i| {
            let (r, g, b) = SOLIDS[i].1;
            weak_target.set_background(Background::Solid { r, g, b });
        }));
        gradients.select(Some(0));

        *target.ivars().state.borrow_mut() = Some(EditorState::Frame(FrameState {
            files,
            full_size,
            preview_src,
            preview_scale,
            background_image: None,
            spec,
            preview,
            gradients,
            solids,
            sliders,
            values,
            ratio,
        }));
        *target.ivars().window.borrow_mut() = Some(window.clone());
        target.render_frame_preview();
        if show {
            Self::show(&window, mtm);
        }
        Ok(EditorWindow { window, _target: target })
    }

    /// Sets one adjustment as if its slider moved (used by `satsuma render-editor`).
    pub fn set_adjustment(&self, key: &str, value: f64) {
        let slider = match self._target.ivars().state.borrow().as_ref() {
            Some(EditorState::Adjust(a)) => Adjustments::index(key).map(|i| a.rows[i].1.clone()),
            _ => None,
        };
        if let Some(slider) = slider {
            slider.setDoubleValue(value);
            self._target.on_adjustment(&slider);
        }
    }

    /// Selects a segment of the editor's first segmented control as a user would (used by
    /// `satsuma render-editor`), then snapshots the window content to a PNG.
    pub fn render_png(&self, segment: Option<isize>, out: &std::path::Path) -> Result<(), String> {
        if let Some(i) = segment {
            self.select_segment(i);
        }
        let rep = self.snapshot()?;
        let data = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) }
            .ok_or("Could not encode the PNG.")?;
        std::fs::write(out, data.to_vec()).map_err(|e| e.to_string())
    }

    /// Picks a segment of the editor's main control (crop aspect, background ratio or photo section).
    pub fn select_segment(&self, i: isize) {
        match self._target.ivars().state.borrow().as_ref() {
            Some(EditorState::Crop(c)) => c.aspect.setSelectedSegment(i),
            Some(EditorState::Frame(f)) => f.ratio.setSelectedSegment(i),
            Some(EditorState::Adjust(a)) => a.section.setSelectedSegment(i),
            None => {}
        }
        self._target.crop_aspect_changed();
        self._target.frame_controls_changed();
        self._target.show_section();
    }

    /// The window's content (title bar excluded) drawn into a bitmap.
    pub fn snapshot(&self) -> Result<Retained<objc2_app_kit::NSBitmapImageRep>, String> {
        let view = self.window.contentView().ok_or("No content view.")?;
        let bounds = view.bounds();
        let rep = view.bitmapImageRepForCachingDisplayInRect(bounds).ok_or("Could not create a bitmap.")?;
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        Ok(rep)
    }

    #[cfg(feature = "demo")]
    fn crop_view(&self) -> Option<Retained<CropView>> {
        match self._target.ivars().state.borrow().as_ref() {
            Some(EditorState::Crop(c)) => Some(c.view.clone()),
            _ => None,
        }
    }

    /// The crop rectangle in window coordinates (for the scripted demo).
    #[cfg(feature = "demo")]
    pub fn crop_frame(&self) -> Option<NSRect> {
        let view = self.crop_view()?;
        let r = {
            let guard = view.ivars().borrow();
            let m = guard.as_ref()?;
            view.to_view(m.size, m.rect)
        };
        Some(view.convertRect_toView(r, None))
    }

    /// The crop rectangle in image pixels (y down).
    #[cfg(feature = "demo")]
    pub fn crop_rect(&self) -> Option<Rect> {
        let view = self.crop_view()?;
        let r = view.ivars().borrow().as_ref().map(|m| m.rect);
        r
    }

    /// Scripted pointer input on the crop preview, in window coordinates: press, drag, release.
    #[cfg(feature = "demo")]
    pub fn crop_pointer(&self, p: NSPoint, phase: Pointer) {
        let Some(view) = self.crop_view() else { return };
        let q = view.convertPoint_fromView(p, None);
        match phase {
            Pointer::Down => view.begin_drag(q),
            Pointer::Drag => view.continue_drag(q),
            Pointer::Up => view.end_drag(),
        }
    }

    /// Edit Photo: live preview, Light/Color/Detail/Effects sliders, Show Original, Save Edited Copy.
    pub fn adjust(mtm: MainThreadMarker, files: Vec<PathBuf>, delegate: &ProtocolObject<dyn NSWindowDelegate>, show: bool) -> Result<Self, String> {
        let first = files.first().ok_or("No photo to edit.")?;
        let img = imaging::load(first).map_err(|e| e.to_string())?;
        let size = imaging::size(&img);
        let scale = (900.0 / size.0.max(size.1)).min(1.0);
        let preview_src = imaging::crop(
            &img,
            Rect::new(0.0, 0.0, size.0, size.1),
            Some(((size.0 * scale).round().max(1.0) as u32, (size.1 * scale).round().max(1.0) as u32)),
        )
        .map_err(|e| e.to_string())?;

        let height = 780.0;
        let (window, content) = window(mtm, "Edit Photo", height, delegate);
        let target = EditorTarget::new(mtm);
        let t: &EditorTarget = &target;

        content.addSubview(&button(mtm, "Reset", rect(16.0, 14.0, 90.0, 30.0), t, sel!(reset:), false));
        let show_original = button(mtm, "Show Original", rect(112.0, 14.0, 130.0, 30.0), t, sel!(toggleOriginal:), false);
        show_original.setButtonType(NSButtonType::PushOnPushOff);
        content.addSubview(&show_original);
        content.addSubview(&button(mtm, "Save Edited Copy", rect(W - 16.0 - 170.0, 14.0, 170.0, 30.0), t, sel!(apply:), true));

        let mut rows = Vec::new();
        for (i, info) in ADJUSTMENTS.iter().enumerate() {
            let l = label(mtm, info.label, rect(16.0, 0.0, 100.0, 18.0), NSTextAlignment::Left);
            let s = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(0.0, info.min, info.max, Some(t), Some(sel!(adjustmentChanged:)), mtm) };
            s.setTag(i as isize);
            let v = label(mtm, "0", rect(0.0, 0.0, 56.0, 18.0), NSTextAlignment::Right);
            content.addSubview(&l);
            content.addSubview(&s);
            content.addSubview(&v);
            rows.push((l, s, v));
        }

        let names: Vec<&str> = Section::ALL.iter().map(|s| s.name()).collect();
        let section = segmented(mtm, &names, rect((W - 320.0) / 2.0, 306.0, 320.0, 26.0), t, sel!(sectionChanged:));
        content.addSubview(&section);

        let preview = ImagePreview::new(mtm, rect(16.0, 344.0, W - 32.0, height - 344.0 - 36.0));
        content.addSubview(&preview);
        let original = nsimage(&preview_src);
        preview.set(original.clone());

        *target.ivars().state.borrow_mut() = Some(EditorState::Adjust(AdjustState {
            files,
            preview_src,
            original,
            adjustments: Adjustments::default(),
            preview,
            section,
            rows,
            show_original,
        }));
        *target.ivars().window.borrow_mut() = Some(window.clone());
        target.show_section();
        if show {
            Self::show(&window, mtm);
        }
        Ok(EditorWindow { window, _target: target })
    }

    /// Breaks the swatch → target reference cycle when the window closes.
    pub fn teardown(&self) {
        if let Some(EditorState::Frame(f)) = self._target.ivars().state.borrow().as_ref() {
            f.gradients.ivars().borrow_mut().on_pick = None;
            f.solids.ivars().borrow_mut().on_pick = None;
        }
        *self._target.ivars().state.borrow_mut() = None;
        *self._target.ivars().window.borrow_mut() = None;
    }
}
