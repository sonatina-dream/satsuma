//! Float: a small always-on-top shelf that holds files so they can be dragged into other apps.

use crate::draw::{accent, attrs_wrapped, rect, text_in};
use crate::events::{post, UiEvent};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSDragOperation, NSDraggingContext,
    NSDraggingDestination, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSFont, NSFontWeightRegular, NSImage, NSPanel,
    NSPasteboardTypeFileURL, NSPasteboardWriting, NSTextAlignment, NSView, NSWindowDelegate,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

const CELL_W: f64 = 92.0;
const CELL_H: f64 = 96.0;
const PAD: f64 = 10.0;
const ICON: f64 = 52.0;
const MAX_COLS: usize = 4;

pub struct FloatItem {
    pub path: PathBuf,
    icon: Retained<NSImage>,
}

#[derive(Default)]
pub struct FloatState {
    items: Vec<FloatItem>,
    selected: BTreeSet<usize>,
    pressed: Option<usize>,
    drop_target: bool,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaFloatView"]
    #[ivars = RefCell<FloatState>]
    pub struct FloatView;

    impl FloatView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let hit = self.index_at(p);
            let toggle = event.modifierFlags().contains(NSEventModifierFlags::Command);
            {
                let mut s = self.ivars().borrow_mut();
                s.pressed = hit;
                match hit {
                    Some(i) if toggle => {
                        if !s.selected.remove(&i) {
                            s.selected.insert(i);
                        }
                    }
                    Some(i) => {
                        if !s.selected.contains(&i) {
                            s.selected.clear();
                            s.selected.insert(i);
                        }
                    }
                    None => s.selected.clear(),
                }
            }
            if event.clickCount() == 2 {
                if let Some(i) = hit {
                    let path = self.ivars().borrow().items.get(i).map(|it| it.path.clone());
                    if let Some(url) = path.and_then(|p| NSURL::from_file_path(p)) {
                        NSWorkspace::sharedWorkspace().openURL(&url);
                    }
                }
            }
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let (paths, frames): (Vec<(PathBuf, Retained<NSImage>)>, Vec<NSRect>) = {
                let mut s = self.ivars().borrow_mut();
                if s.pressed.take().is_none() {
                    return;
                }
                let idx: Vec<usize> = s.selected.iter().copied().collect();
                (
                    idx.iter().map(|i| (s.items[*i].path.clone(), s.items[*i].icon.clone())).collect(),
                    idx.iter().map(|i| icon_rect(*i, self.columns())).collect(),
                )
            };
            let mut drag_items = Vec::new();
            for ((path, icon), frame) in paths.iter().zip(frames) {
                let Some(url) = NSURL::from_file_path(path) else { continue };
                let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*url);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
                unsafe { item.setDraggingFrame_contents(frame, Some(icon)) };
                drag_items.push(item);
            }
            if drag_items.is_empty() {
                return;
            }
            let array = NSArray::from_retained_slice(&drag_items);
            self.beginDraggingSessionWithItems_event_source(&array, event, ProtocolObject::from_ref(self));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if matches!(event.keyCode(), 51 | 117) {
                if let Some(w) = self.window() {
                    post(UiEvent::FloatRemoveSelected { window: w.windowNumber() });
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for FloatView {}

    unsafe impl NSDraggingSource for FloatView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy | NSDragOperation::Generic | NSDragOperation::Link
        }
    }

    unsafe impl NSDraggingDestination for FloatView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            // Ignore drags that started in this shelf.
            if sender.draggingSource().is_some() {
                return NSDragOperation::None;
            }
            self.ivars().borrow_mut().drop_target = true;
            self.setNeedsDisplay(true);
            NSDragOperation::Copy
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.ivars().borrow_mut().drop_target = false;
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.ivars().borrow_mut().drop_target = false;
            self.setNeedsDisplay(true);
            let files = crate::system::file_paths(&sender.draggingPasteboard());
            match self.window() {
                Some(w) if !files.is_empty() => {
                    post(UiEvent::FloatAdd { window: w.windowNumber(), files });
                    true
                }
                _ => false,
            }
        }
    }
);

fn icon_rect(i: usize, cols: usize) -> NSRect {
    let (col, row) = (i % cols, i / cols);
    let x = PAD + col as f64 * CELL_W + (CELL_W - ICON) / 2.0;
    let y = PAD + row as f64 * CELL_H + 8.0;
    rect(x, y, ICON, ICON)
}

fn cell_rect(i: usize, cols: usize) -> NSRect {
    let (col, row) = (i % cols, i / cols);
    rect(PAD + col as f64 * CELL_W, PAD + row as f64 * CELL_H, CELL_W, CELL_H)
}

