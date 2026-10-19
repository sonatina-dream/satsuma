//! File formats Satsuma understands, and how they are recognised from file names.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    // Images
    Jpg,
    Png,
    Webp,
    Heic,
    Tiff,
    Svg,
    Avif,
    Bmp,
    // Animated image (treated as video)
    Gif,
    // Audio
    Mp3,
    M4a,
    Wav,
    Flac,
    Ogg,
    Opus,
    Aiff,
    Wma,
    // Video
    Mp4,
    Mov,
    Mkv,
    Webm,
    Avi,
    Wmv,
    // Subtitles
    Srt,
    Vtt,
    // Documents
    Pdf,
    Docx,
    Pages,
    Md,
    Rtf,
    Rtfd,
    TextClipping,
    Txt,
    Html,
    Epub,
    // Spreadsheets
    Xlsx,
    Xls,
    Ods,
    Csv,
    Tsv,
    // Slides
    Ppt,
    Pptx,
    // Archives
    Zip,
    Tar,
    Gz,
    Rar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Image,
    Audio,
    Video,
    Document,
    Spreadsheet,
    Subtitle,
    Archive,
}

impl Format {
    pub const ALL: [Format; 46] = [
        Format::Jpg,
        Format::Png,
        Format::Webp,
        Format::Heic,
        Format::Tiff,
        Format::Svg,
        Format::Avif,
        Format::Bmp,
        Format::Gif,
        Format::Mp3,
        Format::M4a,
        Format::Wav,
        Format::Flac,
        Format::Ogg,
        Format::Opus,
        Format::Aiff,
        Format::Wma,
        Format::Mp4,
        Format::Mov,
        Format::Mkv,
        Format::Webm,
        Format::Avi,
        Format::Wmv,
        Format::Srt,
        Format::Vtt,
        Format::Pdf,
        Format::Docx,
        Format::Pages,
        Format::Md,
        Format::Rtf,
        Format::Rtfd,
        Format::TextClipping,
        Format::Txt,
        Format::Html,
        Format::Epub,
        Format::Xlsx,
        Format::Xls,
        Format::Ods,
        Format::Csv,
        Format::Tsv,
        Format::Ppt,
        Format::Pptx,
        Format::Zip,
        Format::Tar,
        Format::Gz,
        Format::Rar,
    ];

