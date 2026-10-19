//! A small, tolerant HTML tokenizer plus HTML → Markdown conversion.
//!
//! It handles what `textutil` and EPUB chapters produce: headings, paragraphs, lists, emphasis,
//! links, images, code, quotes, rules and simple tables.

#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    Open { name: String, attrs: &'a str, self_closing: bool },
    Close { name: String },
    Text(&'a str),
}

/// Splits HTML into tags and text. Comments, doctypes and processing instructions are dropped.
pub fn tokenize(html: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let bytes = html.as_bytes();
    let mut pos = 0;
    while pos < html.len() {
        if bytes[pos] == b'<' {
            if html[pos..].starts_with("<!--") {
                pos = html[pos..].find("-->").map_or(html.len(), |e| pos + e + 3);
                continue;
            }
            let Some(end_rel) = html[pos..].find('>') else {
                tokens.push(Token::Text(&html[pos..]));
                break;
            };
            let inner = &html[pos + 1..pos + end_rel];
            pos += end_rel + 1;
            if inner.starts_with('!') || inner.starts_with('?') {
                continue;
            }
            if let Some(rest) = inner.strip_prefix('/') {
                tokens.push(Token::Close { name: tag_name(rest) });
            } else {
                let name = tag_name(inner);
                if name.is_empty() {
                    continue;
                }
                let attrs = &inner[inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len())..];
                let self_closing = inner.trim_end().ends_with('/');
                tokens.push(Token::Open { name, attrs, self_closing });
            }
        } else {
            let end = html[pos..].find('<').map_or(html.len(), |e| pos + e);
            tokens.push(Token::Text(&html[pos..end]));
            pos = end;
        }
    }
    tokens
}

fn tag_name(s: &str) -> String {
    s.trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Value of attribute `name` in a tag's attribute text.
pub fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut search = 0;
    while let Some(i) = lower[search..].find(name) {
        let start = search + i;
        let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric() && lower.as_bytes()[start - 1] != b'-' && lower.as_bytes()[start - 1] != b':';
        let rest = attrs[start + name.len()..].trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let value = if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let body = &rest[1..];
                    &body[..body.find(q).unwrap_or(body.len())]
                } else {
                    &rest[..rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len())]
                };
                return Some(decode_entities(value));
            }
        }
        search = start + name.len();
    }
    None
}

/// Decodes the common named entities and numeric character references.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                "copy" => Some('©'),
                "reg" => Some('®'),
                "trade" => Some('™'),
                "bull" => Some('•'),
                _ => {
                    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = name.strip_prefix('#') {
                        dec.parse().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            ch.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Wraps a body in a minimal UTF-8 HTML document.
pub fn document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n</head>\n<body>\n{}\n</body>\n</html>\n",
        escape(title),
        body
    )
}

/// Inner HTML of `<body>`, or the whole input when there is no body tag.
pub fn body_of(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    let start = lower
        .find("<body")
        .and_then(|i| lower[i..].find('>').map(|e| i + e + 1))
        .unwrap_or(0);
    let end = lower.rfind("</body>").filter(|e| *e >= start).unwrap_or(html.len());
    &html[start..end]
}

fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '`' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Default)]
struct Md {
    out: String,
    lists: Vec<Option<usize>>,
    skip: usize,
    pre: bool,
    links: Vec<(usize, Option<String>)>,
    table: Option<Vec<Vec<String>>>,
    row: Option<Vec<String>>,
    cell: Option<String>,
    quote: usize,
}

impl Md {
    fn buf(&mut self) -> &mut String {
        if let Some(c) = self.cell.as_mut() {
            c
        } else {
            &mut self.out
        }
    }

    fn blank_line(&mut self) {
        if self.cell.is_some() {
            self.buf().push(' ');
            return;
        }
        let trimmed = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(trimmed);
        if self.out.is_empty() {
            return;
        }
        while !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        self.quote_prefix();
    }

    fn newline(&mut self) {
        if self.cell.is_some() {
            self.buf().push(' ');
            return;
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.quote_prefix();
    }

    fn quote_prefix(&mut self) {
        if self.quote > 0 {
            for _ in 0..self.quote {
                self.out.push_str("> ");
            }
        }
    }

    fn text(&mut self, raw: &str) {
        if self.skip > 0 {
            return;
        }
        let decoded = decode_entities(raw);
        if self.pre {
            self.buf().push_str(&decoded);
            return;
        }
        let collapsed: String = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if decoded.chars().any(char::is_whitespace) && !self.buf().ends_with([' ', '\n']) && !self.buf().is_empty() {
                self.buf().push(' ');
            }
            return;
        }
        let lead = decoded.starts_with(char::is_whitespace);
        let trail = decoded.ends_with(char::is_whitespace);
        let buf = self.buf();
        if lead && !buf.is_empty() && !buf.ends_with([' ', '\n']) {
            buf.push(' ');
        }
        buf.push_str(&escape_markdown(&collapsed));
        if trail {
            buf.push(' ');
        }
    }

