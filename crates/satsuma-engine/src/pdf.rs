//! PDF work with PDFKit (merge, split, text, metadata, compression), CoreGraphics (page
//! rendering) and CoreText (paginating rich text into a PDF).

use crate::{Error, Result};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{NSAttributedStringAppKitDocumentFormats, NSColor, NSForegroundColorAttributeName};
use objc2_core_foundation::{CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CFURL, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
    CGPDFBox, CGPDFContextBeginPage, CGPDFContextClose, CGPDFContextCreateWithURL,
    CGPDFContextEndPage, CGPDFDocument, CGPDFPage, CGPath, kCGColorSpaceSRGB,
};
use objc2_core_text::{kCTForegroundColorAttributeName, CTFramesetter};
use objc2_foundation::{
    NSAttributedString, NSCopying, NSData, NSDictionary, NSMutableAttributedString, NSNumber,
    NSRange, NSString, NSURL,
};
use objc2_image_io::{kCGImageDestinationLossyCompressionQuality, kCGImagePropertyDPIHeight, kCGImagePropertyDPIWidth, CGImageDestination};
use objc2_pdf_kit::{
    PDFDocument, PDFDocumentOptimizeImagesForScreenOption, PDFDocumentSaveImagesAsJPEGOption,
};
use satsuma_core::Format;
use std::path::{Path, PathBuf};

fn display_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn ns_url(path: &Path) -> Result<Retained<NSURL>> {
    NSURL::from_file_path(path).ok_or_else(|| Error::msg(format!("Invalid path {}", path.display())))
}

fn cf_url(path: &Path) -> Result<CFRetained<CFURL>> {
    CFURL::from_file_path(path).ok_or_else(|| Error::msg(format!("Invalid path {}", path.display())))
}

/// Toll-free bridged CFString → NSString.
fn ns_key(key: &CFString) -> &NSString {
    unsafe { &*(key as *const CFString as *const NSString) }
}

fn open(path: &Path) -> Result<Retained<PDFDocument>> {
    let url = ns_url(path)?;
    let doc = unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }
        .ok_or_else(|| Error::msg(format!("Could not read the PDF {}.", display_name(path))))?;
    if unsafe { doc.isLocked() } {
        return Err(Error::msg(format!("{} is password protected.", display_name(path))));
    }
    Ok(doc)
}

fn write(doc: &PDFDocument, out: &Path) -> Result<()> {
    let url = ns_url(out)?;
    if unsafe { doc.writeToURL(&url) } {
        Ok(())
    } else {
        Err(Error::msg("Could not create the PDF."))
    }
}

pub fn page_count(path: &Path) -> Result<usize> {
    autoreleasepool(|_| Ok(unsafe { open(path)?.pageCount() }))
}

/// Selectable text of the whole document.
pub fn text(path: &Path) -> Result<String> {
    autoreleasepool(|_| {
        let doc = open(path)?;
        let n = unsafe { doc.pageCount() };
        let mut pages = Vec::new();
        for i in 0..n {
            if let Some(page) = unsafe { doc.pageAtIndex(i) } {
                if let Some(s) = unsafe { page.string() } {
                    pages.push(s.to_string());
                }
            }
        }
        Ok(pages.join("\n\n"))
    })
}

/// Combine PDFs in order.
pub fn merge(inputs: &[PathBuf], out: &Path) -> Result<()> {
    autoreleasepool(|_| {
        let merged = unsafe { PDFDocument::new() };
        for input in inputs {
            let doc = open(input)?;
            for i in 0..unsafe { doc.pageCount() } {
                if let Some(page) = unsafe { doc.pageAtIndex(i) } {
                    let copy = page.copy();
                    unsafe { merged.insertPage_atIndex(&copy, merged.pageCount()) };
                }
            }
        }
        if unsafe { merged.pageCount() } == 0 {
            return Err(Error::msg("The selected PDFs have no pages."));
        }
        write(&merged, out)
    })
}

