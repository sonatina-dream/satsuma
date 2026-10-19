//! File tools offered on the Tools wheel (SPEC §6).

use crate::format::Format::{self, *};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    Compress,
    RemoveMetadata,
    EditImage,
    AnnotateImage,
    FrameImage,
    CropImage,
    RedactImage,
    NormalizeAudio,
    AudioToVideo,
    TrimAudio,
    AudioChannels,
    RedactAudio,
    MuteVideo,
    TrimVideo,
    CropVideo,
    ChangeVideoSpeed,
    VideoSnapshots,
    SplitVideo,
    RedactVideo,
    JoinVideos,
    SplitPdf,
    OrganizePdf,
    MergePdf,
    CreatePdf,
    CreateCollage,
    ExtractArchive,
}

/// What a tool writes next to the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutput {
    /// Same format as the input (with `wav` compressing to `flac`).
    SameFormat,
    /// A fixed format.
    Fixed(Format),
    /// A new folder.
    Folder,
}

pub struct ToolInfo {
    pub id: &'static str,
    /// Upper-case title shown on the petal.
    pub title: &'static str,
    /// Human name, e.g. for menus and progress.
    pub name: &'static str,
    /// SF Symbol name.
    pub symbol: &'static str,
    /// Word appended to output names (`photo Compressed.jpg`).
    pub label: &'static str,
    pub output: ToolOutput,
    /// Progress title while running, e.g. "Compressing".
    pub verb: &'static str,
}

impl Tool {
    pub const ALL: [Tool; 26] = [
        Tool::Compress,
        Tool::RemoveMetadata,
        Tool::EditImage,
        Tool::AnnotateImage,
        Tool::FrameImage,
        Tool::CropImage,
        Tool::RedactImage,
        Tool::NormalizeAudio,
        Tool::AudioToVideo,
        Tool::TrimAudio,
        Tool::AudioChannels,
        Tool::RedactAudio,
        Tool::MuteVideo,
        Tool::TrimVideo,
        Tool::CropVideo,
        Tool::ChangeVideoSpeed,
        Tool::VideoSnapshots,
        Tool::SplitVideo,
        Tool::RedactVideo,
        Tool::JoinVideos,
        Tool::SplitPdf,
        Tool::OrganizePdf,
        Tool::MergePdf,
        Tool::CreatePdf,
        Tool::CreateCollage,
        Tool::ExtractArchive,
    ];

    pub fn info(self) -> ToolInfo {
        use Tool::*;
        use ToolOutput::*;
        let (id, title, name, symbol, label, output, verb) = match self {
            Compress => ("compress", "COMPRESS", "Compress", "arrow.down.right.and.arrow.up.left", "Compressed", SameFormat, "Compressing"),
            RemoveMetadata => ("removeMetadata", "METADATA", "Remove Metadata", "tag", "No Metadata", SameFormat, "Removing metadata"),
            EditImage => ("editImage", "EDIT", "Edit Photo", "slider.horizontal.3", "Edited", SameFormat, "Editing photo"),
            AnnotateImage => ("annotateImage", "ANNOTATE", "Annotate Photo", "pencil.and.outline", "Annotated", Fixed(Png), "Annotating photo"),
            FrameImage => ("frameImage", "ADD BG", "Add Background", "photo.artframe", "With Background", Fixed(Png), "Adding background"),
            CropImage => ("cropImage", "CROP", "Crop Image", "crop", "Cropped", SameFormat, "Cropping"),
            RedactImage => ("redactImage", "REDACT", "Redact Photo", "eye.slash", "Redacted", SameFormat, "Redacting"),
            NormalizeAudio => ("normalizeAudio", "NORMALIZE", "Normalize Volume", "waveform", "Normalized", SameFormat, "Normalizing volume"),
            AudioToVideo => ("audioToVideo", "VISUALIZE", "Audio Visualizer", "waveform", "Visualizer", Fixed(Mp4), "Creating visualization"),
            TrimAudio => ("trimAudio", "TRIM", "Trim Silence", "scissors", "Trimmed", SameFormat, "Trimming"),
            AudioChannels => ("audioChannels", "CHANNELS", "Convert Audio Channels", "hifispeaker.2", "Mono", SameFormat, "Converting channels"),
            RedactAudio => ("redactAudio", "BLEEP", "Bleep Audio", "waveform.badge.minus", "Bleeped", SameFormat, "Bleeping"),
            MuteVideo => ("muteVideo", "MUTE", "Remove Audio", "speaker.slash.fill", "Muted", SameFormat, "Removing audio"),
            TrimVideo => ("trimVideo", "TRIM", "Trim Video", "scissors", "Trimmed", SameFormat, "Trimming"),
            CropVideo => ("cropVideo", "CROP", "Crop Video", "crop", "Cropped", SameFormat, "Cropping"),
            ChangeVideoSpeed => ("changeVideoSpeed", "SPEED", "Change Video Speed", "speedometer", "Speed 2x", SameFormat, "Changing speed"),
            VideoSnapshots => ("videoSnapshots", "SNAPSHOT", "Video Snapshot", "camera", "Snapshot", Fixed(Png), "Saving snapshots"),
            SplitVideo => ("splitVideo", "SPLIT", "Split Video", "film", "Clips", Folder, "Splitting video"),
            RedactVideo => ("redactVideo", "REDACT", "Redact Video", "eye.slash", "Redacted", SameFormat, "Redacting"),
            JoinVideos => ("joinVideos", "JOIN", "Join Videos", "film.stack", "Joined", Fixed(Mp4), "Joining videos"),
            SplitPdf => ("splitPDF", "SPLIT", "Split PDF", "rectangle.split.2x1", "Pages", Folder, "Splitting PDF"),
            OrganizePdf => ("organizePDF", "ORGANIZE", "Organize PDF", "square.grid.2x2", "Organized", Fixed(Pdf), "Organizing PDF"),
            MergePdf => ("mergePDF", "MERGE PDF", "Merge PDFs", "doc.on.doc", "Merged", Fixed(Pdf), "Merging PDFs"),
            CreatePdf => ("createPDF", "CREATE PDF", "Create PDF", "doc.badge.plus", "Images", Fixed(Pdf), "Creating PDF"),
            CreateCollage => ("createCollage", "COLLAGE", "Create Collage", "rectangle.3.group", "Collage", Fixed(Png), "Creating collage"),
            ExtractArchive => ("extractArchive", "EXTRACT", "Extract Archive", "archivebox", "Extracted", Folder, "Extracting archive"),
        };
        ToolInfo { id, title, name, symbol, label, output, verb }
    }

