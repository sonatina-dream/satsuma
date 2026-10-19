//! Edit Photo: CoreImage adjustments (light, colour, detail, effects). Tangerine uses the
//! same framework (E11: CIExposureAdjust, CIHighlightShadowAdjust, CITemperatureAndTint,
//! CINoiseReduction, CISharpenLuminance, CIRandomGenerator + CISoftLightBlendMode); the
//! filter mappings and strengths here are Satsuma's own.

use crate::imaging::{self, Image};
use crate::{Error, Result};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGRect};
use objc2_core_graphics::CGImage;
use objc2_core_image::{CIContext, CIImage, CIVector};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use satsuma_core::edit::Adjustments;
use std::ptr::NonNull;

thread_local! {
    static CONTEXT: Retained<CIContext> = unsafe { CIContext::contextWithOptions(None) };
}

enum Param {
    Num(f64),
    Vec2(f64, f64),
    Vec4(f64, f64, f64, f64),
    Image(Retained<CIImage>),
}

fn apply_filter(img: &CIImage, name: &str, params: &[(&str, Param)]) -> Retained<CIImage> {
    let keys: Vec<Retained<NSString>> = params.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let values: Vec<Retained<AnyObject>> = params
        .iter()
        .map(|(_, v)| -> Retained<AnyObject> {
            match v {
                Param::Num(n) => Retained::into_super(Retained::into_super(Retained::into_super(NSNumber::new_f64(*n)))),
                Param::Vec2(x, y) => Retained::into_super(Retained::into_super(unsafe { CIVector::vectorWithX_Y(*x, *y) })),
                Param::Vec4(x, y, z, w) => {
                    Retained::into_super(Retained::into_super(unsafe { CIVector::vectorWithX_Y_Z_W(*x, *y, *z, *w) }))
                }
                Param::Image(i) => Retained::into_super(Retained::into_super(i.clone())),
            }
        })
        .collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let value_refs: Vec<&AnyObject> = values.iter().map(|v| &**v).collect();
    let dict = NSDictionary::from_slices(&key_refs, &value_refs);
    unsafe { img.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &dict) }
}

