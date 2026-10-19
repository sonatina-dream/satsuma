//! The radial wheel: a borderless floating panel that is a drop target during file drags,
//! and a keyboard/click menu when opened from the menu bar.

use crate::draw::{self, accent, attrs, image_centered, path_from, rect, symbol, text_centered, text_in};
use crate::events::{post, UiEvent};
use crate::system::{file_paths, highlight_feedback, mods_from_flags, screen_frame_for};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSBackingStoreType, NSBezierPath, NSBitmapImageFileType, NSBitmapImageRep, NSColor,
    NSCalibratedRGBColorSpace, NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSEvent,
    NSFont, NSFontWeightBold, NSFontWeightSemibold, NSPanel, NSPasteboardTypeFileURL,
    NSTextAlignment, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSObjectProtocol, NSPoint, NSRect, NSSize};
use satsuma_core::geometry::{Point, WheelGeometry, WHEEL_VIEW_SIZE};
use satsuma_core::wheel::{empty_message, navigate, NavKey};
use satsuma_core::{WheelCombos, WheelItem, WheelMode};
use std::cell::RefCell;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct WheelModel {
    pub mode: WheelMode,
    pub items: Vec<WheelItem>,
    pub selected: Option<usize>,
    /// Opened from the menu (keyboard and clicks) rather than during a drag.
    pub keyboard: bool,
    pub feedback: bool,
    pub combos: WheelCombos,
    /// Centre text when nothing is highlighted, e.g. ["1.2 MB"] (E10).
    pub hub: Vec<String>,
}

impl WheelModel {
    pub fn new(mode: WheelMode, items: Vec<WheelItem>) -> Self {
        WheelModel { mode, items, selected: None, keyboard: false, feedback: true, combos: WheelCombos::default(), hub: Vec::new() }
    }
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; WheelView does not implement Drop.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaWheelView"]
    #[ivars = RefCell<WheelModel>]
    pub struct WheelView;

    impl WheelView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.handle_key(event.keyCode());
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let (keyboard, combos) = {
                let m = self.ivars().borrow();
                (m.keyboard, m.combos)
            };
            if keyboard {
                if let Some(mode) = combos.mode_for(mods_from_flags(event.modifierFlags())) {
                    post(UiEvent::WheelSwitch(mode));
                }
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            if self.ivars().borrow().keyboard {
                let p = self.convertPoint_fromView(event.locationInWindow(), None);
                if let Some(i) = self.hit(p) {
                    self.set_selected(Some(i));
                }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !self.ivars().borrow().keyboard {
                return;
            }
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            match self.hit(p) {
                Some(i) => {
                    self.set_selected(Some(i));
                    self.pick();
                }
                None => post(UiEvent::WheelCancel),
            }
        }
    }

    unsafe impl NSObjectProtocol for WheelView {}

    unsafe impl NSDraggingDestination for WheelView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_update(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_update(sender)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.set_selected(None);
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.ivars().borrow().selected.is_some()
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let item = {
                let m = self.ivars().borrow();
                m.selected.and_then(|i| m.items.get(i).copied())
            };
            self.drop_files(item, sender)
        }
    }
);