    pub fn id(self) -> &'static str {
        self.info().id
    }

    pub fn from_id(s: &str) -> Option<Tool> {
        Tool::ALL
            .into_iter()
            .find(|t| t.id().eq_ignore_ascii_case(s))
    }

    /// Tools that work on a multi-file selection.
    pub fn is_batch_capable(self) -> bool {
        use Tool::*;
        matches!(
            self,
            Compress
                | RemoveMetadata
                | CropImage
                | NormalizeAudio
                | MuteVideo
                | SplitPdf
                | ExtractArchive
                | CreatePdf
                | MergePdf
                | JoinVideos
                | CreateCollage
        )
    }

    /// Tools that combine several inputs into one output.
    pub fn combines_inputs(self) -> bool {
        matches!(
            self,
            Tool::CreatePdf | Tool::MergePdf | Tool::JoinVideos | Tool::CreateCollage
        )
    }

    /// Tools whose original opens an editor window before saving.
    pub fn is_interactive(self) -> bool {
        use Tool::*;
        matches!(
            self,
            EditImage
                | AnnotateImage
                | FrameImage
                | CropImage
                | RedactImage
                | RedactAudio
                | TrimVideo
                | CropVideo
                | RedactVideo
                | OrganizePdf
        )
    }

    /// Interactive tools whose editor window Satsuma has.
    pub fn has_editor(self) -> bool {
        matches!(self, Tool::CropImage | Tool::FrameImage | Tool::EditImage)
    }

    /// Output format for a given input format, or `None` when the tool writes a folder.
    pub fn output_format(self, input: Format) -> Option<Format> {
        match self.info().output {
            ToolOutput::SameFormat => Some(if self == Tool::Compress && input == Wav {
                Flac
            } else {
                input
            }),
            ToolOutput::Fixed(f) => Some(f),
            ToolOutput::Folder => None,
        }
    }
}

/// Tools for a single format and a selection of `count` files, in wheel order (before
/// filtering to batch-capable tools).
fn tools_for_format(f: Format, count: usize) -> Vec<Tool> {
    use Tool::*;
    let many = count > 1;
    match f {
        Jpg | Png | Webp | Heic | Tiff => {
            if many {
                vec![Compress, RemoveMetadata, CropImage, CreatePdf, CreateCollage]
            } else {
                vec![Compress, RemoveMetadata, EditImage, AnnotateImage, FrameImage, CropImage, RedactImage]
            }
        }
        Svg => {
            if many {
                vec![CreatePdf, CreateCollage]
            } else {
                vec![]
            }
        }
        Mp3 | M4a | Wav | Flac => vec![
            Compress,
            RemoveMetadata,
            NormalizeAudio,
            AudioToVideo,
            TrimAudio,
            AudioChannels,
            RedactAudio,
        ],
        Mp4 | Mov | Mkv => {
            if many {
                vec![Compress, RemoveMetadata, MuteVideo, TrimVideo, CropVideo, ChangeVideoSpeed, JoinVideos]
            } else {
                vec![
                    Compress,
                    RemoveMetadata,
                    MuteVideo,
                    TrimVideo,
                    CropVideo,
                    ChangeVideoSpeed,
                    VideoSnapshots,
                    SplitVideo,
                    RedactVideo,
                ]
            }
        }
        Gif => vec![RemoveMetadata],
        Pdf => {
            if many {
                vec![Compress, RemoveMetadata, SplitPdf, MergePdf]
            } else {
                vec![Compress, RemoveMetadata, SplitPdf, OrganizePdf]
            }
        }
        Zip | Tar | Gz | Rar => vec![ExtractArchive],
        _ => vec![],
    }
}

