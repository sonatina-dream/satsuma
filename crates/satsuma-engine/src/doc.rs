//! Document conversions. Every input is first turned into HTML (via `textutil`, a Markdown
//! renderer, the EPUB reader or PDF text), and every output is produced from that HTML.

use crate::env::Env;
use crate::html::{document, escape, html_to_markdown};
use crate::process::{run, Cancel};
use crate::{epub, pdf, Error, Result};
use satsuma_core::naming::stem_of;
use satsuma_core::Format::{self, *};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `textutil -convert <to>`; `to` is one of txt, html, rtf, rtfd, doc, docx, odt.
pub fn textutil(env: &Env, input: &Path, out: &Path, to: &str, cancel: &Cancel) -> Result<()> {
    let mut cmd = Command::new(&env.textutil);
    cmd.args(["-convert", to]);
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "txt" || ext == "text" || ext == "md" {
        cmd.args(["-format", "txt", "-inputencoding", "UTF-8"]);
    }
    if to == "txt" || to == "html" {
        cmd.args(["-encoding", "UTF-8"]);
    }
    cmd.arg("-output").arg(out).arg(input);
    run(&mut cmd, cancel, &mut |_| {})?;
    if !out.exists() {
        return Err(Error::msg("textutil did not produce an output file."));
    }
    Ok(())
}

pub fn markdown_to_html_body(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));
    out
}

/// Paragraphs of plain text as HTML.
pub fn text_to_html_body(text: &str) -> String {
    let mut body = String::new();
    for para in split_paragraphs(text) {
        body.push_str("<p>");
        body.push_str(&escape(&para).replace('\n', "<br />\n"));
        body.push_str("</p>\n");
    }
    body
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split("\n\n")
        .map(|p| p.trim_matches('\n').to_string())
        .filter(|p| !p.trim().is_empty())
        .collect()
}

/// Reflows extracted PDF text into Markdown paragraphs.
pub fn text_to_markdown(text: &str) -> String {
    let mut out = String::new();
    for para in split_paragraphs(text) {
        let lines: Vec<&str> = para.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let mut joined = String::new();
        for line in lines {
            if joined.ends_with('-') && !joined.ends_with(" -") {
                joined.pop(); // de-hyphenate line breaks
                joined.push_str(line);
            } else {
                if !joined.is_empty() {
                    joined.push(' ');
                }
                joined.push_str(line);
            }
        }
        out.push_str(&joined);
        out.push_str("\n\n");
    }
    let trimmed = out.trim_end();
    format!("{trimmed}\n")
}

/// Selectable text of a PDF; scanned pages fall back to on-device text recognition.
pub fn pdf_text(input: &Path, tmp: &Path) -> Result<String> {
    let text = pdf::text(input)?;
    if !text.trim().is_empty() {
        return Ok(text);
    }
    let dir = tmp.join("ocr-pages");
    std::fs::create_dir_all(&dir)?;
    let pages = pdf::render_pages(input, Png, 200.0, &|i, _| dir.join(format!("{i:05}.png")), &mut |_| {})?;
    let mut out = Vec::new();
    for page in pages {
        let paragraphs = crate::ocr::paragraphs(crate::ocr::recognize(&page)?);
        if !paragraphs.is_empty() {
            out.push(paragraphs.join("\n\n"));
        }
    }
    if out.is_empty() {
        return Err(Error::msg("This PDF has no text, and none could be recognized."));
    }
    Ok(out.join("\n\n"))
}

enum Clipping {
    Rtf(Vec<u8>),
    Text(String),
}

