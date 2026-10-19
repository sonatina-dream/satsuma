//! Wheel contents and keyboard navigation (SPEC §2.1, §2.2).

use crate::actions::Action;
use crate::format::Format;
use crate::gesture::WheelMode;
use crate::matrix::conversion_targets_for_selection;
use crate::tools::{tools_for_selection, Tool};

/// One petal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WheelItem {
    Convert(Format),
    Tool(Tool),
    Action(Action),
}

impl WheelItem {
    /// Text on the petal and in the centre.
    pub fn title(&self) -> String {
        match self {
            WheelItem::Convert(f) => f.label(),
            WheelItem::Tool(t) => t.info().title.to_string(),
            WheelItem::Action(a) => a.title().to_string(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            WheelItem::Convert(f) => format!("Convert to {}", f.label()),
            WheelItem::Tool(t) => t.info().name.to_string(),
            WheelItem::Action(a) => a.name().to_string(),
        }
    }

    /// SF Symbol shown above the label (tools and actions only).
    pub fn symbol(&self) -> Option<&'static str> {
        match self {
            WheelItem::Convert(_) => None,
            WheelItem::Tool(t) => Some(t.info().symbol),
            WheelItem::Action(a) => Some(a.symbol()),
        }
    }
}

/// What a selection looks like to the wheel: the recognised format of each item, or `None`
/// for folders and unknown files.
pub fn wheel_items(
    mode: WheelMode,
    formats: &[Option<Format>],
    available: &dyn Fn(&WheelItem) -> bool,
) -> Vec<WheelItem> {
    let items: Vec<WheelItem> = match mode {
        WheelMode::Actions => Action::ALL.into_iter().map(WheelItem::Action).collect(),
        WheelMode::Conversions | WheelMode::Tools => {
            let known: Option<Vec<Format>> = formats.iter().copied().collect();
            match known {
                Some(fs) if !fs.is_empty() => {
                    if mode == WheelMode::Conversions {
                        conversion_targets_for_selection(&fs)
                            .into_iter()
                            .map(WheelItem::Convert)
                            .collect()
                    } else {
                        tools_for_selection(&fs).into_iter().map(WheelItem::Tool).collect()
                    }
                }
                _ => Vec::new(),
            }
        }
    };
    items.into_iter().filter(|i| available(i)).collect()
}

/// The wheel to open in keyboard/click mode: conversions, or actions when there are none.
pub fn initial_keyboard_mode(has_conversions: bool) -> WheelMode {
    if has_conversions {
        WheelMode::Conversions
    } else {
        WheelMode::Actions
    }
}

pub fn empty_message(mode: WheelMode) -> &'static str {
    match mode {
        WheelMode::Conversions => "No conversions for this format. Tap Control for file actions.",
        WheelMode::Tools => {
            "No tools for this format. Tap Shift to convert or Control for file actions."
        }
        WheelMode::Actions => "",
    }
}

/// Bytes as Finder shows them (decimal units): "823 KB", "1.2 MB", "143 MB".
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Text for the wheel's centre when no petal is highlighted (E10 shows "1.2 MB").
pub fn hub_caption(count: usize, total_bytes: Option<u64>) -> Vec<String> {
    let size = total_bytes.map(format_bytes);
    match (count, size) {
        (0, _) => Vec::new(),
        (1, Some(s)) => vec![s],
        (1, None) => vec!["FOLDER".into()],
        (n, Some(s)) => vec![format!("{n} ITEMS"), s],
        (n, None) => vec![format!("{n} ITEMS")],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Next,
    Previous,
    First,
    Last,
}

/// Keyboard navigation over `n` petals, wrapping around.
pub fn navigate(selected: Option<usize>, n: usize, key: NavKey) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match key {
        NavKey::First => 0,
        NavKey::Last => n - 1,
        NavKey::Next => selected.map_or(0, |s| (s + 1) % n),
        NavKey::Previous => selected.map_or(n - 1, |s| (s + n - 1) % n),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &WheelItem) -> bool {
        true
    }

    #[test]
    fn conversions_for_wav_match_demo_video_order() {
        let items = wheel_items(WheelMode::Conversions, &[Some(Format::Wav)], &all);
        let titles: Vec<_> = items.iter().map(|i| i.title()).collect();
        assert_eq!(titles, ["MP3", "M4A", "FLAC", "OGG", "OPUS", "AIFF", "WMA"]);
    }

    #[test]
    fn folders_get_only_actions() {
        assert!(wheel_items(WheelMode::Conversions, &[None], &all).is_empty());
        assert!(wheel_items(WheelMode::Tools, &[Some(Format::Jpg), None], &all).is_empty());
        assert_eq!(wheel_items(WheelMode::Actions, &[None], &all).len(), 6);
    }

    #[test]
    fn availability_filter_hides_items() {
        let no_webp = |i: &WheelItem| *i != WheelItem::Convert(Format::Webp);
        let items = wheel_items(WheelMode::Conversions, &[Some(Format::Png)], &no_webp);
        assert!(!items.contains(&WheelItem::Convert(Format::Webp)));
        assert!(items.contains(&WheelItem::Convert(Format::Jpg)));
    }

    #[test]
    fn tool_items_have_symbols() {
        let items = wheel_items(WheelMode::Tools, &[Some(Format::Pdf)], &all);
        assert_eq!(items[0].title(), "COMPRESS");
        assert_eq!(items[0].symbol(), Some("arrow.down.right.and.arrow.up.left"));
        assert_eq!(WheelItem::Convert(Format::Mp3).name(), "Convert to MP3");
    }

    #[test]
    fn navigation_wraps() {
        assert_eq!(navigate(Some(5), 6, NavKey::Next), Some(0));
        assert_eq!(navigate(Some(0), 6, NavKey::Previous), Some(5));
        assert_eq!(navigate(None, 6, NavKey::Next), Some(0));
        assert_eq!(navigate(Some(2), 6, NavKey::Last), Some(5));
        assert_eq!(navigate(Some(2), 6, NavKey::First), Some(0));
        assert_eq!(navigate(Some(0), 0, NavKey::Next), None);
    }

    #[test]
    fn byte_formatting() {
        assert_eq!(format_bytes(12), "12 bytes");
        assert_eq!(format_bytes(823_400), "823 KB");
        assert_eq!(format_bytes(1_234_567), "1.2 MB");
        assert_eq!(format_bytes(143_000_000), "143 MB");
        assert_eq!(format_bytes(999_960), "1.0 MB");
        assert_eq!(format_bytes(4_500_000_000), "4.5 GB");
        assert_eq!(hub_caption(1, Some(1_200_000)), vec!["1.2 MB"]);
        assert_eq!(hub_caption(3, Some(4_500_000)), vec!["3 ITEMS", "4.5 MB"]);
        assert!(hub_caption(0, None).is_empty());
    }

    #[test]
    fn keyboard_mode_falls_back_to_actions() {
        assert_eq!(initial_keyboard_mode(true), WheelMode::Conversions);
        assert_eq!(initial_keyboard_mode(false), WheelMode::Actions);
    }
}
