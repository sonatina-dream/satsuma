//! In-process image editing with ImageIO and CoreGraphics: orientation-correct loading,
//! cropping, Add Background compositing, and writing.

use crate::{Error, Result};
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFURL, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGColor, CGColorSpace,
    CGContext, CGGradient, CGGradientDrawingOptions, CGImage, CGImageAlphaInfo,
    CGInterpolationQuality, CGPath,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_image_io::{
    kCGImageDestinationLossyCompressionQuality, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceThumbnailMaxPixelSize,
    CGImageDestination, CGImageSource,
};
use satsuma_core::edit::{frame_layout, Background, FrameSpec, Rect, GRADIENTS};
use satsuma_core::Format;
use std::path::Path;

pub type Image = CFRetained<CGImage>;

fn key(k: &CFString) -> &NSString {
    unsafe { &*(k as *const CFString as *const NSString) }
}

/// Toll-free bridged NSDictionary → CFDictionary.
fn cf_dict(d: &NSDictionary<NSString, AnyObject>) -> &CFDictionary {
    unsafe { &*(d as *const NSDictionary<NSString, AnyObject> as *const CFDictionary) }
}

fn name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

pub fn size(img: &CGImage) -> (f64, f64) {
    (CGImage::width(Some(img)) as f64, CGImage::height(Some(img)) as f64)
}

/// Loads the first image of a file, upright (EXIF orientation applied), at full size.
pub fn load(path: &Path) -> Result<Image> {
    autoreleasepool(|_| {
        let url = CFURL::from_file_path(path).ok_or_else(|| Error::msg("Invalid image path."))?;
        let source = unsafe { CGImageSource::with_url(&url, None) }
            .ok_or_else(|| Error::msg(format!("Could not read the image {}.", name(path))))?;
        let raw = unsafe { source.image_at_index(0, None) }
            .ok_or_else(|| Error::msg(format!("Could not read the image {}.", name(path))))?;
        let longest = size(&raw).0.max(size(&raw).1);
        let yes = NSNumber::new_bool(true);
        let max = NSNumber::new_f64(longest);
        let keys = unsafe {
            [
                key(kCGImageSourceCreateThumbnailFromImageAlways),
                key(kCGImageSourceCreateThumbnailWithTransform),
                key(kCGImageSourceThumbnailMaxPixelSize),
            ]
        };
        let values: [&AnyObject; 3] = [&yes, &yes, &max];
        let options = NSDictionary::from_slices(&keys, &values);
        Ok(unsafe { source.thumbnail_at_index(0, Some(cf_dict(&options))) }.unwrap_or(raw))
    })
}

fn context(w: usize, h: usize) -> Result<CFRetained<CGContext>> {
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
    unsafe {
        CGBitmapContextCreate(std::ptr::null_mut(), w.max(1), h.max(1), 8, 0, space.as_deref(), CGImageAlphaInfo::PremultipliedLast.0)
    }
    .ok_or_else(|| Error::msg("Could not create the image canvas."))
}

fn finish(ctx: &CGContext) -> Result<Image> {
    CGBitmapContextCreateImage(Some(ctx)).ok_or_else(|| Error::msg("Could not render the image."))
}

/// Crops `rect` (pixels, top-left origin) and optionally resizes to `output`.
pub fn crop(img: &CGImage, rect: Rect, output: Option<(u32, u32)>) -> Result<Image> {
    let r = CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h));
    let cropped = CGImage::with_image_in_rect(Some(img), r).ok_or_else(|| Error::msg("Could not create the cropped image."))?;
    match output {
        Some((w, h)) if (w as f64, h as f64) != size(&cropped) => {
            let ctx = context(w as usize, h as usize)?;
            CGContext::set_interpolation_quality(Some(&ctx), CGInterpolationQuality::High);
            CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)), Some(&cropped));
            finish(&ctx)
        }
        _ => Ok(cropped),
    }
}

