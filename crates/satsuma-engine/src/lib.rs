//! Local conversion backends for Satsuma.
//!
//! Everything runs on the Mac: `sips` (ImageIO), `textutil`, `tar`, PDFKit, CoreGraphics,
//! CoreText, and a user-installed ffmpeg for audio and video.

pub mod archive;
pub mod capability;
pub mod doc;
pub mod docx;
pub mod env;
pub mod epub;
pub mod html;
pub mod image;
pub mod imaging;
pub mod media;
pub mod metadata;
pub mod ocr;
pub mod pdf;
pub mod photo;
pub mod plan;
pub mod probe;
pub mod process;
pub mod run;
pub mod sheet;
pub mod subtitle;

pub use env::Env;
pub use plan::{detect_format, plan, Job, Operation, Plan};
pub use process::Cancel;
pub use run::{run_job, run_plan, BatchProgress, BatchReport, Context};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("Cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Message(s.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
