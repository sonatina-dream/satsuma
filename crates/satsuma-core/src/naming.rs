//! Output file naming (SPEC §5). Results never overwrite: collisions get " 2", " 3", …

use crate::format::Format;

/// Strips `.tar.gz`, `.tgz` or the last extension. Names that are only an extension
/// (".bashrc") are kept whole.
pub fn stem_of(file_name: &str) -> &str {
    let lower = file_name.to_ascii_lowercase();
    let cut = if lower.ends_with(".tar.gz") {
        Some(file_name.len() - ".tar.gz".len())
    } else if lower.ends_with(".tgz") {
        Some(file_name.len() - ".tgz".len())
    } else {
        file_name.rfind('.')
    };
    match cut {
        Some(0) | None => file_name,
        Some(i) => &file_name[..i],
    }
}

fn with_ext(base: &str, ext: Option<&str>) -> String {
    match ext {
        Some(e) => format!("{base}.{e}"),
        None => base.to_string(),
    }
}

/// First free name of the form `<base>.<ext>`, `<base> 2.<ext>`, `<base> 3.<ext>` …
pub fn first_free(base: &str, ext: Option<&str>, taken: &dyn Fn(&str) -> bool) -> String {
    let candidate = with_ext(base, ext);
    if !taken(&candidate) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = with_ext(&format!("{base} {n}"), ext);
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Name for converting `file_name` to `target`: `photo.png` → `photo.jpg` / `photo 2.jpg`.
pub fn conversion_output_name(
    file_name: &str,
    target: Format,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    first_free(stem_of(file_name), Some(target.extension()), taken)
}

/// Name for a tool result: `photo.jpg` → `photo Compressed.jpg`; `None` format means a folder.
pub fn tool_output_name(
    file_name: &str,
    label: &str,
    output: Option<Format>,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let base = format!("{} {}", stem_of(file_name), label);
    first_free(&base, output.map(|f| f.extension()), taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn set(names: &[&str]) -> impl Fn(&str) -> bool {
        let s: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        move |n: &str| s.contains(&n.to_lowercase())
    }

    #[test]
    fn stems() {
        assert_eq!(stem_of("photo.jpg"), "photo");
        assert_eq!(stem_of("my.photo.JPG"), "my.photo");
        assert_eq!(stem_of("backup.tar.gz"), "backup");
        assert_eq!(stem_of("backup.TGZ"), "backup");
        assert_eq!(stem_of("README"), "README");
        assert_eq!(stem_of(".bashrc"), ".bashrc");
    }

    #[test]
    fn conversion_names_avoid_collisions() {
        let taken = set(&["sample.wav", "sample.mp3"]);
        assert_eq!(
            conversion_output_name("sample.wav", Format::Mp3, &taken),
            "sample 2.mp3"
        );
        let taken = set(&["sample.mp3", "sample 2.mp3"]);
        assert_eq!(
            conversion_output_name("sample.wav", Format::Mp3, &taken),
            "sample 3.mp3"
        );
        assert_eq!(
            conversion_output_name("sample.wav", Format::Flac, &set(&[])),
            "sample.flac"
        );
    }

    #[test]
    fn gz_target_writes_tar_gz() {
        assert_eq!(
            conversion_output_name("project.zip", Format::Gz, &set(&[])),
            "project.tar.gz"
        );
        assert_eq!(
            conversion_output_name("project.tar.gz", Format::Zip, &set(&[])),
            "project.zip"
        );
    }

    #[test]
    fn tool_names() {
        assert_eq!(
            tool_output_name("photo.jpg", "Compressed", Some(Format::Jpg), &set(&[])),
            "photo Compressed.jpg"
        );
        assert_eq!(
            tool_output_name(
                "photo.jpg",
                "Compressed",
                Some(Format::Jpg),
                &set(&["photo compressed.jpg"])
            ),
            "photo Compressed 2.jpg"
        );
        assert_eq!(
            tool_output_name("little-project.zip", "Extracted", None, &set(&[])),
            "little-project Extracted"
        );
    }
}