/// One PDF per page in `out_dir`, named `<stem> 01.pdf`, `<stem> 02.pdf`, …
pub fn split(input: &Path, out_dir: &Path, stem: &str, progress: &mut dyn FnMut(f64)) -> Result<Vec<PathBuf>> {
    autoreleasepool(|_| {
        let doc = open(input)?;
        let n = unsafe { doc.pageCount() };
        if n == 0 {
            return Err(Error::msg("This PDF has no pages."));
        }
        std::fs::create_dir_all(out_dir)?;
        let width = n.to_string().len().max(2);
        let mut outputs = Vec::new();
        for i in 0..n {
            let single = unsafe { PDFDocument::new() };
            if let Some(page) = unsafe { doc.pageAtIndex(i) } {
                unsafe { single.insertPage_atIndex(&page.copy(), 0) };
            }
            let out = out_dir.join(format!("{stem} {:0width$}.pdf", i + 1));
            write(&single, &out)?;
            outputs.push(out);
            progress((i + 1) as f64 / n as f64);
        }
        Ok(outputs)
    })
}

/// Drops the document info dictionary (author, title, creator, keywords …).
pub fn strip_metadata(input: &Path, out: &Path) -> Result<()> {
    autoreleasepool(|_| {
        let doc = open(input)?;
        let empty = NSDictionary::<AnyObject, AnyObject>::new();
        unsafe { doc.setDocumentAttributes(Some(&empty)) };
        write(&doc, out)
    })
}

/// Recompresses images as JPEG (and downsamples them for screens with `strong`).
pub fn compress(input: &Path, out: &Path, strong: bool) -> Result<()> {
    autoreleasepool(|_| {
        let doc = open(input)?;
        let yes = NSNumber::new_bool(true);
        let no = NSNumber::new_bool(false);
        let keys: [&NSString; 2] = unsafe {
            [PDFDocumentSaveImagesAsJPEGOption, PDFDocumentOptimizeImagesForScreenOption]
        };
        let values: [&AnyObject; 2] = [&yes, if strong { &yes } else { &no }];
        let options = NSDictionary::from_slices(&keys, &values);
        let url = ns_url(out)?;
        if unsafe { doc.writeToURL_withOptions(&url, Some(&options)) } {
            Ok(())
        } else {
            Err(Error::msg("Could not create the compressed PDF."))
        }
    })
}

/// Renders every page to JPG/PNG at `dpi`. `name_for(page_index, page_count)` picks each path.
pub fn render_pages(
    input: &Path,
    format: Format,
    dpi: f64,
    name_for: &dyn Fn(usize, usize) -> PathBuf,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<PathBuf>> {
    let uti = match format {
        Format::Png => "public.png",
        Format::Jpg => "public.jpeg",
        other => return Err(Error::msg(format!("Cannot render PDF pages as {}.", other.label()))),
    };
    autoreleasepool(|_| {
        let url = cf_url(input)?;
        let doc = CGPDFDocument::with_url(Some(&url))
            .ok_or_else(|| Error::msg(format!("Could not read the PDF {}.", display_name(input))))?;
        let n = CGPDFDocument::number_of_pages(Some(&doc));
        if n == 0 {
            return Err(Error::msg("This PDF has no pages."));
        }
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        let mut outputs = Vec::new();
        for index in 0..n {
            let page = CGPDFDocument::page(Some(&doc), index + 1)
                .ok_or_else(|| Error::msg(format!("Could not read PDF page {}.", index + 1)))?;
            let crop = CGPDFPage::box_rect(Some(&page), CGPDFBox::CropBox);
            let rotated = CGPDFPage::rotation_angle(Some(&page)).rem_euclid(180) != 0;
            let (w_pt, h_pt) = if rotated {
                (crop.size.height, crop.size.width)
            } else {
                (crop.size.width, crop.size.height)
            };
            // Keep huge pages within a sane bitmap size.
            let scale = (dpi / 72.0).min(16000.0 / w_pt.max(h_pt).max(1.0));
            let (pw, ph) = ((w_pt * scale).ceil() as usize, (h_pt * scale).ceil() as usize);
            let ctx = unsafe {
                CGBitmapContextCreate(
                    std::ptr::null_mut(),
                    pw.max(1),
                    ph.max(1),
                    8,
                    0,
                    space.as_deref(),
                    CGImageAlphaInfo::PremultipliedLast.0,
                )
            }
            .ok_or_else(|| Error::msg("Could not create the PDF page image."))?;
            let full = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(pw as f64, ph as f64));
            CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);
            CGContext::fill_rect(Some(&ctx), full);
            CGContext::scale_ctm(Some(&ctx), scale, scale);
            let target = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w_pt, h_pt));
            let transform = CGPDFPage::drawing_transform(Some(&page), CGPDFBox::CropBox, target, 0, true);
            CGContext::concat_ctm(Some(&ctx), transform);
            CGContext::clip_to_rect(Some(&ctx), crop);
            CGContext::draw_pdf_page(Some(&ctx), Some(&page));
            let image = CGBitmapContextCreateImage(Some(&ctx))
                .ok_or_else(|| Error::msg("Could not create the PDF page image."))?;

            let out = name_for(index, n);
            let out_url = cf_url(&out)?;
            let kind = CFString::from_static_str(uti);
            let dest = unsafe { CGImageDestination::with_url(&out_url, &kind, 1, None) }
                .ok_or_else(|| Error::msg("Could not save the PDF page image."))?;
            let dpi_value = NSNumber::new_f64(dpi);
            let quality = NSNumber::new_f64(0.9);
            let keys: [&NSString; 3] = unsafe {
                [
                    ns_key(kCGImagePropertyDPIWidth),
                    ns_key(kCGImagePropertyDPIHeight),
                    ns_key(kCGImageDestinationLossyCompressionQuality),
                ]
            };
            let values: [&AnyObject; 3] = [&dpi_value, &dpi_value, &quality];
            let props = NSDictionary::from_slices(&keys, &values);
            let props_cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&props) as *const CFDictionary) };
            unsafe { dest.add_image(&image, Some(props_cf)) };
            if !unsafe { dest.finalize() } {
                return Err(Error::msg("Could not save the PDF page image."));
            }
            outputs.push(out);
            progress((index + 1) as f64 / n as f64);
        }
        Ok(outputs)
    })
}