    fn open(&mut self, name: &str, attrs: &str, self_closing: bool) {
        if matches!(name, "head" | "style" | "script" | "title") {
            if !self_closing {
                self.skip += 1;
            }
            return;
        }
        if self.skip > 0 {
            return;
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.blank_line();
                let level = name[1..].parse::<usize>().unwrap_or(1);
                let hashes = "#".repeat(level);
                self.buf().push_str(&hashes);
                self.buf().push(' ');
            }
            "p" | "div" | "section" | "article" | "header" | "footer" => self.blank_line(),
            "blockquote" => {
                self.blank_line();
                self.quote += 1;
                self.out.push_str("> ");
            }
            "br" => {
                if self.cell.is_some() {
                    self.buf().push(' ');
                } else {
                    self.out.push_str("  \n");
                    self.quote_prefix();
                }
            }
            "hr" => {
                self.blank_line();
                self.out.push_str("---");
                self.blank_line();
            }
            "ul" | "ol" => {
                if self.lists.is_empty() {
                    self.blank_line();
                }
                self.lists.push(if name == "ol" { Some(0) } else { None });
            }
            "li" => {
                self.newline();
                let depth = self.lists.len().max(1);
                let indent = "  ".repeat(depth - 1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{n}. ")
                    }
                    _ => "- ".to_string(),
                };
                self.out.push_str(&indent);
                self.out.push_str(&marker);
            }
            "b" | "strong" => self.buf().push_str("**"),
            "i" | "em" => self.buf().push('*'),
            "code" if !self.pre => self.buf().push('`'),
            "pre" => {
                self.blank_line();
                self.out.push_str("```\n");
                self.pre = true;
            }
            "a" => {
                let start = self.buf().len();
                self.links.push((start, attr(attrs, "href")));
            }
            "img" => {
                let alt = attr(attrs, "alt").unwrap_or_default();
                if let Some(src) = attr(attrs, "src") {
                    let s = format!("![{}]({})", alt, src);
                    self.buf().push_str(&s);
                }
            }
            "table" => {
                self.blank_line();
                self.table = Some(Vec::new());
            }
            "tr" => self.row = Some(Vec::new()),
            "td" | "th" => self.cell = Some(String::new()),
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        if matches!(name, "head" | "style" | "script" | "title") {
            self.skip = self.skip.saturating_sub(1);
            return;
        }
        if self.skip > 0 {
            return;
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article"
            | "header" | "footer" => self.blank_line(),
            "blockquote" => {
                self.quote = self.quote.saturating_sub(1);
                self.blank_line();
            }
            "ul" | "ol" => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank_line();
                } else {
                    self.newline();
                }
            }
            "b" | "strong" => self.close_marker("**"),
            "i" | "em" => self.close_marker("*"),
            "code" if !self.pre => self.close_marker("`"),
            "pre" => {
                self.pre = false;
                self.newline();
                self.out.push_str("```");
                self.blank_line();
            }
            "a" => {
                if let Some((start, href)) = self.links.pop() {
                    let buf = self.buf();
                    let start = start.min(buf.len());
                    if let Some(href) = href.filter(|h| !h.is_empty() && !h.starts_with('#')) {
                        let label = buf[start..].trim().to_string();
                        buf.truncate(start);
                        buf.push_str(&format!("[{}]({})", if label.is_empty() { href.as_str() } else { label.as_str() }, href));
                    }
                }
            }
            "td" | "th" => {
                if let Some(c) = self.cell.take() {
                    let c = c.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|");
                    self.row.get_or_insert_with(Vec::new).push(c);
                }
            }
            "tr" => {
                if let (Some(row), Some(table)) = (self.row.take(), self.table.as_mut()) {
                    if !row.is_empty() {
                        table.push(row);
                    }
                }
            }
            "table" => {
                if let Some(rows) = self.table.take() {
                    self.out.push_str(&markdown_table(&rows));
                    self.blank_line();
                }
            }
            _ => {}
        }
    }

    /// Closes an emphasis marker, moving trailing spaces outside it and dropping empty pairs.
    fn close_marker(&mut self, marker: &str) {
        let buf = self.buf();
        let trailing = buf.len() - buf.trim_end_matches(' ').len();
        buf.truncate(buf.len() - trailing);
        if buf.ends_with(marker) {
            buf.truncate(buf.len() - marker.len());
        } else {
            buf.push_str(marker);
        }
        for _ in 0..trailing {
            buf.push(' ');
        }
    }
}