/// Tools offered for `count` files of format `f` (SPEC §6).
pub fn tools_for(f: Format, count: usize) -> Vec<Tool> {
    if count == 0 {
        return Vec::new();
    }
    tools_for_format(f, count)
        .into_iter()
        .filter(|t| count == 1 || t.is_batch_capable())
        .collect()
}

/// Tools offered for a (possibly mixed) selection: the intersection of each format's list,
/// in the order of the first file's list.
pub fn tools_for_selection(formats: &[Format]) -> Vec<Tool> {
    let Some(first) = formats.first() else {
        return Vec::new();
    };
    let count = formats.len();
    let mut tools = tools_for(*first, count);
    for f in &formats[1..] {
        let other = tools_for(*f, count);
        tools.retain(|t| other.contains(t));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_26_tools_with_unique_ids() {
        let mut ids: Vec<_> = Tool::ALL.iter().map(|t| t.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 26);
        for t in Tool::ALL {
            assert_eq!(Tool::from_id(t.id()), Some(t));
        }
    }

    #[test]
    fn single_photo_gets_editing_tools() {
        assert_eq!(
            tools_for(Jpg, 1),
            vec![
                Tool::Compress,
                Tool::RemoveMetadata,
                Tool::EditImage,
                Tool::AnnotateImage,
                Tool::FrameImage,
                Tool::CropImage,
                Tool::RedactImage
            ]
        );
    }

    #[test]
    fn several_photos_get_batch_tools() {
        assert_eq!(
            tools_for(Png, 3),
            vec![
                Tool::Compress,
                Tool::RemoveMetadata,
                Tool::CropImage,
                Tool::CreatePdf,
                Tool::CreateCollage
            ]
        );
    }

    #[test]
    fn audio_batch_drops_non_batch_tools() {
        assert_eq!(
            tools_for(Mp3, 2),
            vec![Tool::Compress, Tool::RemoveMetadata, Tool::NormalizeAudio]
        );
    }

    #[test]
    fn video_batch_offers_join() {
        assert_eq!(
            tools_for(Mov, 2),
            vec![Tool::Compress, Tool::RemoveMetadata, Tool::MuteVideo, Tool::JoinVideos]
        );
    }

    #[test]
    fn pdf_tools_depend_on_count() {
        assert_eq!(
            tools_for(Pdf, 1),
            vec![Tool::Compress, Tool::RemoveMetadata, Tool::SplitPdf, Tool::OrganizePdf]
        );
        assert_eq!(
            tools_for(Pdf, 2),
            vec![Tool::Compress, Tool::RemoveMetadata, Tool::SplitPdf, Tool::MergePdf]
        );
    }

    #[test]
    fn svg_alone_has_no_tools_and_zero_count_is_empty() {
        assert!(tools_for(Svg, 1).is_empty());
        assert!(tools_for(Jpg, 0).is_empty());
        assert!(tools_for(Docx, 1).is_empty());
    }

    #[test]
    fn mixed_selection_intersects() {
        assert_eq!(
            tools_for_selection(&[Jpg, Svg]),
            vec![Tool::CreatePdf, Tool::CreateCollage]
        );
        assert_eq!(
            tools_for_selection(&[Mp4, Mov, Mkv]),
            vec![Tool::Compress, Tool::RemoveMetadata, Tool::MuteVideo, Tool::JoinVideos]
        );
        assert!(tools_for_selection(&[Jpg, Pdf]).len() == 2); // compress + metadata
    }

    #[test]
    fn compress_wav_outputs_flac() {
        assert_eq!(Tool::Compress.output_format(Wav), Some(Flac));
        assert_eq!(Tool::Compress.output_format(Mp3), Some(Mp3));
        assert_eq!(Tool::SplitVideo.output_format(Mp4), None);
        assert_eq!(Tool::VideoSnapshots.output_format(Mov), Some(Png));
        // E10: "Page 001 Cropped.jpg" and "Page 001 Cropped With Background.png".
        assert_eq!(Tool::CropImage.output_format(Jpg), Some(Jpg));
        assert_eq!(Tool::FrameImage.output_format(Jpg), Some(Png));
        assert_eq!(Tool::FrameImage.info().label, "With Background");
    }
}