/// Content size for `n` items.
pub fn content_size(n: usize) -> NSSize {
    let cols = n.clamp(1, MAX_COLS);
    let rows = n.max(1).div_ceil(cols);
    NSSize::new(PAD * 2.0 + cols as f64 * CELL_W, PAD * 2.0 + rows as f64 * CELL_H)
}

impl FloatView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(FloatState::default()));
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: rect(0.0, 0.0, 100.0, 100.0)] };
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        view
    }

    fn columns(&self) -> usize {
        let w = self.bounds().size.width;
        (((w - PAD * 2.0) / CELL_W).floor() as usize).max(1)
    }

    fn index_at(&self, p: NSPoint) -> Option<usize> {
        let cols = self.columns();
        let n = self.ivars().borrow().items.len();
        (0..n).find(|i| {
            let r = cell_rect(*i, cols);
            p.x >= r.origin.x && p.x < r.origin.x + r.size.width && p.y >= r.origin.y && p.y < r.origin.y + r.size.height
        })
    }

    fn draw(&self) {
        let s = self.ivars().borrow();
        let cols = self.columns();
        if s.drop_target {
            let b = self.bounds();
            let r = rect(3.0, 3.0, b.size.width - 6.0, b.size.height - 6.0);
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, 10.0, 10.0);
            accent().setStroke();
            path.setLineWidth(2.0);
            path.stroke();
        }
        let font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightRegular });
        for (i, item) in s.items.iter().enumerate() {
            let cell = cell_rect(i, cols);
            if s.selected.contains(&i) {
                let hl = rect(cell.origin.x + 4.0, cell.origin.y + 2.0, cell.size.width - 8.0, cell.size.height - 4.0);
                NSColor::selectedContentBackgroundColor().colorWithAlphaComponent(0.25).setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(hl, 8.0, 8.0).fill();
            }
            item.icon.drawInRect(icon_rect(i, cols));
            let name = item.path.file_name().unwrap_or_default().to_string_lossy();
            let label = rect(cell.origin.x + 4.0, cell.origin.y + ICON + 12.0, cell.size.width - 8.0, 30.0);
            text_in(&name, label, &attrs_wrapped(&font, &NSColor::labelColor(), Some(NSTextAlignment::Center)));
        }
    }

    pub fn add(&self, paths: &[PathBuf]) {
        let ws = NSWorkspace::sharedWorkspace();
        let mut s = self.ivars().borrow_mut();
        for p in paths {
            if s.items.iter().any(|it| &it.path == p) {
                continue;
            }
            let icon = ws.iconForFile(&NSString::from_str(&p.to_string_lossy()));
            s.items.push(FloatItem { path: p.clone(), icon });
        }
        drop(s);
        self.setNeedsDisplay(true);
    }

    pub fn remove_selected(&self) {
        let mut s = self.ivars().borrow_mut();
        let selected: Vec<usize> = s.selected.iter().rev().copied().collect();
        for i in selected {
            if i < s.items.len() {
                s.items.remove(i);
            }
        }
        s.selected.clear();
        drop(s);
        self.setNeedsDisplay(true);
    }

    pub fn len(&self) -> usize {
        self.ivars().borrow().items.len()
    }
}

pub struct FloatWindow {
    pub panel: Retained<NSPanel>,
    pub view: Retained<FloatView>,
}

impl FloatWindow {
    pub fn new(mtm: MainThreadMarker, delegate: &ProtocolObject<dyn NSWindowDelegate>) -> Self {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::UtilityWindow
            | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect(0.0, 0.0, 200.0, 120.0),
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setTitle(&NSString::from_str("Float"));
        panel.setFloatingPanel(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setDelegate(Some(delegate));
        let view = FloatView::new(mtm);
        panel.setContentView(Some(&view));
        FloatWindow { panel, view }
    }

    pub fn window_number(&self) -> isize {
        self.panel.windowNumber()
    }

    /// Adds files and resizes the shelf to fit, keeping its top-left corner in place.
    pub fn add(&self, paths: &[PathBuf]) {
        self.view.add(paths);
        self.fit();
    }

    pub fn fit(&self) {
        let size = content_size(self.view.len());
        let frame = self.panel.frame();
        let content = self.panel.contentRectForFrameRect(frame);
        let top = content.origin.y + content.size.height;
        let new_content = rect(content.origin.x, top - size.height, size.width, size.height);
        let new_frame = self.panel.frameRectForContentRect(new_content);
        self.panel.setFrame_display(new_frame, true);
    }

    pub fn show_near(&self, p: NSPoint) {
        let size = content_size(self.view.len());
        let frame = self.panel.frameRectForContentRect(rect(p.x + 40.0, p.y - size.height / 2.0, size.width, size.height));
        self.panel.setFrame_display(frame, true);
        self.panel.orderFrontRegardless();
    }
}