/// Renders rows as a GitHub-style table; the first row is the header.
pub fn markdown_table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    let line = |r: &[String]| {
        let cells: Vec<String> = (0..cols).map(|i| r.get(i).cloned().unwrap_or_default()).collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut s = line(&rows[0]);
    s.push('\n');
    s.push_str(&format!("|{}\n", " --- |".repeat(cols)));
    for r in &rows[1..] {
        s.push_str(&line(r));
        s.push('\n');
    }
    s
}

/// Rewrites HTML so a plain text engine (CoreText) can print it: list items become indented
/// paragraphs with explicit markers, tables become aligned monospace blocks, and quotes
/// become indented blocks. Everything else is passed through unchanged.
pub fn printable(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut lists: Vec<Option<usize>> = Vec::new();
    let mut in_item = 0usize;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut cell: Option<String> = None;
    for token in tokenize(html) {
        if let Some(rows) = table.as_mut() {
            match &token {
                Token::Open { name, .. } if name == "tr" => rows.push(Vec::new()),
                Token::Open { name, .. } if name == "td" || name == "th" => cell = Some(String::new()),
                Token::Open { name, .. } if name == "br" => {
                    if let Some(c) = cell.as_mut() {
                        c.push(' ');
                    }
                }
                Token::Close { name } if name == "td" || name == "th" => {
                    if let (Some(c), Some(row)) = (cell.take(), rows.last_mut()) {
                        row.push(c.split_whitespace().collect::<Vec<_>>().join(" "));
                    }
                }
                Token::Close { name } if name == "table" => {
                    let rows = table.take().unwrap_or_default();
                    out.push_str("<pre style=\"font-family: Menlo; font-size: 10px\">");
                    out.push_str(&escape(&aligned_rows(&rows)));
                    out.push_str("</pre>\n");
                }
                Token::Text(t) => {
                    if let Some(c) = cell.as_mut() {
                        c.push_str(&decode_entities(t));
                    }
                }
                _ => {}
            }
            continue;
        }
        match token {
            Token::Open { name, .. } if name == "table" => table = Some(Vec::new()),
            Token::Open { name, .. } if name == "ul" || name == "ol" => {
                lists.push(if name == "ol" { Some(0) } else { None });
            }
            Token::Close { name } if name == "ul" || name == "ol" => {
                lists.pop();
            }
            Token::Open { name, .. } if name == "li" => {
                let depth = lists.len().max(1);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{n}.")
                    }
                    _ => "•".to_string(),
                };
                if in_item > 0 {
                    out.push_str("</p>\n");
                }
                in_item += 1;
                out.push_str(&format!(
                    "<p style=\"margin-left: {}px; text-indent: -14px\">{marker}&nbsp;&nbsp;",
                    18 * depth
                ));
            }
            Token::Close { name } if name == "li" => {
                if in_item > 0 {
                    in_item -= 1;
                    out.push_str("</p>\n");
                }
            }
            // Paragraph tags inside list items would split the marker from its text.
            Token::Open { name, .. } | Token::Close { name } if in_item > 0 && name == "p" => {}
            Token::Open { name, .. } if name == "blockquote" => {
                out.push_str("<div style=\"margin-left: 24px; color: #555555\">");
            }
            Token::Close { name } if name == "blockquote" => out.push_str("</div>"),
            Token::Open { name, attrs, self_closing } => {
                out.push('<');
                out.push_str(&name);
                out.push_str(attrs.trim_end().trim_end_matches('/').trim_end());
                out.push_str(if self_closing { " />" } else { ">" });
            }
            Token::Close { name } => {
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
            }
            Token::Text(t) => out.push_str(t),
        }
    }
    out
}

/// Rows as space-aligned columns.
pub fn aligned_rows(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let line: Vec<String> = (0..cols)
            .map(|c| {
                let s = row.get(c).map(String::as_str).unwrap_or("");
                format!("{s}{}", " ".repeat(widths[c] - s.chars().count()))
            })
            .collect();
        out.push_str(line.join("   ").trim_end());
        out.push('\n');
        if i == 0 && rows.len() > 1 {
            let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            out.push_str(&rule.join("   "));
            out.push('\n');
        }
    }
    out
}

