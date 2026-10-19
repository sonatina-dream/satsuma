//! Minimal EPUB 3 writer and reader.

use crate::html::{attr, body_of, document, escape, tokenize, Token};
use crate::{Error, Result};
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

fn pseudo_uuid() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = nanos ^ ((std::process::id() as u128) << 64) ^ 0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C834;
    // xorshift to spread the bits
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let h = format!("{x:032x}");
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// Writes a single-chapter EPUB. `xhtml_body` must be well-formed XHTML.
pub fn write(out: &Path, title: &str, xhtml_body: &str) -> Result<()> {
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let title = escape(title);
    let modified = "2026-01-01T00:00:00Z";

    let z = |e: zip::result::ZipError| Error::msg(format!("Could not write the EPUB: {e}"));
    zip.start_file("mimetype", stored).map_err(z)?;
    zip.write_all(b"application/epub+zip")?;
    zip.start_file("META-INF/container.xml", deflated).map_err(z)?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"#,
    )?;
    zip.start_file("OEBPS/content.opf", deflated).map_err(z)?;
    zip.write_all(
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:uuid:{uuid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="chapter"/></spine>
</package>
"#,
            uuid = pseudo_uuid()
        )
        .as_bytes(),
    )?;
    zip.start_file("OEBPS/nav.xhtml", deflated).map_err(z)?;
    zip.write_all(
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>{title}</title></head>
<body><nav epub:type="toc" id="toc"><ol><li><a href="chapter.xhtml">{title}</a></li></ol></nav></body>
</html>
"#
        )
        .as_bytes(),
    )?;
    zip.start_file("OEBPS/chapter.xhtml", deflated).map_err(z)?;
    zip.write_all(
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><meta charset="utf-8"/><title>{title}</title></head>
<body>
{xhtml_body}
</body>
</html>
"#
        )
        .as_bytes(),
    )?;
    zip.finish().map_err(z)?;
    Ok(())
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| Error::msg(format!("The EPUB is missing {name}.")))?;
    let mut s = String::new();
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    s.push_str(&String::from_utf8_lossy(&bytes));
    Ok(s)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn join_path(base_dir: &str, href: &str) -> String {
    let mut parts: Vec<&str> = if base_dir.is_empty() { vec![] } else { base_dir.split('/').collect() };
    for seg in href.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Reads an EPUB's spine into one HTML document. Returns (title, html).
pub fn read(path: &Path) -> Result<(String, String)> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| Error::msg("This EPUB file looks damaged."))?;
    let container = read_entry(&mut zip, "META-INF/container.xml")?;
    let opf_path = tokenize(&container)
        .into_iter()
        .find_map(|t| match t {
            Token::Open { name, attrs, .. } if name == "rootfile" => attr(attrs, "full-path"),
            _ => None,
        })
        .ok_or_else(|| Error::msg("The EPUB has no package document."))?;
    let opf = read_entry(&mut zip, &opf_path)?;
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();

    let mut items = std::collections::HashMap::new();
    let mut spine = Vec::new();
    let mut title = String::new();
    let mut in_title = false;
    for t in tokenize(&opf) {
        match t {
            Token::Open { name, attrs, .. } => match name.as_str() {
                "item" => {
                    if let (Some(id), Some(href)) = (attr(attrs, "id"), attr(attrs, "href")) {
                        items.insert(id, (href, attr(attrs, "media-type").unwrap_or_default()));
                    }
                }
                "itemref" => {
                    if let Some(id) = attr(attrs, "idref") {
                        spine.push(id);
                    }
                }
                "dc:title" => in_title = title.is_empty(),
                _ => {}
            },
            Token::Close { name } if name == "dc:title" => in_title = false,
            Token::Text(text) if in_title => title.push_str(&crate::html::decode_entities(text.trim())),
            _ => {}
        }
    }

    let mut body = String::new();
    for id in spine {
        let Some((href, media)) = items.get(&id) else { continue };
        if !(media.contains("html") || href.ends_with("html") || href.ends_with(".htm")) {
            continue;
        }
        let full = join_path(&opf_dir, &percent_decode(href));
        if let Ok(chapter) = read_entry(&mut zip, &full) {
            body.push_str(body_of(&chapter));
            body.push('\n');
        }
    }
    if body.trim().is_empty() {
        return Err(Error::msg("The EPUB has no readable chapters."));
    }
    Ok((title.clone(), document(&title, &body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        write(&path, "My <Book>", "<h1>Chapter</h1>\n<p>Hello &amp; welcome.</p>").unwrap();

        // mimetype must be the first entry and stored uncompressed.
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let first = zip.by_index(0).unwrap();
        assert_eq!(first.name().unwrap(), "mimetype");
        assert_eq!(first.compression(), CompressionMethod::Stored);
        drop(first);

        let (title, html) = read(&path).unwrap();
        assert_eq!(title, "My <Book>");
        assert!(html.contains("<h1>Chapter</h1>"));
        assert!(html.contains("Hello &amp; welcome."));
    }

    #[test]
    fn paths() {
        assert_eq!(join_path("OEBPS", "text/ch%201.xhtml"), "OEBPS/text/ch%201.xhtml");
        assert_eq!(join_path("OEBPS/text", "../img/a.png"), "OEBPS/img/a.png");
        assert_eq!(percent_decode("ch%201.xhtml"), "ch 1.xhtml");
    }
}
