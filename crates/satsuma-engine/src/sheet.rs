//! Spreadsheet conversions (first sheet): XLSX/XLS/ODS via calamine, CSV/TSV via csv, XLSX
//! output via rust_xlsxwriter, ODS output written by hand, PDF as a monospaced table.

use crate::{Error, Result};
use satsuma_core::Format::{self, *};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl Cell {
    /// Interprets CSV text: plain numbers become numbers (but "007" stays text).
    pub fn from_text(s: &str) -> Cell {
        let t = s.trim();
        if t.is_empty() {
            return Cell::Empty;
        }
        let digits = t.trim_start_matches('-');
        let leading_zero = digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.");
        if !leading_zero && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E' | '+')) {
            if let Ok(n) = t.parse::<f64>() {
                if n.is_finite() {
                    return Cell::Number(n);
                }
            }
        }
        Cell::Text(s.to_string())
    }

    pub fn display(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Number(n) => format_number(*n),
            Cell::Text(s) => s.clone(),
            Cell::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }
}

pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
}

pub fn read(path: &Path, fmt: Format) -> Result<Sheet> {
    match fmt {
        Csv | Tsv => read_delimited(path, if fmt == Tsv { b'\t' } else { b',' }),
        Xlsx | Xls | Ods => read_workbook(path),
        other => Err(Error::msg(format!("{} is not a spreadsheet.", other.label()))),
    }
}

fn read_delimited(path: &Path, delimiter: u8) -> Result<Sheet> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(bytes);
    let mut rows = Vec::new();
    for record in reader.byte_records() {
        let record = record.map_err(|e| Error::msg(format!("Could not read the table: {e}")))?;
        rows.push(record.iter().map(|f| Cell::from_text(&String::from_utf8_lossy(f))).collect());
    }
    Ok(Sheet { name: "Sheet1".into(), rows })
}

fn read_workbook(path: &Path) -> Result<Sheet> {
    use calamine::{open_workbook_auto, Data, Reader};
    let mut wb = open_workbook_auto(path).map_err(|e| Error::msg(format!("Could not read the spreadsheet: {e}")))?;
    let name = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| Error::msg("The spreadsheet has no sheets."))?;
    let range = wb
        .worksheet_range(&name)
        .map_err(|e| Error::msg(format!("Could not read the sheet {name}: {e}")))?;
    let (row0, col0) = range.start().unwrap_or((0, 0));
    let mut rows: Vec<Vec<Cell>> = (0..row0).map(|_| Vec::new()).collect();
    for row in range.rows() {
        let mut cells: Vec<Cell> = (0..col0).map(|_| Cell::Empty).collect();
        for c in row {
            cells.push(match c {
                Data::Empty => Cell::Empty,
                Data::Int(i) => Cell::Number(*i as f64),
                Data::Float(f) => Cell::Number(*f),
                Data::Bool(b) => Cell::Bool(*b),
                Data::String(s) => Cell::Text(s.clone()),
                other => Cell::Text(other.to_string()),
            });
        }
        while matches!(cells.last(), Some(Cell::Empty)) {
            cells.pop();
        }
        rows.push(cells);
    }
    Ok(Sheet { name, rows })
}

pub fn write(sheet: &Sheet, fmt: Format, out: &Path, tmp: &Path) -> Result<()> {
    match fmt {
        Csv | Tsv => write_delimited(sheet, if fmt == Tsv { b'\t' } else { b',' }, out),
        Xlsx => write_xlsx(sheet, out),
        Ods => write_ods(sheet, out),
        Pdf => crate::doc::monospace_to_pdf(&text_table(sheet), out, tmp),
        other => Err(Error::msg(format!("Cannot write spreadsheets as {}.", other.label()))),
    }
}

fn write_delimited(sheet: &Sheet, delimiter: u8, out: &Path) -> Result<()> {
    let mut w = csv::WriterBuilder::new().delimiter(delimiter).flexible(true).from_path(out)
        .map_err(|e| Error::msg(format!("Could not write the table: {e}")))?;
    for row in &sheet.rows {
        let fields: Vec<String> = row.iter().map(Cell::display).collect();
        if fields.is_empty() {
            w.write_record([""]).map_err(|e| Error::msg(e.to_string()))?;
        } else {
            w.write_record(&fields).map_err(|e| Error::msg(e.to_string()))?;
        }
    }
    w.flush()?;
    Ok(())
}

fn write_xlsx(sheet: &Sheet, out: &Path) -> Result<()> {
    use rust_xlsxwriter::Workbook;
    let x = |e: rust_xlsxwriter::XlsxError| Error::msg(format!("Could not write the workbook: {e}"));
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    let name: String = sheet.name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if !name.is_empty() {
        ws.set_name(&name).map_err(x)?;
    }
    for (r, row) in sheet.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let (r, c) = (r as u32, c as u16);
            match cell {
                Cell::Empty => {}
                Cell::Number(n) => {
                    ws.write_number(r, c, *n).map_err(x)?;
                }
                Cell::Text(s) => {
                    ws.write_string(r, c, s).map_err(x)?;
                }
                Cell::Bool(b) => {
                    ws.write_boolean(r, c, *b).map_err(x)?;
                }
            }
        }
    }
    wb.save(out).map_err(x)?;
    Ok(())
}

fn xml_escape(s: &str) -> String {
    crate::html::escape(s).replace('\'', "&apos;")
}