/// Finder text clippings store their contents in a property list (`UTI-Data`).
fn read_text_clipping(path: &Path) -> Result<Clipping> {
    let value = plist::Value::from_file(path)
        .map_err(|_| Error::msg("Could not read the text clipping."))?;
    let data = value
        .as_dictionary()
        .and_then(|d| d.get("UTI-Data"))
        .and_then(|v| v.as_dictionary())
        .ok_or_else(|| Error::msg("The text clipping has no text."))?;
    if let Some(rtf) = data.get("public.rtf").and_then(|v| v.as_data()) {
        return Ok(Clipping::Rtf(rtf.to_vec()));
    }
    if let Some(s) = data.get("public.utf8-plain-text").and_then(|v| v.as_string()) {
        return Ok(Clipping::Text(s.to_string()));
    }
    if let Some(d) = data.get("public.utf8-plain-text").and_then(|v| v.as_data()) {
        return Ok(Clipping::Text(String::from_utf8_lossy(d).into_owned()));
    }
    if let Some(d) = data.get("public.utf16-plain-text").and_then(|v| v.as_data()) {
        let units: Vec<u16> = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return Ok(Clipping::Text(String::from_utf16_lossy(&units)));
    }
    Err(Error::msg("The text clipping has no text."))
}

/// Loads any supported document as an HTML string.
pub fn load_html(env: &Env, input: &Path, fmt: Format, tmp: &Path, cancel: &Cancel) -> Result<String> {
    let title = stem_of(&input.file_name().unwrap_or_default().to_string_lossy()).to_string();
    match fmt {
        Md => {
            let md = read_text(input)?;
            Ok(document(&title, &markdown_to_html_body(&md)))
        }
        Epub => Ok(epub::read(input)?.1),
        Pdf => {
            let text = pdf_text(input, tmp)?;
            Ok(document(&title, &text_to_html_body(&text)))
        }
        Html => read_text(input),
        TextClipping => match read_text_clipping(input)? {
            Clipping::Text(t) => Ok(document(&title, &text_to_html_body(&t))),
            Clipping::Rtf(bytes) => {
                let rtf = tmp.join("clipping.rtf");
                std::fs::write(&rtf, bytes)?;
                load_via_textutil(env, &rtf, tmp, cancel)
            }
        },
        Docx | Rtf | Rtfd | Txt => load_via_textutil(env, input, tmp, cancel),
        Pages => Err(Error::msg("Pages documents are not supported yet.")),
        other => Err(Error::msg(format!("{} is not a document format.", other.label()))),
    }
}

fn load_via_textutil(env: &Env, input: &Path, tmp: &Path, cancel: &Cancel) -> Result<String> {
    let out = crate::image::unique_in(tmp, "source.html");
    textutil(env, input, &out, "html", cancel)?;
    read_text(&out)
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Writes HTML as a paginated PDF (HTML → RTF via textutil → CoreText).
pub fn html_to_pdf(env: &Env, html: &str, out: &Path, tmp: &Path, cancel: &Cancel) -> Result<()> {
    let src = crate::image::unique_in(tmp, "page.html");
    std::fs::write(&src, crate::html::printable(html))?;
    let rtf = crate::image::unique_in(tmp, "page.rtf");
    textutil(env, &src, &rtf, "rtf", cancel)?;
    pdf::rtf_to_pdf(&rtf, out)
}

/// Writes monospaced text (e.g. a spreadsheet table) as a PDF.
pub fn monospace_to_pdf(text: &str, out: &Path, tmp: &Path) -> Result<()> {
    let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(1).max(1);
    let size = (468.0 / (widest as f64 * 0.61)).clamp(5.0, 10.0);
    let mut rtf = format!("{{\\rtf1\\ansi\\deff0{{\\fonttbl{{\\f0\\fmodern Menlo;}}}}\\f0\\fs{}\n", (size * 2.0).round() as u32);
    for line in text.lines() {
        for c in line.chars() {
            match c {
                '\\' | '{' | '}' => {
                    rtf.push('\\');
                    rtf.push(c);
                }
                c if (c as u32) < 128 => rtf.push(c),
                c => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        rtf.push_str(&format!("\\u{}?", *unit as i16));
                    }
                }
            }
        }
        rtf.push_str("\\par\n");
    }
    rtf.push('}');
    let path = crate::image::unique_in(tmp, "table.rtf");
    std::fs::write(&path, rtf)?;
    pdf::rtf_to_pdf(&path, out)
}

