//! The app icon (a satsuma with a leaf on teal), drawn with AppKit so the repo needs no
//! binary art. `scripts/bundle.sh` renders it into an .icns.

use crate::draw::rect;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBezierPath, NSBitmapImageFileType, NSBitmapImageRep,
    NSCalibratedRGBColorSpace, NSColor, NSGradient, NSGraphicsContext,
};
use objc2_foundation::{NSAffineTransform, NSDictionary, NSPoint, NSSize};
use std::path::Path;

fn rgb(r: u8, g: u8, b: u8, a: f64) -> objc2::rc::Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a)
}

pub fn render_icon(px: usize, out: &Path) -> Result<(), String> {
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            px as isize,
            px as isize,
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
    rep.setSize(NSSize::new(1024.0, 1024.0));
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).ok_or("No graphics context.")?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    draw();
    NSGraphicsContext::restoreGraphicsState_class();
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
        .ok_or("Could not encode the PNG.")?;
    std::fs::write(out, data.to_vec()).map_err(|e| e.to_string())
}

/// Draws in a 1024×1024 canvas following the macOS icon grid (824 pt body, 100 pt margin).
fn draw() {
    // Body: rounded square with a deep teal gradient.
    let body = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(100.0, 100.0, 824.0, 824.0), 185.0, 185.0);
    let bg = NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &rgb(22, 92, 94, 1.0), &rgb(9, 46, 52, 1.0))
        .expect("gradient");
    bg.drawInBezierPath_angle(&body, -90.0);

    // Soft shadow under the fruit.
    rgb(0, 0, 0, 0.22).setFill();
    NSBezierPath::bezierPathWithOvalInRect(rect(262.0, 196.0, 500.0, 90.0)).fill();

    // The fruit: slightly squat, like a satsuma.
    let fruit = NSBezierPath::bezierPathWithOvalInRect(rect(232.0, 232.0, 560.0, 500.0));
    let peel = NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &rgb(255, 178, 64, 1.0), &rgb(244, 98, 18, 1.0))
        .expect("gradient");
    peel.drawInBezierPath_relativeCenterPosition(&fruit, NSPoint::new(-0.35, 0.4));

    // Peel dimples.
    rgb(255, 255, 255, 0.10).setFill();
    for (x, y, r) in [(380.0, 560.0, 9.0), (460.0, 610.0, 7.0), (560.0, 600.0, 8.0), (640.0, 520.0, 7.0), (420.0, 440.0, 8.0), (600.0, 400.0, 9.0), (500.0, 330.0, 7.0), (340.0, 380.0, 7.0)] {
        NSBezierPath::bezierPathWithOvalInRect(rect(x - r, y - r, r * 2.0, r * 2.0)).fill();
    }
    // Highlight.
    rgb(255, 255, 255, 0.35).setFill();
    NSBezierPath::bezierPathWithOvalInRect(rect(330.0, 560.0, 120.0, 70.0)).fill();

    // Stem.
    rgb(86, 60, 30, 1.0).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(500.0, 712.0, 24.0, 50.0), 10.0, 10.0).fill();

    // Leaf, rotated.
    let leaf = NSBezierPath::bezierPath();
    leaf.moveToPoint(NSPoint::new(0.0, 0.0));
    leaf.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(230.0, 0.0), NSPoint::new(60.0, 95.0), NSPoint::new(180.0, 95.0));
    leaf.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(0.0, 0.0), NSPoint::new(180.0, -80.0), NSPoint::new(60.0, -80.0));
    leaf.closePath();
    let t = NSAffineTransform::transform();
    t.translateXBy_yBy(520.0, 748.0);
    t.rotateByDegrees(24.0);
    leaf.transformUsingAffineTransform(&t);
    let green = NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &rgb(120, 200, 90, 1.0), &rgb(46, 140, 70, 1.0))
        .expect("gradient");
    green.drawInBezierPath_angle(&leaf, 90.0);
    let vein = NSBezierPath::bezierPath();
    vein.moveToPoint(NSPoint::new(10.0, 0.0));
    vein.lineToPoint(NSPoint::new(200.0, 0.0));
    vein.transformUsingAffineTransform(&t);
    rgb(255, 255, 255, 0.45).setStroke();
    vein.setLineWidth(6.0);
    vein.stroke();
}
