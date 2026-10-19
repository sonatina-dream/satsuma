//! Small drawing helpers shared by the wheel, HUD and Float views.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSImage, NSImageSymbolConfiguration, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawing, NSTextAlignment,
};
use objc2_foundation::{NSAttributedStringKey, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use satsuma_core::geometry::PathCmd;

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// The tangerine accent.
pub fn accent() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.43, 0.09, 1.0)
}

pub fn path_from(cmds: &[PathCmd], cx: f64, cy: f64) -> Retained<NSBezierPath> {
    let path = NSBezierPath::bezierPath();
    let p = |pt: satsuma_core::geometry::Point| NSPoint::new(cx + pt.x, cy + pt.y);
    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(pt) => path.moveToPoint(p(pt)),
            PathCmd::LineTo(pt) => path.lineToPoint(p(pt)),
            PathCmd::CubicTo { c1, c2, to } => path.curveToPoint_controlPoint1_controlPoint2(p(to), p(c1), p(c2)),
            PathCmd::Close => path.closePath(),
        }
    }
    path
}

pub type Attrs = Retained<NSDictionary<NSAttributedStringKey, AnyObject>>;

pub fn attrs(font: &NSFont, color: &NSColor, align: Option<NSTextAlignment>) -> Attrs {
    attrs_with(font, color, align, NSLineBreakMode::ByTruncatingMiddle)
}

/// Like [`attrs`], but wrapping words across lines (for `text_in`).
pub fn attrs_wrapped(font: &NSFont, color: &NSColor, align: Option<NSTextAlignment>) -> Attrs {
    attrs_with(font, color, align, NSLineBreakMode::ByWordWrapping)
}

fn attrs_with(font: &NSFont, color: &NSColor, align: Option<NSTextAlignment>, line_break: NSLineBreakMode) -> Attrs {
    let style = NSMutableParagraphStyle::new();
    style.setAlignment(align.unwrap_or(NSTextAlignment::Left));
    style.setLineBreakMode(line_break);
    let keys: [&NSAttributedStringKey; 3] =
        unsafe { [NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName] };
    let values: [&AnyObject; 3] = [font, color, &style];
    NSDictionary::from_slices(&keys, &values)
}

/// Draws one line of text centred on `center`.
pub fn text_centered(text: &str, center: NSPoint, attrs: &Attrs) {
    let s = NSString::from_str(text);
    let size = unsafe { s.sizeWithAttributes(Some(attrs)) };
    unsafe {
        s.drawAtPoint_withAttributes(
            NSPoint::new(center.x - size.width / 2.0, center.y - size.height / 2.0),
            Some(attrs),
        )
    };
}

pub fn text_in(text: &str, r: NSRect, attrs: &Attrs) {
    let s = NSString::from_str(text);
    unsafe { s.drawInRect_withAttributes(r, Some(attrs)) };
}

/// An SF Symbol tinted with `color`.
pub fn symbol(name: &str, size: f64, color: &NSColor) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let weight = unsafe { NSFontWeightSemibold };
    let base = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, weight);
    let tint = NSImageSymbolConfiguration::configurationWithHierarchicalColor(color);
    let config = base.configurationByApplyingConfiguration(&tint);
    image.imageWithSymbolConfiguration(&config)
}

/// Draws `image` at its natural size centred on `center`.
pub fn image_centered(image: &NSImage, center: NSPoint) {
    let s = image.size();
    image.drawInRect(rect(center.x - s.width / 2.0, center.y - s.height / 2.0, s.width, s.height));
}

/// A black shape image, used as a mask for visual-effect backgrounds.
pub fn mask_image(size: NSSize, shape: impl Fn(NSRect) -> Retained<NSBezierPath>) -> Retained<NSImage> {
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    #[allow(deprecated)]
    image.lockFocus();
    NSColor::blackColor().setFill();
    shape(rect(0.0, 0.0, size.width, size.height)).fill();
    #[allow(deprecated)]
    image.unlockFocus();
    image
}

