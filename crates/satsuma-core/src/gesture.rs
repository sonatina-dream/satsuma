//! Drag-gesture state machine (SPEC §2, §10).
//!
//! The app samples the system state (mouse button, modifier keys, drag pasteboard) on a timer
//! and feeds [`Sample`]s to [`DragTracker::update`], which says when to open, switch or close
//! the wheel. No Accessibility permission is needed for any of the inputs.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WheelMode {
    Conversions,
    Tools,
    Actions,
}

impl WheelMode {
    pub const ALL: [WheelMode; 3] = [WheelMode::Conversions, WheelMode::Tools, WheelMode::Actions];

    pub fn name(self) -> &'static str {
        match self {
            WheelMode::Conversions => "Convert",
            WheelMode::Tools => "Tools",
            WheelMode::Actions => "Actions",
        }
    }
}

/// A set of modifier keys.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1);
    pub const CONTROL: Mods = Mods(2);
    pub const OPTION: Mods = Mods(4);
    pub const COMMAND: Mods = Mods(8);

    const NAMES: [(Mods, &'static str, &'static str); 4] = [
        (Mods::CONTROL, "control", "⌃"),
        (Mods::OPTION, "option", "⌥"),
        (Mods::SHIFT, "shift", "⇧"),
        (Mods::COMMAND, "command", "⌘"),
    ];

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u8) -> Mods {
        Mods(bits & 0b1111)
    }

    pub const fn union(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Key names in macOS menu order, e.g. `["option", "shift"]`.
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(m, _, _)| self.contains(*m))
            .map(|(_, n, _)| *n)
            .collect()
    }

    /// Symbol string such as "⌥⇧".
    pub fn symbols(self) -> String {
        Self::NAMES
            .iter()
            .filter(|(m, _, _)| self.contains(*m))
            .map(|(_, _, s)| *s)
            .collect()
    }

    /// Human description such as "Option+Shift".
    pub fn describe(self) -> String {
        if self.is_empty() {
            return "None".into();
        }
        self.names()
            .iter()
            .map(|n| {
                let mut c = n.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Mods> {
        let mut m = Mods::NONE;
        for n in names {
            let key = n.trim().to_ascii_lowercase();
            let found = Self::NAMES.iter().find(|(_, name, _)| {
                *name == key
                    || matches!(
                        (key.as_str(), *name),
                        ("ctrl", "control") | ("alt", "option") | ("cmd", "command") | ("meta", "command")
                    )
            })?;
            m = m.union(found.0);
        }
        Some(m)
    }
}

impl fmt::Debug for Mods {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mods({})", self.describe())
    }
}

impl Serialize for Mods {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Mods {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names: Vec<String> = Vec::deserialize(d)?;
        Mods::from_names(names.iter().map(String::as_str))
            .ok_or_else(|| serde::de::Error::custom("unknown modifier key name"))
    }
}

/// Which modifier combination opens which wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelCombos {
    pub conversions: Mods,
    pub tools: Mods,
    pub actions: Mods,
}

impl Default for WheelCombos {
    fn default() -> Self {
        WheelCombos {
            conversions: Mods::SHIFT,
            tools: Mods::COMMAND,
            actions: Mods::CONTROL,
        }
    }
}

impl WheelCombos {
    pub fn combo(&self, mode: WheelMode) -> Mods {
        match mode {
            WheelMode::Conversions => self.conversions,
            WheelMode::Tools => self.tools,
            WheelMode::Actions => self.actions,
        }
    }

    pub fn set(&mut self, mode: WheelMode, mods: Mods) {
        match mode {
            WheelMode::Conversions => self.conversions = mods,
            WheelMode::Tools => self.tools = mods,
            WheelMode::Actions => self.actions = mods,
        }
    }

    /// The wheel whose combination exactly equals the held modifiers.
    pub fn mode_for(&self, held: Mods) -> Option<WheelMode> {
        if held.is_empty() {
            return None;
        }
        WheelMode::ALL.into_iter().find(|m| self.combo(*m) == held)
    }
}