    /// Stable lower-case identifier (used in settings, CLI and logs).
    pub fn id(self) -> &'static str {
        use Format::*;
        match self {
            Jpg => "jpg",
            Png => "png",
            Webp => "webp",
            Heic => "heic",
            Tiff => "tiff",
            Svg => "svg",
            Avif => "avif",
            Bmp => "bmp",
            Gif => "gif",
            Mp3 => "mp3",
            M4a => "m4a",
            Wav => "wav",
            Flac => "flac",
            Ogg => "ogg",
            Opus => "opus",
            Aiff => "aiff",
            Wma => "wma",
            Mp4 => "mp4",
            Mov => "mov",
            Mkv => "mkv",
            Webm => "webm",
            Avi => "avi",
            Wmv => "wmv",
            Srt => "srt",
            Vtt => "vtt",
            Pdf => "pdf",
            Docx => "docx",
            Pages => "pages",
            Md => "md",
            Rtf => "rtf",
            Rtfd => "rtfd",
            TextClipping => "textClipping",
            Txt => "txt",
            Html => "html",
            Epub => "epub",
            Xlsx => "xlsx",
            Xls => "xls",
            Ods => "ods",
            Csv => "csv",
            Tsv => "tsv",
            Ppt => "ppt",
            Pptx => "pptx",
            Zip => "zip",
            Tar => "tar",
            Gz => "gz",
            Rar => "rar",
        }
    }

    /// Parses an identifier as produced by [`Format::id`] (case-insensitive) or a common alias.
    pub fn from_id(s: &str) -> Option<Format> {
        let lower = s.trim().trim_start_matches('.').to_ascii_lowercase();
        Self::from_extension(&lower)
    }

    /// The file extension written for outputs of this format.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Gz => "tar.gz",
            other => other.id(),
        }
    }

    /// Upper-case label shown on wheel petals.
    pub fn label(self) -> String {
        match self {
            Format::Gz => "GZIP".to_string(),
            Format::TextClipping => "CLIPPING".to_string(),
            other => other.id().to_ascii_uppercase(),
        }
    }

    pub fn category(self) -> Category {
        use Format::*;
        match self {
            Jpg | Png | Webp | Heic | Tiff | Svg | Avif | Bmp => Category::Image,
            Mp3 | M4a | Wav | Flac | Ogg | Opus | Aiff | Wma => Category::Audio,
            Gif | Mp4 | Mov | Mkv | Webm | Avi | Wmv => Category::Video,
            Srt | Vtt => Category::Subtitle,
            Xlsx | Xls | Ods | Csv | Tsv => Category::Spreadsheet,
            Zip | Tar | Gz | Rar => Category::Archive,
            Pdf | Docx | Pages | Md | Rtf | Rtfd | TextClipping | Txt | Html | Epub | Ppt
            | Pptx => Category::Document,
        }
    }

    /// Formats stored as directory bundles on macOS.
    pub fn is_package(self) -> bool {
        matches!(self, Format::Rtfd | Format::Pages)
    }

    fn from_extension(ext: &str) -> Option<Format> {
        use Format::*;
        Some(match ext {
            "jpg" | "jpeg" | "jpe" | "jfif" => Jpg,
            "png" => Png,
            "webp" => Webp,
            "heic" | "heif" => Heic,
            "tif" | "tiff" => Tiff,
            "svg" => Svg,
            "avif" => Avif,
            "bmp" => Bmp,
            "gif" => Gif,
            "mp3" => Mp3,
            "m4a" => M4a,
            "wav" | "wave" => Wav,
            "flac" => Flac,
            "ogg" | "oga" => Ogg,
            "opus" => Opus,
            "aif" | "aiff" | "aifc" => Aiff,
            "wma" => Wma,
            "mp4" | "m4v" => Mp4,
            "mov" | "qt" => Mov,
            "mkv" => Mkv,
            "webm" => Webm,
            "avi" => Avi,
            "wmv" => Wmv,
            "srt" => Srt,
            "vtt" => Vtt,
            "pdf" => Pdf,
            "docx" => Docx,
            "pages" => Pages,
            "md" | "markdown" => Md,
            "rtf" => Rtf,
            "rtfd" => Rtfd,
            "textclipping" => TextClipping,
            "txt" | "text" => Txt,
            "html" | "htm" => Html,
            "epub" => Epub,
            "xlsx" => Xlsx,
            "xls" => Xls,
            "ods" => Ods,
            "csv" => Csv,
            "tsv" | "tab" => Tsv,
            "ppt" => Ppt,
            "pptx" => Pptx,
            "zip" => Zip,
            "tar" => Tar,
            "gz" | "tgz" | "gzip" | "tar.gz" => Gz,
            "rar" => Rar,
            _ => return None,
        })
    }

    /// Recognises a format from a file name. `.tar.gz` and `.tgz` count as [`Format::Gz`].
    pub fn from_path(path: &Path) -> Option<Format> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            return Some(Format::Gz);
        }
        let (stem, ext) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        Self::from_extension(ext)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_extensions() {
        let cases = [
            ("photo.JPG", Format::Jpg),
            ("photo.jpeg", Format::Jpg),
            ("scan.tif", Format::Tiff),
            ("song.aif", Format::Aiff),
            ("clip.m4v", Format::Mp4),
            ("notes.markdown", Format::Md),
            ("page.htm", Format::Html),
            ("Snippet.textClipping", Format::TextClipping),
            ("backup.tar.gz", Format::Gz),
            ("backup.TGZ", Format::Gz),
            ("data.tab", Format::Tsv),
        ];
        for (name, expected) in cases {
            assert_eq!(Format::from_path(Path::new(name)), Some(expected), "{name}");
        }
    }

    #[test]
    fn rejects_unknown_or_extensionless_names() {
        assert_eq!(Format::from_path(Path::new("README")), None);
        assert_eq!(Format::from_path(Path::new(".zip")), None);
        assert_eq!(Format::from_path(Path::new("thing.xyz")), None);
    }

    #[test]
    fn ids_round_trip() {
        for f in Format::ALL {
            assert_eq!(Format::from_id(f.id()), Some(f), "{f:?}");
        }
    }

    #[test]
    fn labels_are_upper_case_with_gzip_special_case() {
        assert_eq!(Format::Mp3.label(), "MP3");
        assert_eq!(Format::Gz.label(), "GZIP");
        assert_eq!(Format::Gz.extension(), "tar.gz");
    }

    #[test]
    fn gif_counts_as_video() {
        assert_eq!(Format::Gif.category(), Category::Video);
        assert_eq!(Format::Svg.category(), Category::Image);
    }
}