fn rgb(c: (u8, u8, u8)) -> [f64; 3] {
    [c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0]
}

/// Composites the photo onto a background. `scale` shrinks everything for live previews.
pub fn frame(img: &CGImage, spec: &FrameSpec, background: Option<&CGImage>, scale: f64) -> Result<Image> {
    let (iw, ih) = size(img);
    let (iw, ih) = ((iw * scale).round().max(1.0), (ih * scale).round().max(1.0));
    let ((cw, ch), place) = frame_layout((iw, ih), spec.padding * scale, spec.ratio);
    let ctx = context(cw as usize, ch as usize)?;
    let canvas = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(cw, ch));
    CGContext::set_interpolation_quality(Some(&ctx), CGInterpolationQuality::High);

    match (&spec.background, background) {
        (Background::Image(_), Some(bg)) => {
            let (bw, bh) = size(bg);
            let s = (cw / bw).max(ch / bh);
            let (dw, dh) = (bw * s, bh * s);
            let r = CGRect::new(CGPoint::new((cw - dw) / 2.0, (ch - dh) / 2.0), CGSize::new(dw, dh));
            CGContext::draw_image(Some(&ctx), r, Some(bg));
        }
        (Background::Solid { r, g, b }, _) => {
            let [r, g, b] = rgb((*r, *g, *b));
            CGContext::set_fill_color_with_color(Some(&ctx), Some(&CGColor::new_srgb(r, g, b, 1.0)));
            CGContext::fill_rect(Some(&ctx), canvas);
        }
        (bg, _) => {
            let preset = match bg {
                Background::Gradient(i) => GRADIENTS[(*i).min(GRADIENTS.len() - 1)],
                _ => GRADIENTS[0],
            };
            let mut components = Vec::with_capacity(12);
            for stop in preset.stops {
                components.extend(rgb(stop));
                components.push(1.0);
            }
            let locations = [0.0, 0.5, 1.0];
            let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
            let gradient = unsafe { CGGradient::with_color_components(space.as_deref(), components.as_ptr(), locations.as_ptr(), 3) }
                .ok_or_else(|| Error::msg("Could not prepare the image background."))?;
            // Top-left to bottom-right (CoreGraphics has y pointing up).
            CGContext::draw_linear_gradient(
                Some(&ctx),
                Some(&gradient),
                CGPoint::new(0.0, ch),
                CGPoint::new(cw, 0.0),
                CGGradientDrawingOptions::DrawsBeforeStartLocation | CGGradientDrawingOptions::DrawsAfterEndLocation,
            );
        }
    }

    let photo = CGRect::new(CGPoint::new(place.x, ch - place.y - place.h), CGSize::new(place.w, place.h));
    let radius = (spec.corners * scale).clamp(0.0, place.w.min(place.h) / 2.0 - 0.5).max(0.0);
    let path = unsafe { CGPath::with_rounded_rect(photo, radius, radius, std::ptr::null()) };
    let shadow = spec.shadow * scale;
    if shadow > 0.0 {
        CGContext::save_g_state(Some(&ctx));
        let shade = CGColor::new_srgb(0.0, 0.0, 0.0, 0.35);
        CGContext::set_shadow_with_color(Some(&ctx), CGSize::new(0.0, -shadow * 0.35), shadow, Some(&shade));
        CGContext::add_path(Some(&ctx), Some(&path));
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&CGColor::new_srgb(1.0, 1.0, 1.0, 1.0)));
        CGContext::fill_path(Some(&ctx));
        CGContext::restore_g_state(Some(&ctx));
    }
    CGContext::save_g_state(Some(&ctx));
    CGContext::add_path(Some(&ctx), Some(&path));
    CGContext::clip(Some(&ctx));
    CGContext::draw_image(Some(&ctx), photo, Some(img));
    CGContext::restore_g_state(Some(&ctx));
    finish(&ctx)
}

