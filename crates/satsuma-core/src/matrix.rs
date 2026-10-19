//! The conversion matrix: which output formats are offered for an input (SPEC §4).

use crate::format::Format::{self, *};

const IMG_IN: &[Format] = &[Jpg, Png, Webp, Heic, Tiff, Svg, Avif, Bmp];
const IMG_OUT: &[Format] = &[Jpg, Png, Webp, Heic, Tiff, Avif, Bmp, Pdf];
const AUD: &[Format] = &[Mp3, M4a, Wav, Flac, Ogg, Opus, Aiff, Wma];
const VID: &[Format] = &[Mp4, Mov, Mkv, Webm, Avi, Wmv];
const SUB: &[Format] = &[Srt, Vtt];
const DOC_IN: &[Format] = &[Docx, Pages, Md, Rtf, Rtfd, TextClipping];
const SHEET: &[Format] = &[Xlsx, Xls, Ods, Csv, Tsv];
const DOC_OUT: &[Format] = &[Docx, Pages, Pdf, Txt, Md, Html, Epub, Png, Jpg];
const ARCHIVE_IN: &[Format] = &[Zip, Tar, Gz, Rar];
const ARCHIVE_OUT: &[Format] = &[Zip, Tar, Gz];

/// Formats that can be read but are never offered as an output.
pub fn is_input_only(f: Format) -> bool {
    matches!(f, Rar | Ppt | Xls | Rtf | Rtfd | TextClipping)
}

fn raw_targets(f: Format) -> Vec<Format> {
    if IMG_IN.contains(&f) {
        // Tangerine 2.0 also turns images into Word documents (recognised text), E10.
        let mut v = IMG_OUT.to_vec();
        v.push(Docx);
        v
    } else if AUD.contains(&f) {
        AUD.to_vec()
    } else if VID.contains(&f) {
        let mut v = VID.to_vec();
        v.extend([Gif, Mp3]);
        v
    } else if f == Gif {
        VID.to_vec()
    } else if f == Pdf {
        vec![Docx, Pages, Pptx, Xlsx, Jpg, Png, Epub, Txt, Md]
    } else if matches!(f, Ppt | Pptx) {
        vec![Pdf, Jpg, Png]
    } else if SHEET.contains(&f) {
        let mut v = SHEET.to_vec();
        v.push(Pdf);
        v
    } else if DOC_IN.contains(&f) {
        DOC_OUT.to_vec()
    } else if f == Epub {
        vec![Pdf, Docx, Pages, Txt, Md, Html]
    } else if f == Txt {
        vec![Docx, Pages, Pdf, Md, Html, Epub, Jpg, Png]
    } else if SUB.contains(&f) {
        vec![Srt, Vtt, Txt]
    } else if ARCHIVE_IN.contains(&f) {
        ARCHIVE_OUT.to_vec()
    } else {
        Vec::new()
    }
}

/// Output formats offered for a single input, in wheel order.
pub fn conversion_targets(f: Format) -> Vec<Format> {
    raw_targets(f)
        .into_iter()
        .filter(|t| *t != f && !is_input_only(*t))
        .collect()
}

/// Output formats offered for a selection (SPEC §4.1).
///
/// A target is offered when every file can convert to it or already has that format, and at
/// least one file actually needs converting. Petal order follows the first such file.
pub fn conversion_targets_for_selection(formats: &[Format]) -> Vec<Format> {
    // Build the candidate list from the formats in selection order, keeping first-seen order.
    let mut candidates: Vec<Format> = Vec::new();
    for f in formats {
        for t in conversion_targets(*f) {
            if !candidates.contains(&t) {
                candidates.push(t);
            }
        }
    }
    candidates
        .into_iter()
        .filter(|t| {
            let all_ok = formats
                .iter()
                .all(|f| f == t || conversion_targets(*f).contains(t));
            let some_needs = formats.iter().any(|f| f != t);
            all_ok && some_needs
        })
        .collect()
}

/// Total number of (input, output) conversion paths in the matrix.
pub fn total_conversion_paths() -> usize {
    Format::ALL.iter().map(|f| conversion_targets(*f).len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_matches_the_original() {
        // The website matrix (E1) has exactly the advertised 275 paths (E9); the 2.0 app adds
        // image → DOCX for the 8 image inputs (E10).
        assert_eq!(total_conversion_paths(), 275 + 8);
    }

    #[test]
    fn jpg_wheel_matches_the_screen_recording() {
        // E10: PNG, WEBP, HEIC, TIFF, AVIF, BMP, PDF, DOCX clockwise from the top.
        assert_eq!(conversion_targets(Jpg), vec![Png, Webp, Heic, Tiff, Avif, Bmp, Pdf, Docx]);
    }

    #[test]
    fn wav_offers_other_audio_formats_in_order() {
        assert_eq!(
            conversion_targets(Wav),
            vec![Mp3, M4a, Flac, Ogg, Opus, Aiff, Wma]
        );
    }

    #[test]
    fn svg_is_input_only_and_offers_all_image_outputs() {
        let mut expected = IMG_OUT.to_vec();
        expected.push(Docx);
        assert_eq!(conversion_targets(Svg), expected);
        assert!(!conversion_targets(Png).contains(&Svg));
    }

    #[test]
    fn video_offers_gif_and_mp3() {
        let t = conversion_targets(Mov);
        assert_eq!(t, vec![Mp4, Mkv, Webm, Avi, Wmv, Gif, Mp3]);
        assert_eq!(conversion_targets(Gif), VID.to_vec());
    }

    #[test]
    fn input_only_formats_are_never_targets() {
        for f in Format::ALL {
            for t in conversion_targets(f) {
                assert!(!is_input_only(t), "{f:?} -> {t:?}");
                assert_ne!(f, t);
            }
        }
    }

    #[test]
    fn spreadsheets_drop_xls_and_add_pdf() {
        assert_eq!(conversion_targets(Xls), vec![Xlsx, Ods, Csv, Tsv, Pdf]);
        assert_eq!(conversion_targets(Csv), vec![Xlsx, Ods, Tsv, Pdf]);
    }

    #[test]
    fn archives_and_subtitles() {
        assert_eq!(conversion_targets(Rar), vec![Zip, Tar, Gz]);
        assert_eq!(conversion_targets(Gz), vec![Zip, Tar]);
        assert_eq!(conversion_targets(Srt), vec![Vtt, Txt]);
    }

    #[test]
    fn unsupported_inputs_have_no_targets() {
        assert!(conversion_targets(Html).is_empty());
    }

    #[test]
    fn selection_of_same_format_equals_single() {
        assert_eq!(
            conversion_targets_for_selection(&[Jpg, Jpg]),
            conversion_targets(Jpg)
        );
    }

    #[test]
    fn mixed_image_selection_keeps_shared_targets() {
        let t = conversion_targets_for_selection(&[Jpg, Png]);
        // PNG is offered (the PNG file is skipped), as is JPG (the JPG file is skipped).
        assert_eq!(t, vec![Png, Webp, Heic, Tiff, Avif, Bmp, Pdf, Docx, Jpg]);
    }

    #[test]
    fn incompatible_selection_has_no_targets() {
        assert!(conversion_targets_for_selection(&[Jpg, Mp3]).is_empty());
        assert!(conversion_targets_for_selection(&[]).is_empty());
    }
}