/// Converts a document. Returns the output actually written (a folder for multi-page images).
pub fn convert(env: &Env, input: &Path, fmt: Format, out: &Path, target: Format, tmp: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<PathBuf> {
    let title = stem_of(&input.file_name().unwrap_or_default().to_string_lossy()).to_string();

    // Shortcuts that keep more of the source.
    match (fmt, target) {
        (Md, Html) => {
            let md = read_text(input)?;
            std::fs::write(out, document(&title, &markdown_to_html_body(&md)))?;
            return Ok(out.to_path_buf());
        }
        (Pdf, Txt) => {
            let text = pdf_text(input, tmp)?;
            std::fs::write(out, text)?;
            return Ok(out.to_path_buf());
        }
        (Pdf, Md) => {
            let text = pdf_text(input, tmp)?;
            std::fs::write(out, text_to_markdown(&text))?;
            return Ok(out.to_path_buf());
        }
        (Pdf, Png | Jpg) => return render_pdf_images(input, target, 300.0, out, progress),
        _ => {}
    }

    let html = load_html(env, input, fmt, tmp, cancel)?;
    progress(0.3);
    cancel.check()?;
    match target {
        Html => std::fs::write(out, &html)?,
        Md => std::fs::write(out, html_to_markdown(&html))?,
        Txt | Docx => {
            let src = crate::image::unique_in(tmp, "doc.html");
            std::fs::write(&src, &html)?;
            textutil(env, &src, out, target.id(), cancel)?;
        }
        Epub => {
            // Normalise through Markdown so the chapter is well-formed XHTML.
            let md = html_to_markdown(&html);
            epub::write(out, &title, &markdown_to_html_body(&md))?;
        }
        Pdf => html_to_pdf(env, &html, out, tmp, cancel)?,
        Png | Jpg => {
            let pdf_path = crate::image::unique_in(tmp, "doc.pdf");
            html_to_pdf(env, &html, &pdf_path, tmp, cancel)?;
            progress(0.6);
            return render_pdf_images(&pdf_path, target, 200.0, out, &mut |f| progress(0.6 + f * 0.4));
        }
        other => return Err(Error::msg(format!("Cannot convert documents to {}.", other.label()))),
    }
    progress(1.0);
    Ok(out.to_path_buf())
}

/// Renders PDF pages; one page writes `out`, several write a folder named like `out`'s stem.
pub fn render_pdf_images(input: &Path, target: Format, dpi: f64, out: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf> {
    let pages = pdf::page_count(input)?;
    if pages <= 1 {
        pdf::render_pages(input, target, dpi, &|_, _| out.to_path_buf(), progress)?;
        return Ok(out.to_path_buf());
    }
    let dir = out.with_extension("");
    std::fs::create_dir_all(&dir)?;
    let width = pages.to_string().len();
    let ext = target.extension();
    pdf::render_pages(
        input,
        target,
        dpi,
        &|i, _| dir.join(format!("Page {:0width$}.{ext}", i + 1)),
        progress,
    )?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_renders_tables_and_xhtml_breaks() {
        let html = markdown_to_html_body("# Hi\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nline  \nbreak\n");
        assert!(html.contains("<h1>Hi</h1>"));
        assert!(html.contains("<table>"));
        assert!(html.contains("<br />"));
    }

    #[test]
    fn text_paragraphs() {
        assert_eq!(text_to_html_body("a & b\nc\n\n\nd"), "<p>a &amp; b<br />\nc</p>\n<p>d</p>\n");
        assert_eq!(text_to_markdown("Hello wor-\nld and\nmore\n\nNext"), "Hello world and more\n\nNext\n");
    }
}