/// Applies `a` to `img` and renders the result at the same size.
pub fn apply(img: &CGImage, a: &Adjustments) -> Result<Image> {
    if a.is_identity() {
        return Ok(unsafe { CFRetained::retain(NonNull::from(img)) });
    }
    autoreleasepool(|_| {
        let (w, h) = imaging::size(img);
        let long = w.max(h);
        let source = unsafe { CIImage::imageWithCGImage(img) };
        let extent: CGRect = unsafe { source.extent() };
        let v = |k: &str| a.get(k) / 100.0;
        let mut ci = source.clone();

        if v("exposure") != 0.0 {
            ci = apply_filter(&ci, "CIExposureAdjust", &[("inputEV", Param::Num(v("exposure") * 2.0))]);
        }

        // Tone curve for highlights, shadows, whites, blacks and fade.
        let (hi, sh, wh, bl, fade) = (v("highlights"), v("shadows"), v("whites"), v("blacks"), v("fade"));
        if hi != 0.0 || sh != 0.0 || wh != 0.0 || bl != 0.0 || fade != 0.0 {
            let p0 = ((-bl).max(0.0) * 0.12, bl.max(0.0) * 0.12 + fade * 0.15);
            let p1 = (0.25, (0.25 + sh * 0.12 + fade * 0.08).clamp(0.0, 1.0));
            let p2 = (0.5, (0.5 + (sh + hi) * 0.03).clamp(0.0, 1.0));
            let p3 = (0.75, (0.75 + hi * 0.12).clamp(0.0, 1.0));
            let p4 = (1.0 - wh.max(0.0) * 0.12, 1.0 - (-wh).max(0.0) * 0.12 - fade * 0.04);
            ci = apply_filter(
                &ci,
                "CIToneCurve",
                &[
                    ("inputPoint0", Param::Vec2(p0.0, p0.1)),
                    ("inputPoint1", Param::Vec2(p1.0, p1.1)),
                    ("inputPoint2", Param::Vec2(p2.0, p2.1)),
                    ("inputPoint3", Param::Vec2(p3.0, p3.1)),
                    ("inputPoint4", Param::Vec2(p4.0, p4.1)),
                ],
            );
        }

        let (br, co, sa, dh) = (v("brightness"), v("contrast"), v("saturation"), v("dehaze"));
        if br != 0.0 || co != 0.0 || sa != 0.0 || dh != 0.0 {
            ci = apply_filter(
                &ci,
                "CIColorControls",
                &[
                    ("inputBrightness", Param::Num(br * 0.2 - dh * 0.03)),
                    ("inputContrast", Param::Num((1.0 + co * 0.4 + dh * 0.25).max(0.25))),
                    ("inputSaturation", Param::Num((1.0 + sa + dh * 0.15).max(0.0))),
                ],
            );
        }

        if v("gamma") != 0.0 {
            ci = apply_filter(&ci, "CIGammaAdjust", &[("inputPower", Param::Num(1.0 - v("gamma") * 0.5))]);
        }

        let (temp, tint) = (v("temperature"), v("tint"));
        if temp != 0.0 || tint != 0.0 {
            // Lowering the target white point warms; a negative target tint leans magenta (both
            // verified by the tests below).
            ci = apply_filter(
                &ci,
                "CITemperatureAndTint",
                &[
                    ("inputNeutral", Param::Vec2(6500.0, 0.0)),
                    ("inputTargetNeutral", Param::Vec2(6500.0 - temp * 3000.0, -tint * 60.0)),
                ],
            );
        }

        if v("vibrance") != 0.0 {
            ci = apply_filter(&ci, "CIVibrance", &[("inputAmount", Param::Num(v("vibrance")))]);
        }

        // Local contrast: clarity (wide radius) and texture (fine radius).
        for (key, radius, strength) in [("clarity", 0.02, 0.6), ("texture", 0.004, 0.9)] {
            if v(key) > 0.0 {
                let clamped = unsafe { ci.imageByClampingToExtent() };
                ci = apply_filter(
                    &clamped,
                    "CIUnsharpMask",
                    &[("inputRadius", Param::Num((long * radius).max(1.0))), ("inputIntensity", Param::Num(v(key) * strength))],
                );
                ci = unsafe { ci.imageByCroppingToRect(extent) };
            }
        }

        if v("noiseReduction") > 0.0 {
            ci = apply_filter(
                &ci,
                "CINoiseReduction",
                &[("inputNoiseLevel", Param::Num(v("noiseReduction") * 0.06)), ("inputSharpness", Param::Num(0.4))],
            );
        }

        if v("sharpness") > 0.0 {
            ci = apply_filter(
                &ci,
                "CISharpenLuminance",
                &[("inputSharpness", Param::Num(v("sharpness") * 1.2)), ("inputRadius", Param::Num((long / 1000.0 * 1.5).max(1.0)))],
            );
        }

        if v("vignette") != 0.0 {
            let diag = (w * w + h * h).sqrt();
            ci = apply_filter(
                &ci,
                "CIVignetteEffect",
                &[
                    ("inputCenter", Param::Vec2(extent.origin.x + w / 2.0, extent.origin.y + h / 2.0)),
                    ("inputRadius", Param::Num(diag * 0.42)),
                    ("inputIntensity", Param::Num(v("vignette"))),
                    ("inputFalloff", Param::Num(0.6)),
                ],
            );
        }

        if v("grain") > 0.0 {
            // Monochrome noise, scaled with the photo so previews and full size look alike.
            let noise = unsafe { CIImage::imageWithCGImage(img) };
            let random = apply_filter(&noise, "CIRandomGenerator", &[]);
            let s = (long / 1500.0).max(1.0);
            let random = unsafe { random.imageByApplyingTransform(CGAffineTransform { a: s, b: 0.0, c: 0.0, d: s, tx: 0.0, ty: 0.0 }) };
            let alpha = v("grain") * 0.35;
            let mono = apply_filter(
                &random,
                "CIColorMatrix",
                &[
                    ("inputRVector", Param::Vec4(0.0, 1.0, 0.0, 0.0)),
                    ("inputGVector", Param::Vec4(0.0, 1.0, 0.0, 0.0)),
                    ("inputBVector", Param::Vec4(0.0, 1.0, 0.0, 0.0)),
                    ("inputAVector", Param::Vec4(0.0, 0.0, 0.0, 0.0)),
                    ("inputBiasVector", Param::Vec4(0.0, 0.0, 0.0, alpha)),
                ],
            );
            let mono = unsafe { mono.imageByCroppingToRect(extent) };
            ci = apply_filter(&mono, "CISoftLightBlendMode", &[("inputBackgroundImage", Param::Image(ci.clone()))]);
        }

        let ci = unsafe { ci.imageByCroppingToRect(extent) };
        let rendered = CONTEXT.with(|ctx| unsafe { ctx.createCGImage_fromRect(&ci, extent) });
        let rendered = rendered.ok_or_else(|| Error::msg("Could not render the photo adjustments."))?;
        Ok(unsafe { CFRetained::retain(NonNull::from(&*rendered)) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_foundation::{CGPoint, CGSize};
    use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextCreateImage, CGColor, CGColorSpace, CGContext, CGImageAlphaInfo, kCGColorSpaceSRGB};

    /// A flat grey image.
    fn grey(level: f64) -> Image {
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), 64, 64, 8, 0, space.as_deref(), CGImageAlphaInfo::PremultipliedLast.0) }.unwrap();
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&CGColor::new_srgb(level, level, level, 1.0)));
        CGContext::fill_rect(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(64.0, 64.0)));
        CGBitmapContextCreateImage(Some(&ctx)).unwrap()
    }

    /// Mean RGB of an image (rendered into an sRGB RGBA buffer).
    fn mean(img: &CGImage) -> (f64, f64, f64) {
        let (w, h) = imaging::size(img);
        let (w, h) = (w as usize, h as usize);
        let mut buf = vec![0u8; w * h * 4];
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        let ctx = unsafe { CGBitmapContextCreate(buf.as_mut_ptr().cast(), w, h, 8, w * 4, space.as_deref(), CGImageAlphaInfo::PremultipliedLast.0) }.unwrap();
        CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)), Some(img));
        drop(ctx);
        let n = (w * h) as f64;
        let sum = |c: usize| buf.chunks(4).map(|p| p[c] as f64).sum::<f64>() / n;
        (sum(0), sum(1), sum(2))
    }

    fn with(key: &str, value: f64) -> Adjustments {
        let mut a = Adjustments::default();
        a.set(Adjustments::index(key).unwrap(), value);
        a
    }

    #[test]
    fn identity_returns_the_same_pixels() {
        let img = grey(0.5);
        let out = apply(&img, &Adjustments::default()).unwrap();
        assert_eq!(mean(&out), mean(&img));
    }

    #[test]
    fn light_adjustments_move_brightness_the_right_way() {
        let img = grey(0.5);
        let base = mean(&img).0;
        for key in ["exposure", "brightness", "shadows", "highlights", "gamma"] {
            let up = mean(&apply(&img, &with(key, 60.0)).unwrap()).0;
            let down = mean(&apply(&img, &with(key, -60.0)).unwrap()).0;
            assert!(up > base + 2.0, "{key} +: {up} vs {base}");
            assert!(down < base - 2.0, "{key} -: {down} vs {base}");
        }
        assert!(mean(&apply(&grey(0.05), &with("fade", 80.0)).unwrap()).0 > mean(&grey(0.05)).0 + 5.0);
    }

    #[test]
    fn temperature_warms_and_cools() {
        let img = grey(0.5);
        let (r, _, b) = mean(&apply(&img, &with("temperature", 60.0)).unwrap());
        assert!(r > b + 3.0, "warm: r={r} b={b}");
        let (r, _, b) = mean(&apply(&img, &with("temperature", -60.0)).unwrap());
        assert!(b > r + 3.0, "cool: r={r} b={b}");
        // Positive tint leans magenta (less green), negative leans green.
        let (r, g, b) = mean(&apply(&img, &with("tint", 60.0)).unwrap());
        let (r2, g2, b2) = mean(&apply(&img, &with("tint", -60.0)).unwrap());
        assert!(g < (r + b) / 2.0 && g2 > (r2 + b2) / 2.0, "tint+ {r},{g},{b} tint- {r2},{g2},{b2}");
    }

    #[test]
    fn effects_render_at_full_size() {
        let img = grey(0.5);
        let mut a = Adjustments::default();
        for key in ["clarity", "texture", "sharpness", "noiseReduction", "vignette", "grain", "dehaze", "vibrance", "tint", "saturation", "contrast", "whites", "blacks"] {
            a.set(Adjustments::index(key).unwrap(), 50.0);
        }
        let out = apply(&img, &a).unwrap();
        assert_eq!(imaging::size(&out), (64.0, 64.0));
        // Vignette darkens corners relative to the centre.
        let v = apply(&img, &with("vignette", 100.0)).unwrap();
        assert!(mean(&v).0 < mean(&img).0);
    }
}
