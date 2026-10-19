//! File actions offered on the Actions wheel (SPEC §7).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    AirDrop,
    Float,
    Copy,
    Move,
    CopyPath,
    Terminal,
}

impl Action {
    pub const ALL: [Action; 6] = [
        Action::AirDrop,
        Action::Float,
        Action::Copy,
        Action::Move,
        Action::CopyPath,
        Action::Terminal,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Action::AirDrop => "airDrop",
            Action::Float => "float",
            Action::Copy => "copy",
            Action::Move => "move",
            Action::CopyPath => "copyPath",
            Action::Terminal => "terminal",
        }
    }

    pub fn from_id(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(s))
    }

    pub fn title(self) -> &'static str {
        match self {
            Action::AirDrop => "AIRDROP",
            Action::Float => "FLOAT",
            Action::Copy => "COPY",
            Action::Move => "MOVE",
            Action::CopyPath => "COPY PATH",
            Action::Terminal => "TERMINAL",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::AirDrop => "AirDrop",
            Action::Float => "Float",
            Action::Copy => "Copy",
            Action::Move => "Move",
            Action::CopyPath => "Copy Path",
            Action::Terminal => "Terminal",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Action::AirDrop => "airplayaudio",
            Action::Float => "pin.fill",
            Action::Copy => "doc.on.doc",
            Action::Move => "folder",
            Action::CopyPath => "doc.text",
            Action::Terminal => "terminal",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::AirDrop => "Share files and folders with nearby devices.",
            Action::Float => "Keep files in a floating shelf and drag them into other apps.",
            Action::Copy => "Copy files to the clipboard, ready to paste in Finder.",
            Action::Move => "Choose a folder and move the originals there.",
            Action::CopyPath => "Copy full paths as text, one per line.",
            Action::Terminal => "Open the folder, or a file's folder, in a terminal.",
        }
    }
}

/// Text for Copy Path: one POSIX path per line.
pub fn paths_as_text<P: AsRef<std::path::Path>>(paths: &[P]) -> String {
    paths
        .iter()
        .map(|p| p.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Folder a terminal should open for an item: the item itself if it is a folder, else its parent.
pub fn terminal_directory(path: &std::path::Path, is_dir: bool) -> std::path::PathBuf {
    if is_dir {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn six_actions_in_wheel_order() {
        let ids: Vec<_> = Action::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids, ["airDrop", "float", "copy", "move", "copyPath", "terminal"]);
        for a in Action::ALL {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn copy_path_joins_with_newlines() {
        let text = paths_as_text(&[Path::new("/a/b c.txt"), Path::new("/d")]);
        assert_eq!(text, "/a/b c.txt\n/d");
    }

    #[test]
    fn terminal_opens_parent_of_files() {
        assert_eq!(
            terminal_directory(Path::new("/x/y/z.txt"), false),
            PathBuf::from("/x/y")
        );
        assert_eq!(terminal_directory(Path::new("/x/y"), true), PathBuf::from("/x/y"));
    }
}