impl WheelView {
    pub fn new(mtm: MainThreadMarker, model: WheelModel) -> Retained<Self> {
        let frame = rect(0.0, 0.0, WHEEL_VIEW_SIZE, WHEEL_VIEW_SIZE);
        let this = Self::alloc(mtm).set_ivars(RefCell::new(model));
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let types = NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]);
        view.registerForDraggedTypes(&types);
        view
    }

    pub fn set_model(&self, model: WheelModel) {
        *self.ivars().borrow_mut() = model;
        self.setNeedsDisplay(true);
    }

    pub fn model(&self) -> WheelModel {
        self.ivars().borrow().clone()
    }

    fn center(&self) -> NSPoint {
        let b = self.bounds();
        NSPoint::new(b.size.width / 2.0, b.size.height / 2.0)
    }

    fn hit(&self, p: NSPoint) -> Option<usize> {
        let c = self.center();
        let n = self.ivars().borrow().items.len();
        WheelGeometry::default().hit_test(p.x - c.x, p.y - c.y, n)
    }

    fn set_selected(&self, selected: Option<usize>) {
        let feedback = {
            let mut m = self.ivars().borrow_mut();
            if m.selected == selected {
                return;
            }
            m.selected = selected;
            m.feedback
        };
        if feedback && selected.is_some() {
            highlight_feedback();
        }
        self.setNeedsDisplay(true);
    }

    fn drag_update(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        let p = self.convertPoint_fromView(sender.draggingLocation(), None);
        let hit = self.hit(p);
        self.set_selected(hit);
        if hit.is_some() {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    fn drop_files(&self, item: Option<WheelItem>, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let Some(item) = item else { return false };
        let files = file_paths(&sender.draggingPasteboard());
        if files.is_empty() {
            return false;
        }
        let at = self
            .window()
            .map(|w| {
                let f = w.frame();
                NSPoint::new(f.origin.x + f.size.width / 2.0, f.origin.y + f.size.height / 2.0)
            })
            .unwrap_or(NSPoint::new(0.0, 0.0));
        post(UiEvent::WheelDrop { item, files, at });
        true
    }

    fn pick(&self) {
        let item = {
            let m = self.ivars().borrow();
            m.selected.and_then(|i| m.items.get(i).copied())
        };
        if let Some(item) = item {
            post(UiEvent::WheelPick { item });
        }
    }

    fn handle_key(&self, code: u16) {
        let nav = match code {
            124 | 125 => Some(NavKey::Next),     // → ↓
            123 | 126 => Some(NavKey::Previous), // ← ↑
            115 => Some(NavKey::First),          // Home
            119 => Some(NavKey::Last),           // End
            _ => None,
        };
        if let Some(key) = nav {
            let (selected, n) = {
                let m = self.ivars().borrow();
                (m.selected, m.items.len())
            };
            self.set_selected(navigate(selected, n, key));
            return;
        }
        match code {
            36 | 76 | 49 => self.pick(),                 // Return, Enter, Space
            53 | 48 => post(UiEvent::WheelCancel),       // Escape, Tab
            _ => {}
        }
    }

    fn draw(&self) {
        let m = self.ivars().borrow();
        let g = WheelGeometry::default();
        let c = self.center();

        let disc = NSBezierPath::bezierPathWithOvalInRect(rect(
            c.x - g.backdrop,
            c.y - g.backdrop,
            g.backdrop * 2.0,
            g.backdrop * 2.0,
        ));
        NSColor::windowBackgroundColor().colorWithAlphaComponent(0.55).setFill();
        disc.fill();
        NSColor::separatorColor().setStroke();
        disc.setLineWidth(0.5);
        disc.stroke();

        if m.items.is_empty() {
            let font = NSFont::systemFontOfSize_weight(12.0, unsafe { NSFontWeightSemibold });
            let a = draw::attrs_wrapped(&font, &NSColor::secondaryLabelColor(), Some(NSTextAlignment::Center));
            text_in(empty_message(m.mode), rect(c.x - 85.0, c.y - 30.0, 170.0, 60.0), &a);
            return;
        }

        let n = m.items.len();
        let dense = n > 6;
        for (i, item) in m.items.iter().enumerate() {
            let highlighted = m.selected == Some(i);
            let path = path_from(&g.petal_path(i, n), c.x, c.y);
            if highlighted {
                accent().setFill();
            } else {
                NSColor::controlBackgroundColor().colorWithAlphaComponent(0.92).setFill();
            }
            path.fill();
            if !highlighted {
                NSColor::separatorColor().setStroke();
                path.setLineWidth(0.5);
                path.stroke();
            }
            let color = if highlighted { NSColor::whiteColor() } else { NSColor::labelColor() };
            let lp = g.label_position(i, n);
            let center = NSPoint::new(c.x + lp.x, c.y + lp.y);
            let title = item.title();
            match item.symbol() {
                Some(name) => {
                    if let Some(img) = symbol(name, if dense { 13.0 } else { 15.0 }, &color) {
                        image_centered(&img, NSPoint::new(center.x, center.y + 8.0));
                    }
                    let font = NSFont::systemFontOfSize_weight(if dense { 7.5 } else { 8.5 }, unsafe { NSFontWeightBold });
                    text_centered(&title, NSPoint::new(center.x, center.y - 10.0), &attrs(&font, &color, None));
                }
                None => {
                    let font = NSFont::systemFontOfSize_weight(if dense { 10.5 } else { 12.0 }, unsafe { NSFontWeightBold });
                    text_centered(&title, center, &attrs(&font, &color, None));
                }
            }
        }

        let selected = m.selected.and_then(|i| m.items.get(i));
        if selected.is_some() || !m.hub.is_empty() {
            let r = g.inner - 6.0;
            let hub = NSBezierPath::bezierPathWithOvalInRect(rect(c.x - r, c.y - r, r * 2.0, r * 2.0));
            NSColor::controlBackgroundColor().colorWithAlphaComponent(if selected.is_some() { 1.0 } else { 0.85 }).setFill();
            hub.fill();
        }
        if let Some(item) = selected {
            let font = NSFont::systemFontOfSize_weight(9.5, unsafe { NSFontWeightBold });
            text_centered(&item.title(), c, &attrs(&font, &accent(), None));
        } else if !m.hub.is_empty() {
            let n = m.hub.len() as f64;
            for (k, line) in m.hub.iter().enumerate() {
                let (size, color) = if k == 0 { (10.5, NSColor::labelColor()) } else { (9.0, NSColor::secondaryLabelColor()) };
                let font = NSFont::systemFontOfSize_weight(size, unsafe { NSFontWeightSemibold });
                let y = c.y + (n - 1.0) * 6.5 - k as f64 * 13.0;
                text_centered(line, NSPoint::new(c.x, y), &attrs(&font, &color, None));
            }
        }
    }
}

define_class!(
    /// A borderless panel that may become key (for the keyboard mode).
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaKeyPanel"]
    pub struct KeyPanel;

    impl KeyPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }
    }
);