/// Paginates an RTF file onto US Letter pages with 1-inch margins.
pub fn rtf_to_pdf(rtf: &Path, out: &Path) -> Result<()> {
    autoreleasepool(|_| {
        let bytes = std::fs::read(rtf)?;
        let data = NSData::with_bytes(&bytes);
        let text = unsafe {
            NSAttributedString::initWithRTF_documentAttributes(NSAttributedString::alloc(), &data, None)
        }
        .ok_or_else(|| Error::msg("Could not read the document text."))?;
        let text = NSMutableAttributedString::initWithAttributedString(NSMutableAttributedString::alloc(), &text);
        apply_core_text_colors(&text);
        let length = text.length() as isize;

        let cf_text: &CFAttributedString = unsafe { &*(Retained::as_ptr(&text) as *const CFAttributedString) };
        let framesetter = unsafe { CTFramesetter::with_attributed_string(cf_text) };
        let page = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(612.0, 792.0));
        let body = CGRect::new(CGPoint::new(72.0, 72.0), CGSize::new(468.0, 648.0));
        let url = cf_url(out)?;
        let ctx = unsafe { CGPDFContextCreateWithURL(Some(&url), &page, None) }
            .ok_or_else(|| Error::msg("Could not create the PDF."))?;
        let path = unsafe { CGPath::with_rect(body, std::ptr::null()) };
        let mut position: isize = 0;
        loop {
            unsafe { CGPDFContextBeginPage(Some(&ctx), None) };
            let frame = unsafe { framesetter.frame(CFRange { location: position, length: 0 }, &path, None) };
            unsafe { frame.draw(&ctx) };
            let visible = unsafe { frame.visible_string_range() };
            CGPDFContextEndPage(Some(&ctx));
            if visible.length <= 0 {
                break;
            }
            position += visible.length;
            if position >= length {
                break;
            }
        }
        CGPDFContextClose(Some(&ctx));
        Ok(())
    })
}

/// CoreText reads colours from its own attribute as CGColor; copy AppKit's NSColor over.
fn apply_core_text_colors(text: &NSMutableAttributedString) {
    let length = text.length();
    let key = unsafe { NSForegroundColorAttributeName };
    let ct_key = ns_key(unsafe { kCTForegroundColorAttributeName });
    let mut index = 0;
    while index < length {
        let mut range = NSRange::new(0, 0);
        let value = unsafe { text.attribute_atIndex_effectiveRange(key, index, &mut range) };
        if let Some(value) = value {
            if let Ok(color) = value.downcast::<NSColor>() {
                let cg = color.CGColor();
                let cg_obj: &AnyObject = unsafe { &*(Retained::as_ptr(&cg) as *const AnyObject) };
                unsafe { text.addAttribute_value_range(ct_key, cg_obj, range) };
            }
        }
        index = (range.location + range.length).max(index + 1);
    }
}