/// ImageIO type identifier for a format it can write.
pub fn uti(format: Format) -> Option<&'static str> {
    Some(match format {
        Format::Jpg => "public.jpeg",
        Format::Png => "public.png",
        Format::Heic => "public.heic",
        Format::Tiff => "public.tiff",
        Format::Bmp => "com.microsoft.bmp",
        Format::Avif => "public.avif",
        Format::Gif => "com.compuserve.gif",
        _ => return None,
    })
}

/// Writes `img` in `format` (not WebP; see `run.rs`).
pub fn write(img: &CGImage, path: &Path, format: Format) -> Result<()> {
    let uti = uti(format).ok_or_else(|| Error::msg(format!("Cannot save {} images here.", format.label())))?;
    autoreleasepool(|_| {
        let url = CFURL::from_file_path(path).ok_or_else(|| Error::msg("Invalid output path."))?;
        let kind = CFString::from_static_str(uti);
        let dest = unsafe { CGImageDestination::with_url(&url, &kind, 1, None) }
            .ok_or_else(|| Error::msg("Could not save the image."))?;
        let quality = NSNumber::new_f64(0.92);
        let keys = [key(unsafe { kCGImageDestinationLossyCompressionQuality })];
        let values: [&AnyObject; 1] = [&quality];
        let props = NSDictionary::from_slices(&keys, &values);
        unsafe { dest.add_image(img, Some(cf_dict(&props))) };
        if unsafe { dest.finalize() } {
            Ok(())
        } else {
            Err(Error::msg("Could not save the image."))
        }
    })
}

/// PNG bytes of an image (for embedding).
pub fn png_bytes(img: &CGImage, tmp: &Path) -> Result<Vec<u8>> {
    let p = crate::image::unique_in(tmp, "embed.png");
    write(img, &p, Format::Png)?;
    Ok(std::fs::read(p)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use satsuma_core::edit::CropSpec;

    fn sample(dir: &Path, w: usize, h: usize) -> std::path::PathBuf {
        let ctx = context(w, h).unwrap();
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&CGColor::new_srgb(0.2, 0.5, 0.9, 1.0)));
        CGContext::fill_rect(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)));
        let img = finish(&ctx).unwrap();
        let p = dir.join("sample.png");
        write(&img, &p, Format::Png).unwrap();
        p
    }

    #[test]
    fn crop_and_resize() {
        let dir = tempfile::tempdir().unwrap();
        let img = load(&sample(dir.path(), 400, 300)).unwrap();
        assert_eq!(size(&img), (400.0, 300.0));
        let spec = CropSpec::centered(Some((1, 1)), (400.0, 300.0));
        let c = crop(&img, spec.pixels((400.0, 300.0)), None).unwrap();
        assert_eq!(size(&c), (300.0, 300.0));
        let r = crop(&img, Rect::new(0.0, 0.0, 200.0, 100.0), Some((50, 25))).unwrap();
        assert_eq!(size(&r), (50.0, 25.0));
    }

    #[test]
    fn frame_sizes_follow_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let img = load(&sample(dir.path(), 400, 200)).unwrap();
        let mut spec = FrameSpec::defaults_for((400.0, 200.0));
        spec.padding = 50.0;
        let out = frame(&img, &spec, None, 1.0).unwrap();
        assert_eq!(size(&out), (500.0, 300.0));
        spec.ratio = Some((1, 1));
        spec.background = Background::Solid { r: 255, g: 255, b: 255 };
        let out = frame(&img, &spec, None, 1.0).unwrap();
        assert_eq!(size(&out), (500.0, 500.0));
        let preview = frame(&img, &spec, None, 0.5).unwrap();
        assert_eq!(size(&preview), (250.0, 250.0));
        let p = dir.path().join("framed.png");
        write(&out, &p, Format::Png).unwrap();
        assert_eq!(size(&load(&p).unwrap()), (500.0, 500.0));
    }
}