fn write_ods(sheet: &Sheet, out: &Path) -> Result<()> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let z = |e: zip::result::ZipError| Error::msg(format!("Could not write the spreadsheet: {e}"));
    let mut content = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet>"#,
    );
    content.push_str(&format!("<table:table table:name=\"{}\">", xml_escape(&sheet.name)));
    for row in &sheet.rows {
        content.push_str("<table:table-row>");
        if row.is_empty() {
            content.push_str("<table:table-cell/>");
        }
        for cell in row {
            match cell {
                Cell::Empty => content.push_str("<table:table-cell/>"),
                Cell::Number(n) => content.push_str(&format!(
                    "<table:table-cell office:value-type=\"float\" office:value=\"{n}\"><text:p>{}</text:p></table:table-cell>",
                    format_number(*n)
                )),
                Cell::Bool(b) => content.push_str(&format!(
                    "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{b}\"><text:p>{}</text:p></table:table-cell>",
                    if *b { "TRUE" } else { "FALSE" }
                )),
                Cell::Text(s) => content.push_str(&format!(
                    "<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>",
                    xml_escape(s)
                )),
            }
        }
        content.push_str("</table:table-row>");
    }
    content.push_str("</table:table></office:spreadsheet></office:body></office:document-content>\n");

    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(out)?);
    zip.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).map_err(z)?;
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet")?;
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("META-INF/manifest.xml", deflate).map_err(z)?;
    zip.write_all(manifest.as_bytes())?;
    zip.start_file("content.xml", deflate).map_err(z)?;
    zip.write_all(content.as_bytes())?;
    zip.finish().map_err(z)?;
    Ok(())
}

/// Aligned plain-text rendering used for PDF export.
pub fn text_table(sheet: &Sheet) -> String {
    const MAX: usize = 40;
    let cols = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    let cell = |r: &Vec<Cell>, c: usize| -> String {
        let s = r.get(c).map(Cell::display).unwrap_or_default().replace(['\n', '\r', '\t'], " ");
        if s.chars().count() > MAX {
            s.chars().take(MAX - 1).collect::<String>() + "…"
        } else {
            s
        }
    };
    let widths: Vec<usize> = (0..cols)
        .map(|c| sheet.rows.iter().map(|r| cell(r, c).chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in &sheet.rows {
        let line: Vec<String> = (0..cols)
            .map(|c| {
                let s = cell(row, c);
                let pad = widths[c] - s.chars().count();
                let numeric = matches!(row.get(c), Some(Cell::Number(_)));
                if numeric {
                    format!("{}{}", " ".repeat(pad), s)
                } else {
                    format!("{}{}", s, " ".repeat(pad))
                }
            })
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sheet {
        Sheet {
            name: "Budget".into(),
            rows: vec![
                vec![Cell::Text("Item".into()), Cell::Text("Cost".into()), Cell::Text("Zip".into())],
                vec![Cell::Text("Rent, monthly".into()), Cell::Number(1200.0), Cell::Text("007".into())],
                vec![Cell::Text("Coffee \"beans\"".into()), Cell::Number(12.5), Cell::Bool(true)],
            ],
        }
    }

    #[test]
    fn number_detection() {
        assert_eq!(Cell::from_text("42"), Cell::Number(42.0));
        assert_eq!(Cell::from_text("-3.5"), Cell::Number(-3.5));
        assert_eq!(Cell::from_text("007"), Cell::Text("007".into()));
        assert_eq!(Cell::from_text("0.5"), Cell::Number(0.5));
        assert_eq!(Cell::from_text("12 apples"), Cell::Text("12 apples".into()));
        assert_eq!(Cell::from_text(" "), Cell::Empty);
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(3.25), "3.25");
    }

    #[test]
    fn csv_tsv_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("t.csv");
        write(&sample(), Csv, &csv, dir.path()).unwrap();
        let text = std::fs::read_to_string(&csv).unwrap();
        assert!(text.contains("\"Rent, monthly\",1200,007"), "{text}");
        let back = read(&csv, Csv).unwrap();
        assert_eq!(back.rows[1][1], Cell::Number(1200.0));
        assert_eq!(back.rows[1][2], Cell::Text("007".into()));

        let tsv = dir.path().join("t.tsv");
        write(&back, Tsv, &tsv, dir.path()).unwrap();
        assert!(std::fs::read_to_string(&tsv).unwrap().contains("Rent, monthly\t1200\t007"));
    }

    #[test]
    fn xlsx_and_ods_round_trip_through_calamine() {
        let dir = tempfile::tempdir().unwrap();
        for fmt in [Xlsx, Ods] {
            let path = dir.path().join(format!("t.{}", fmt.extension()));
            write(&sample(), fmt, &path, dir.path()).unwrap();
            let back = read(&path, fmt).unwrap();
            assert_eq!(back.name, "Budget", "{fmt:?}");
            assert_eq!(back.rows[1][0], Cell::Text("Rent, monthly".into()), "{fmt:?}");
            assert_eq!(back.rows[1][1], Cell::Number(1200.0), "{fmt:?}");
            assert_eq!(back.rows[2][0], Cell::Text("Coffee \"beans\"".into()), "{fmt:?}");
            assert_eq!(back.rows[2][2], Cell::Bool(true), "{fmt:?}");
        }
    }

    #[test]
    fn text_table_aligns_numbers_right() {
        let t = text_table(&sample());
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "Item            Cost  Zip");
        assert_eq!(lines[1], "Rent, monthly   1200  007");
        assert_eq!(lines[2], "Coffee \"beans\"  12.5  TRUE");
    }
}