/// One observation of the system state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Left mouse button is down.
    pub mouse_down: bool,
    /// `NSPasteboard(name: .drag).changeCount`.
    pub drag_change_count: i64,
    /// The drag pasteboard holds file URLs.
    pub drag_has_files: bool,
    pub mods: Mods,
    /// Pointer location in screen coordinates.
    pub location: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    Open { mode: WheelMode, at: (f64, f64) },
    Switch { mode: WheelMode },
    Close,
}

#[derive(Debug, Default)]
pub struct DragTracker {
    baseline: Option<i64>,
    file_drag: bool,
    prev_mods: Mods,
    wheel: Option<WheelMode>,
}

impl DragTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_file_drag_active(&self) -> bool {
        self.file_drag
    }

    pub fn wheel(&self) -> Option<WheelMode> {
        self.wheel
    }

    /// The app closed the wheel itself (e.g. after a drop); forget it without emitting Close.
    pub fn wheel_closed(&mut self) {
        self.wheel = None;
    }

    /// Whether the drag pasteboard should be inspected for file URLs for this change count.
    pub fn needs_type_check(&self, mouse_down: bool, change_count: i64) -> bool {
        mouse_down && !self.file_drag && self.baseline.is_some_and(|b| b != change_count)
    }

    pub fn update(&mut self, s: &Sample, combos: &WheelCombos) -> Option<Effect> {
        if !s.mouse_down {
            self.baseline = Some(s.drag_change_count);
            self.file_drag = false;
            self.prev_mods = s.mods;
            return self.wheel.take().map(|_| Effect::Close);
        }

        // If we start observing mid-press, treat the current pasteboard as the baseline so a
        // stale drag from before launch cannot open a wheel.
        let baseline = *self.baseline.get_or_insert(s.drag_change_count);
        let became_active =
            !self.file_drag && s.drag_change_count != baseline && s.drag_has_files;
        if became_active {
            self.file_drag = true;
        }
        let mods_changed = s.mods != self.prev_mods;
        self.prev_mods = s.mods;

        if !self.file_drag || !(mods_changed || became_active) {
            return None;
        }
        let mode = combos.mode_for(s.mods)?;
        match self.wheel {
            None => {
                self.wheel = Some(mode);
                Some(Effect::Open { mode, at: s.location })
            }
            Some(current) if current != mode => {
                self.wheel = Some(mode);
                Some(Effect::Switch { mode })
            }
            Some(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(mouse_down: bool, count: i64, files: bool, mods: Mods) -> Sample {
        Sample {
            mouse_down,
            drag_change_count: count,
            drag_has_files: files,
            mods,
            location: (100.0, 200.0),
        }
    }

    fn run(t: &mut DragTracker, s: Sample) -> Option<Effect> {
        t.update(&s, &WheelCombos::default())
    }

    #[test]
    fn shift_during_file_drag_opens_conversions_and_release_keeps_it() {
        let mut t = DragTracker::new();
        assert_eq!(run(&mut t, sample(false, 5, false, Mods::NONE)), None);
        assert_eq!(run(&mut t, sample(true, 5, false, Mods::NONE)), None); // press
        assert_eq!(run(&mut t, sample(true, 6, true, Mods::NONE)), None); // drag begins
        assert!(t.is_file_drag_active());
        assert_eq!(
            run(&mut t, sample(true, 6, true, Mods::SHIFT)),
            Some(Effect::Open { mode: WheelMode::Conversions, at: (100.0, 200.0) })
        );
        // Releasing shift does not close.
        assert_eq!(run(&mut t, sample(true, 6, true, Mods::NONE)), None);
        assert_eq!(t.wheel(), Some(WheelMode::Conversions));
        // Command switches to tools, Control to actions.
        assert_eq!(
            run(&mut t, sample(true, 6, true, Mods::COMMAND)),
            Some(Effect::Switch { mode: WheelMode::Tools })
        );
        assert_eq!(run(&mut t, sample(true, 6, true, Mods::NONE)), None);
        assert_eq!(
            run(&mut t, sample(true, 6, true, Mods::CONTROL)),
            Some(Effect::Switch { mode: WheelMode::Actions })
        );
        // Tapping the same wheel again does nothing.
        assert_eq!(run(&mut t, sample(true, 6, true, Mods::NONE)), None);
        assert_eq!(run(&mut t, sample(true, 6, true, Mods::CONTROL)), None);
        // Mouse up closes.
        assert_eq!(run(&mut t, sample(false, 6, true, Mods::NONE)), Some(Effect::Close));
        assert_eq!(run(&mut t, sample(false, 6, true, Mods::NONE)), None);
    }

    #[test]
    fn non_file_drags_and_plain_clicks_never_open() {
        let mut t = DragTracker::new();
        run(&mut t, sample(false, 1, false, Mods::NONE));
        // Click-and-hold without a drag.
        assert_eq!(run(&mut t, sample(true, 1, false, Mods::SHIFT)), None);
        run(&mut t, sample(false, 1, false, Mods::NONE));
        // A text drag (pasteboard changed but no files).
        run(&mut t, sample(true, 1, false, Mods::NONE));
        assert_eq!(run(&mut t, sample(true, 2, false, Mods::SHIFT)), None);
    }

    #[test]
    fn modifier_held_before_drag_opens_when_drag_starts() {
        let mut t = DragTracker::new();
        run(&mut t, sample(false, 1, false, Mods::SHIFT));
        run(&mut t, sample(true, 1, false, Mods::SHIFT));
        assert_eq!(
            run(&mut t, sample(true, 2, true, Mods::SHIFT)),
            Some(Effect::Open { mode: WheelMode::Conversions, at: (100.0, 200.0) })
        );
    }

    #[test]
    fn combos_need_exact_match() {
        let mut t = DragTracker::new();
        run(&mut t, sample(false, 1, false, Mods::NONE));
        run(&mut t, sample(true, 2, true, Mods::NONE));
        assert_eq!(run(&mut t, sample(true, 2, true, Mods::SHIFT.union(Mods::COMMAND))), None);
        assert_eq!(run(&mut t, sample(true, 2, true, Mods::OPTION)), None);

        let combos = WheelCombos {
            conversions: Mods::SHIFT,
            tools: Mods::OPTION.union(Mods::SHIFT),
            actions: Mods::CONTROL,
        };
        let mut t = DragTracker::new();
        t.update(&sample(false, 1, false, Mods::NONE), &combos);
        t.update(&sample(true, 2, true, Mods::NONE), &combos);
        assert_eq!(
            t.update(&sample(true, 2, true, Mods::OPTION.union(Mods::SHIFT)), &combos),
            Some(Effect::Open { mode: WheelMode::Tools, at: (100.0, 200.0) })
        );
    }

    #[test]
    fn stale_pasteboard_at_launch_is_ignored() {
        let mut t = DragTracker::new();
        // First sample arrives with the button already down.
        assert_eq!(run(&mut t, sample(true, 9, true, Mods::SHIFT)), None);
        assert!(!t.is_file_drag_active());
    }

    #[test]
    fn wheel_closed_by_app_does_not_emit_close() {
        let mut t = DragTracker::new();
        run(&mut t, sample(false, 1, false, Mods::NONE));
        run(&mut t, sample(true, 2, true, Mods::NONE));
        run(&mut t, sample(true, 2, true, Mods::SHIFT));
        t.wheel_closed();
        assert_eq!(run(&mut t, sample(false, 2, true, Mods::NONE)), None);
    }

    #[test]
    fn type_check_only_when_pasteboard_changed_during_press() {
        let mut t = DragTracker::new();
        run(&mut t, sample(false, 3, false, Mods::NONE));
        assert!(!t.needs_type_check(false, 4));
        assert!(!t.needs_type_check(true, 3));
        assert!(t.needs_type_check(true, 4));
    }

    #[test]
    fn mods_names_round_trip_and_describe() {
        let m = Mods::OPTION.union(Mods::SHIFT);
        assert_eq!(m.names(), vec!["option", "shift"]);
        assert_eq!(m.describe(), "Option+Shift");
        assert_eq!(m.symbols(), "⌥⇧");
        assert_eq!(Mods::from_names(["shift", "alt"]), Some(m));
        assert_eq!(Mods::from_names(["hyper"]), None);
        let json = serde_json::to_string(&WheelCombos::default()).unwrap();
        assert_eq!(
            json,
            r#"{"conversions":["shift"],"tools":["command"],"actions":["control"]}"#
        );
        let back: WheelCombos = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WheelCombos::default());
    }
}