pub fn html_to_markdown(html: &str) -> String {
    let mut md = Md::default();
    for token in tokenize(html) {
        match token {
            Token::Open { name, attrs, self_closing } => md.open(&name, attrs, self_closing),
            Token::Close { name } => md.close(&name),
            Token::Text(t) => md.text(t),
        }
    }
    // Tidy: trim line ends and collapse blank runs.
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = 0;
    for line in md.out.lines() {
        let l = line.trim_end();
        let l = if l.ends_with('\\') { line } else { l };
        if l.trim().is_empty() || l.trim() == ">" {
            blank += 1;
            if blank > 1 {
                continue;
            }
            lines.push("");
        } else {
            blank = 0;
            lines.push(l);
        }
    }
    let mut s = lines.join("\n").trim().to_string();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textutil_style_html() {
        let html = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN">
<html><head><meta http-equiv="Content-Type" content="text/html; charset=UTF-8">
<title></title><style type="text/css">p.p1 {margin: 0.0px}</style></head>
<body>
<p class="p1"><b>Bold</b> and <i>italic</i> &amp; more</p>
<p class="p1">Second&nbsp;para with <a href="https://example.com">a link</a>.</p>
<ul class="ul1"><li class="li1">one</li><li class="li1">two</li></ul>
</body></html>"#;
        let md = html_to_markdown(html);
        assert_eq!(
            md,
            "**Bold** and *italic* & more\n\nSecond para with [a link](https://example.com).\n\n- one\n- two\n"
        );
    }

    #[test]
    fn headings_lists_tables_code() {
        let html = "<h1>Title</h1><h2>Sub</h2><ol><li>a</li><li>b<ul><li>c</li></ul></li></ol>\
            <table><tr><th>Name</th><th>Qty</th></tr><tr><td>Apple</td><td>3</td></tr></table>\
            <pre><code>let x = 1;\n</code></pre><blockquote><p>quoted</p></blockquote>";
        let md = html_to_markdown(html);
        assert!(md.starts_with("# Title\n\n## Sub\n\n1. a\n2. b\n  - c\n"), "{md}");
        assert!(md.contains("| Name | Qty |\n| --- | --- |\n| Apple | 3 |"), "{md}");
        assert!(md.contains("```\nlet x = 1;\n```"), "{md}");
        assert!(md.contains("> quoted"), "{md}");
    }

    #[test]
    fn escapes_markdown_characters_and_decodes_entities() {
        let md = html_to_markdown("<p>2 * 3 = 6 &lt;ok&gt; &#x263A; &#9731;</p>");
        assert_eq!(md, "2 \\* 3 = 6 <ok> ☺ ☃\n");
    }

    #[test]
    fn printable_lists_tables_quotes() {
        let html = "<h1>T</h1><ul><li><p>one</p></li><li>two<ol><li>a</li></ol></li></ul>\
            <table><tr><th>Tree</th><th>Fruit</th></tr><tr><td>A</td><td>12</td></tr></table>\
            <blockquote><p>q</p></blockquote><img src=\"x.png\" />";
        let p = printable(html);
        assert!(p.contains("<h1>T</h1>"));
        assert!(p.contains("margin-left: 18px; text-indent: -14px\">•&nbsp;&nbsp;one</p>"), "{p}");
        assert!(p.contains("margin-left: 36px; text-indent: -14px\">1.&nbsp;&nbsp;a</p>"), "{p}");
        assert!(p.contains("Tree   Fruit\n────   ─────\nA      12\n"), "{p}");
        assert!(p.contains("<div style=\"margin-left: 24px; color: #555555\"><p>q</p></div>"), "{p}");
        assert!(p.contains("<img src=\"x.png\" />"), "{p}");
        assert!(!p.contains("<ul>") && !p.contains("<table>"));
    }

    #[test]
    fn attributes() {
        assert_eq!(attr(r#" href="a&amp;b" class=x"#, "href").as_deref(), Some("a&b"));
        assert_eq!(attr(" class=x data-href='no'", "href"), None);
        assert_eq!(attr(" class=x", "class").as_deref(), Some("x"));
        assert_eq!(attr(r#" full-path="OEBPS/c.opf""#, "full-path").as_deref(), Some("OEBPS/c.opf"));
    }

    #[test]
    fn body_extraction() {
        assert_eq!(body_of("<html><BODY class=a>hi</BODY></html>"), "hi");
        assert_eq!(body_of("no body"), "no body");
    }
}
