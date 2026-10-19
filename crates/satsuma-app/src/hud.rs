//! Progress HUD: "Converting to MP3 · 1 of 1 saved" with a bar and a close/cancel button.

use crate::draw::{accent, attrs, attrs_wrapped, mask_image, rect, text_centered, text_in};
use crate::events::{post, UiEvent};
use crate::system::screen_frame_for;
use crate::wheel::KeyPanel;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSEvent, NSFont, NSFontWeightRegular, NSFontWeightSemibold, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::cell::RefCell;

pub const WIDTH: f64 = 300.0;
pub const HEIGHT: f64 = 62.0;

#[derive(Debug, Clone, Default)]
pub struct HudModel {
    pub title: String,
    pub subtitle: String,
    /// 0…1, or `None` for no bar.
    pub fraction: Option<f64>,
    pub error: bool,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatsumaHudView"]
    #[ivars = RefCell<HudModel>]
    pub struct HudView;

    impl HudView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let (cx, cy) = close_center();
            if (p.x - cx).hypot(p.y - cy) <= 12.0 {
                post(UiEvent::HudClose);
            }
        }
    }

    unsafe impl NSObjectProtocol for HudView {}
);

fn close_center() -> (f64, f64) {
    (20.0, HEIGHT - 20.0)
}

impl HudView {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(HudModel::default()));
        unsafe { msg_send![super(this), initWithFrame: rect(0.0, 0.0, WIDTH, HEIGHT)] }
    }

    fn draw(&self) {
        let m = self.ivars().borrow();
        let (cx, cy) = close_center();
        let close = NSBezierPath::bezierPathWithOvalInRect(rect(cx - 9.0, cy - 9.0, 18.0, 18.0));
        NSColor::tertiaryLabelColor().colorWithAlphaComponent(0.35).setFill();
        close.fill();
        let small = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
        text_centered("×", NSPoint::new(cx, cy + 0.5), &attrs(&small, &NSColor::secondaryLabelColor(), None));

        let title_font = NSFont::systemFontOfSize_weight(13.0, unsafe { NSFontWeightSemibold });
        let title_color = if m.error { NSColor::systemRedColor() } else { NSColor::labelColor() };
        text_in(&m.title, rect(38.0, HEIGHT - 29.0, WIDTH - 52.0, 18.0), &attrs(&title_font, &title_color, None));
        let sub_font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightRegular });
        let sub_height = if m.fraction.is_some() { 15.0 } else { 30.0 };
        text_in(
            &m.subtitle,
            rect(14.0, HEIGHT - 46.0 - (sub_height - 15.0), WIDTH - 28.0, sub_height),
            &attrs_wrapped(&sub_font, &NSColor::secondaryLabelColor(), None),
        );
        if let Some(f) = m.fraction {
            let track = rect(14.0, 9.0, WIDTH - 28.0, 4.0);
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(track, 2.0, 2.0);
            NSColor::tertiaryLabelColor().colorWithAlphaComponent(0.25).setFill();
            path.fill();
            let fill = rect(track.origin.x, track.origin.y, (track.size.width * f.clamp(0.0, 1.0)).max(4.0), 4.0);
            accent().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill, 2.0, 2.0).fill();
        }
    }
}

pub struct Hud {
    panel: Retained<KeyPanel>,
    view: Retained<HudView>,
}

impl Hud {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let frame = rect(0.0, 0.0, WIDTH, HEIGHT);
        let panel = KeyPanel::new(mtm, frame, NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setLevel(101);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        effect.setMaterial(NSVisualEffectMaterial::Popover);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        effect.setMaskImage(Some(&mask_image(NSSize::new(WIDTH, HEIGHT), |b| {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(b, 14.0, 14.0)
        })));
        let view = HudView::new(mtm);
        effect.addSubview(&view);
        panel.setContentView(Some(&effect));
        Hud { panel, view }
    }

    /// Shows the HUD below `near` (screen coordinates), kept on screen.
    pub fn show(&self, mtm: MainThreadMarker, near: NSPoint, model: HudModel) {
        let screen = screen_frame_for(near, mtm);
        let mut x = near.x - WIDTH / 2.0;
        let mut y = near.y - 150.0 - HEIGHT;
        if y < screen.origin.y + 8.0 {
            y = near.y + 150.0;
        }
        x = x.clamp(screen.origin.x + 8.0, screen.origin.x + screen.size.width - WIDTH - 8.0);
        y = y.clamp(screen.origin.y + 8.0, screen.origin.y + screen.size.height - HEIGHT - 8.0);
        self.panel.setFrameOrigin(NSPoint::new(x, y));
        self.update(model);
        self.panel.orderFrontRegardless();
    }

    pub fn update(&self, model: HudModel) {
        *self.view.ivars().borrow_mut() = model;
        self.view.setNeedsDisplay(true);
    }

    pub fn hide(&self) {
        self.panel.orderOut(None);
    }
}
