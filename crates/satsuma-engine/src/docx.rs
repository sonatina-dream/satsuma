//! A minimal Word (.docx) writer: paragraphs and an optional inline picture.

use crate::html::escape;
use crate::{Error, Result};
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

/// EMUs per inch, and the text width of a US Letter page with 1" margins.
const EMU_PER_INCH: f64 = 914_400.0;
const TEXT_WIDTH_IN: f64 = 6.5;
const TEXT_HEIGHT_IN: f64 = 9.0;

pub struct Picture<'a> {
    pub png: &'a [u8],
    pub width_px: u32,
    pub height_px: u32,
}

fn paragraph(text: &str) -> String {
    let mut runs = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            runs.push_str("<w:r><w:br/></w:r>");
        }
        runs.push_str(&format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", escape(line)));
    }
    format!("<w:p>{runs}</w:p>")
}

fn picture_xml(p: &Picture) -> String {
    // Fit the page text area, never enlarging beyond 96 dpi.
    let (w_in, h_in) = (p.width_px as f64 / 96.0, p.height_px as f64 / 96.0);
    let s = (TEXT_WIDTH_IN / w_in).min(TEXT_HEIGHT_IN / h_in).min(1.0);
    let (cx, cy) = ((w_in * s * EMU_PER_INCH) as u64, (h_in * s * EMU_PER_INCH) as u64);
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="1" name="Picture 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="image1.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

pub fn write(out: &Path, paragraphs: &[String], picture: Option<Picture>) -> Result<()> {
    let z = |e: zip::result::ZipError| Error::msg(format!("Could not write the Word document: {e}"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(out)?);
    let opts = SimpleFileOptions::default();

    zip.start_file("[Content_Types].xml", opts).map_err(z)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#)?;
    zip.start_file("_rels/.rels", opts).map_err(z)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdDoc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#)?;

    let mut body = String::new();
    for p in paragraphs {
        body.push_str(&paragraph(p));
    }
    if let Some(p) = &picture {
        body.push_str(&picture_xml(p));
        zip.start_file("word/_rels/document.xml.rels", opts).map_err(z)?;
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImage1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#)?;
        zip.start_file("word/media/image1.png", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).map_err(z)?;
        zip.write_all(p.png)?;
    }
    if body.is_empty() {
        body.push_str("<w:p/>");
    }
    zip.start_file("word/document.xml", opts).map_err(z)?;
    zip.write_all(
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
        )
        .as_bytes(),
    )?;
    zip.finish().map_err(z)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textutil_reads_our_documents() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("t.docx");
        write(&out, &["Hello & <world>".into(), "Line one\nline two".into()], None).unwrap();
        let txt = dir.path().join("t.txt");
        let st = std::process::Command::new("/usr/bin/textutil")
            .args(["-convert", "txt", "-output"])
            .arg(&txt)
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        let text = std::fs::read_to_string(&txt).unwrap();
        assert!(text.contains("Hello & <world>"), "{text}");
        assert!(text.contains("line two"), "{text}");
    }
}