impl KeyPanel {
    pub fn new(mtm: MainThreadMarker, frame: NSRect, style: NSWindowStyleMask) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let panel: Retained<Self> = unsafe {
            msg_send![super(this), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        unsafe { panel.setReleasedWhenClosed(false) };
        panel
    }
}

/// The wheel panel and its view.
pub struct WheelWindow {
    pub panel: Retained<KeyPanel>,
    pub view: Retained<WheelView>,
    anchor: std::cell::Cell<NSPoint>,
}

impl WheelWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let size = WHEEL_VIEW_SIZE;
        let frame = rect(0.0, 0.0, size, size);
        let panel = KeyPanel::new(mtm, frame, NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        // setFloatingPanel resets the level, so it must come first.
        panel.setFloatingPanel(true);
        panel.setLevel(101); // NSPopUpMenuWindowLevel: above Finder windows and the Dock
        panel.setHidesOnDeactivate(false);
        panel.setAcceptsMouseMovedEvents(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let g = WheelGeometry::default();
        let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        effect.setMaterial(NSVisualEffectMaterial::Popover);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        let r = g.backdrop;
        let mask = draw::mask_image(NSSize::new(size, size), |b| {
            NSBezierPath::bezierPathWithOvalInRect(rect(
                b.size.width / 2.0 - r,
                b.size.height / 2.0 - r,
                r * 2.0,
                r * 2.0,
            ))
        });
        effect.setMaskImage(Some(&mask));

        let view = WheelView::new(mtm, WheelModel::new(WheelMode::Conversions, Vec::new()));
        effect.addSubview(&view);
        panel.setContentView(Some(&effect));
        WheelWindow { panel, view, anchor: std::cell::Cell::new(NSPoint::new(0.0, 0.0)) }
    }

    pub fn anchor(&self) -> NSPoint {
        self.anchor.get()
    }

    pub fn show(&self, mtm: MainThreadMarker, model: WheelModel, at: NSPoint) {
        let bounds = screen_frame_for(at, mtm);
        let g = WheelGeometry::default();
        let a = g.clamp_anchor(
            Point::new(at.x, at.y),
            (bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height),
        );
        let half = WHEEL_VIEW_SIZE / 2.0;
        self.panel.setFrameOrigin(NSPoint::new(a.x - half, a.y - half));
        self.anchor.set(NSPoint::new(a.x, a.y));
        let keyboard = model.keyboard;
        self.view.set_model(model);
        if keyboard {
            self.panel.makeKeyAndOrderFront(None);
            self.panel.makeFirstResponder(Some(&self.view));
        } else {
            self.panel.orderFrontRegardless();
        }
        self.panel.invalidateShadow();
    }

    /// Replaces the petals in place (anchor unchanged).
    pub fn switch(&self, mode: WheelMode, items: Vec<WheelItem>) {
        let mut m = self.view.model();
        m.mode = mode;
        m.selected = if m.keyboard && !items.is_empty() { Some(0) } else { None };
        m.items = items;
        self.view.set_model(m);
    }

    pub fn hide(&self) {
        self.panel.orderOut(None);
    }
}

/// Renders the wheel to a PNG at 2x, without any window (for docs and visual checks).
pub fn render_png(mtm: MainThreadMarker, model: WheelModel, dark: bool, out: &Path) -> Result<(), String> {
    let view = WheelView::new(mtm, model);
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    view.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
    let size = WHEEL_VIEW_SIZE;
    let px = (size * 2.0) as isize;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            px,
            px,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            0,
            32,
        )
    }
    .ok_or("Could not create a bitmap.")?;
    rep.setSize(NSSize::new(size, size));
    view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &rep);
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
        .ok_or("Could not encode the PNG.")?;
    std::fs::write(out, data.to_vec()).map_err(|e| e.to_string())
}
