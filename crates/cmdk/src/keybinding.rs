use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::Path;

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use super::id::CommandId;
use super::registry::CommandRegistry;

// ── Named keys ──────────────────────────────────────────────────────────

/// Named keys that aren't representable as a single `char`.
///
/// String representations match DOM `KeyboardEvent.key` values for direct
/// frontend integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl NamedKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Escape => "Escape",
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
            Self::Enter => "Enter",
            Self::Tab => "Tab",
            Self::Space => "Space",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Escape" => Some(Self::Escape),
            "ArrowUp" => Some(Self::ArrowUp),
            "ArrowDown" => Some(Self::ArrowDown),
            "ArrowLeft" => Some(Self::ArrowLeft),
            "ArrowRight" => Some(Self::ArrowRight),
            "Enter" => Some(Self::Enter),
            "Tab" => Some(Self::Tab),
            "Space" => Some(Self::Space),
            "Backspace" => Some(Self::Backspace),
            "Delete" => Some(Self::Delete),
            "Home" => Some(Self::Home),
            "End" => Some(Self::End),
            "PageUp" => Some(Self::PageUp),
            "PageDown" => Some(Self::PageDown),
            "F1" => Some(Self::F1),
            "F2" => Some(Self::F2),
            "F3" => Some(Self::F3),
            "F4" => Some(Self::F4),
            "F5" => Some(Self::F5),
            "F6" => Some(Self::F6),
            "F7" => Some(Self::F7),
            "F8" => Some(Self::F8),
            "F9" => Some(Self::F9),
            "F10" => Some(Self::F10),
            "F11" => Some(Self::F11),
            "F12" => Some(Self::F12),
            _ => None,
        }
    }
}

// ── Key ─────────────────────────────────────────────────────────────────

/// A physical key: either a character or a named key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// Single character, always stored lowercase.
    Char(char),
    Named(NamedKey),
}

// ── Modifiers ───────────────────────────────────────────────────────────

/// Modifier keys. `cmd_or_ctrl` resolves to Cmd on macOS, Ctrl elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub cmd_or_ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        cmd_or_ctrl: false,
        shift: false,
        alt: false,
    };

    pub const fn has_any(self) -> bool {
        self.cmd_or_ctrl || self.shift || self.alt
    }
}

// ── Chord ───────────────────────────────────────────────────────────────

/// A single key press with optional modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Chord {
    pub const fn key(c: char) -> Self {
        Self {
            key: Key::Char(c),
            modifiers: Modifiers::NONE,
        }
    }

    pub const fn named(n: NamedKey) -> Self {
        Self {
            key: Key::Named(n),
            modifiers: Modifiers::NONE,
        }
    }

    /// Parse a chord from a string like `"j"`, `"Escape"`, or `"CmdOrCtrl+Shift+A"`.
    ///
    /// Accepts `"Ctrl"`, `"Cmd"`, and `"CmdOrCtrl"` as the primary modifier.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('+').collect();
        let key_str = parts.last().ok_or_else(|| "empty chord".to_string())?;

        let mut modifiers = Modifiers::NONE;
        for &m in &parts[..parts.len() - 1] {
            match m {
                "CmdOrCtrl" | "Ctrl" | "Cmd" => modifiers.cmd_or_ctrl = true,
                "Alt" => modifiers.alt = true,
                "Shift" => modifiers.shift = true,
                _ => return Err(format!("unknown modifier: {m}")),
            }
        }

        let key = if let Some(named) = NamedKey::parse(key_str) {
            Key::Named(named)
        } else {
            let chars: Vec<char> = key_str.chars().collect();
            if chars.len() == 1 {
                Key::Char(chars[0].to_ascii_lowercase())
            } else {
                return Err(format!("unknown key: {key_str}"));
            }
        };

        Ok(Self { key, modifiers })
    }

    /// Platform-resolved display string.
    pub fn display(&self, platform: Platform) -> String {
        let modifier_label = match platform {
            Platform::Mac => "Cmd",
            Platform::Windows | Platform::Linux => "Ctrl",
        };
        self.format_with_modifier_label(modifier_label)
    }

    /// Platform-agnostic canonical string (used for serialization).
    fn canonical(&self) -> String {
        self.format_with_modifier_label("CmdOrCtrl")
    }

    fn format_with_modifier_label(&self, cmd_or_ctrl_label: &str) -> String {
        let mut s = String::new();
        if self.modifiers.cmd_or_ctrl {
            s.push_str(cmd_or_ctrl_label);
            s.push('+');
        }
        if self.modifiers.alt {
            s.push_str("Alt+");
        }
        if self.modifiers.shift {
            s.push_str("Shift+");
        }
        match self.key {
            Key::Char(c) => {
                if self.modifiers.has_any() {
                    s.push(c.to_ascii_uppercase());
                } else {
                    s.push(c);
                }
            }
            Key::Named(n) => s.push_str(n.as_str()),
        }
        s
    }
}

// ── KeyBinding ──────────────────────────────────────────────────────────

/// A keybinding: either a single chord or a two-key sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyBinding {
    Chord(Chord),
    /// Two-chord sequence (e.g., "g then i"). The UI layer handles the
    /// pending state and timeout between chords.
    Sequence(Chord, Chord),
}

impl KeyBinding {
    pub const fn key(c: char) -> Self {
        Self::Chord(Chord::key(c))
    }

    pub const fn named(n: NamedKey) -> Self {
        Self::Chord(Chord::named(n))
    }

    pub const fn cmd_or_ctrl(c: char) -> Self {
        Self::Chord(Chord {
            key: Key::Char(c),
            modifiers: Modifiers {
                cmd_or_ctrl: true,
                shift: false,
                alt: false,
            },
        })
    }

    pub const fn cmd_or_ctrl_shift(c: char) -> Self {
        Self::Chord(Chord {
            key: Key::Char(c),
            modifiers: Modifiers {
                cmd_or_ctrl: true,
                shift: true,
                alt: false,
            },
        })
    }

    /// Two-character sequence with no modifiers (e.g., `seq('g', 'i')` for "g then i").
    pub const fn seq(first: char, second: char) -> Self {
        Self::Sequence(Chord::key(first), Chord::key(second))
    }

    /// Parse from canonical string format.
    ///
    /// Accepts `"j"`, `"Escape"`, `"CmdOrCtrl+A"`, `"g then i"`, etc.
    pub fn parse(s: &str) -> Result<Self, String> {
        if let Some((first, second)) = s.split_once(" then ") {
            let a = Chord::parse(first)?;
            let b = Chord::parse(second)?;
            Ok(Self::Sequence(a, b))
        } else {
            Ok(Self::Chord(Chord::parse(s)?))
        }
    }

    /// Platform-resolved display string.
    pub fn display(&self, platform: Platform) -> String {
        match self {
            Self::Chord(c) => c.display(platform),
            Self::Sequence(a, b) => format!("{} then {}", a.display(platform), b.display(platform)),
        }
    }

    /// Platform-agnostic canonical string (serde serialization format).
    pub fn canonical(&self) -> String {
        match self {
            Self::Chord(c) => c.canonical(),
            Self::Sequence(a, b) => format!("{} then {}", a.canonical(), b.canonical()),
        }
    }
}

impl fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl Serialize for KeyBinding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> Deserialize<'de> for KeyBinding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

// ── Platform ────────────────────────────────────────────────────────────

/// Target platform for resolving `CmdOrCtrl` in display strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

/// Returns the platform detected at compile time via `cfg!`.
pub fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

// ── ResolveResult ───────────────────────────────────────────────────────

/// Result of resolving a single chord against the binding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveResult {
    /// No binding matches this chord.
    NoMatch,
    /// Single-chord binding matched - execute this command.
    Command(CommandId),
    /// This chord starts one or more sequences - caller should wait for the
    /// second chord (with a timeout) and then call `resolve_sequence`.
    Pending,
}

// ── BindingTable ────────────────────────────────────────────────────────

/// Keybinding resolution table with override support and conflict detection.
///
/// Built from `CommandRegistry` defaults, then optionally layered with user
/// overrides. Maintains O(1) reverse indexes for chord → command lookup.
pub struct BindingTable {
    defaults: HashMap<CommandId, KeyBinding>,
    /// `None` = explicitly unbound (don't fall back to default).
    /// `Some(binding)` = custom binding.
    /// Absent = use default.
    overrides: HashMap<CommandId, Option<KeyBinding>>,
    single_reverse: HashMap<Chord, CommandId>,
    sequence_first_chords: HashMap<Chord, HashMap<Chord, CommandId>>,
    platform: Platform,
}

impl BindingTable {
    /// Build from registry defaults. Validates uniqueness in debug builds.
    pub fn new(registry: &CommandRegistry, platform: Platform) -> Self {
        let defaults: HashMap<CommandId, KeyBinding> = registry.default_bindings().collect();

        #[cfg(debug_assertions)]
        {
            let mut seen: HashMap<KeyBinding, CommandId> = HashMap::new();
            for (&id, &binding) in &defaults {
                if let Some(&prev) = seen.get(&binding) {
                    panic!("duplicate default binding {binding}: {prev:?} and {id:?}");
                }
                seen.insert(binding, id);
            }
        }

        let mut table = Self {
            defaults,
            overrides: HashMap::new(),
            single_reverse: HashMap::new(),
            sequence_first_chords: HashMap::new(),
            platform,
        };
        table.rebuild_reverse();
        table
    }

    /// Load user overrides (typically deserialized from settings DB).
    /// Replaces all existing overrides and rebuilds the reverse index.
    pub fn load_overrides(&mut self, overrides: HashMap<CommandId, Option<KeyBinding>>) {
        self.overrides = overrides;
        self.rebuild_reverse();
    }

    /// Resolve a single chord press.
    ///
    /// Returns `Pending` if this chord starts one or more sequences - the
    /// caller should wait for the second chord (with a timeout) then call
    /// `resolve_sequence`. Returns `Command` for single-chord bindings.
    pub fn resolve_chord(&self, chord: &Chord) -> ResolveResult {
        if self.sequence_first_chords.contains_key(chord) {
            return ResolveResult::Pending;
        }
        match self.single_reverse.get(chord) {
            Some(&id) => ResolveResult::Command(id),
            None => ResolveResult::NoMatch,
        }
    }

    /// Resolve the second chord of a pending sequence.
    pub fn resolve_sequence(&self, first: &Chord, second: &Chord) -> Option<CommandId> {
        self.sequence_first_chords
            .get(first)
            .and_then(|seconds| seconds.get(second))
            .copied()
    }

    /// Effective binding for a command (override > default). `None` if unbound.
    pub fn binding_for(&self, id: CommandId) -> Option<KeyBinding> {
        match self.overrides.get(&id) {
            Some(Some(binding)) => Some(*binding),
            Some(None) => None,
            None => self.defaults.get(&id).copied(),
        }
    }

    /// Platform-resolved display string for a command's effective binding.
    pub fn display_binding(&self, id: CommandId) -> Option<String> {
        self.binding_for(id).map(|kb| kb.display(self.platform))
    }

    /// Check if a proposed binding for `id` would conflict with an existing
    /// command's effective binding. Returns the first conflicting `CommandId`.
    ///
    /// Conflict rules:
    /// - Two single-chord bindings can't share the same chord
    /// - Two sequences can't share both chords
    /// - A single chord can't use a chord that starts a sequence (ambiguous)
    /// - A sequence's first chord can't be an existing single-chord binding
    pub fn check_conflict(&self, id: CommandId, binding: &KeyBinding) -> Option<CommandId> {
        match binding {
            KeyBinding::Chord(chord) => self.check_chord_conflict(id, chord),
            KeyBinding::Sequence(first, second) => self.check_sequence_conflict(id, first, second),
        }
    }

    /// Set a user override. Returns `Err(conflicting_id)` if the binding
    /// conflicts with another command. Call `check_conflict` first to
    /// inspect conflicts, or `unbind` the conflicting command before retrying.
    pub fn set_override(&mut self, id: CommandId, binding: KeyBinding) -> Result<(), CommandId> {
        if let Some(conflict) = self.check_conflict(id, &binding) {
            return Err(conflict);
        }
        self.overrides.insert(id, Some(binding));
        self.rebuild_reverse();
        Ok(())
    }

    /// Explicitly unbind a command (no fallback to default).
    pub fn unbind(&mut self, id: CommandId) {
        self.overrides.insert(id, None);
        self.rebuild_reverse();
    }

    /// Remove the override for a command, reverting to its default binding.
    pub fn remove_override(&mut self, id: CommandId) {
        self.overrides.remove(&id);
        self.rebuild_reverse();
    }

    /// Reset all overrides, reverting every command to its default binding.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
        self.rebuild_reverse();
    }

    /// All current overrides (for persistence in slice 6).
    pub fn overrides(&self) -> &HashMap<CommandId, Option<KeyBinding>> {
        &self.overrides
    }

    /// Save current overrides to a JSON file.
    ///
    /// Format: `{ "overrides": { "command.id": "binding" | null } }`
    /// where `null` means explicitly unbound. Only non-empty override
    /// maps are written; an empty map deletes the file.
    pub fn save_overrides(&self, path: &Path) -> Result<(), String> {
        if self.overrides.is_empty() {
            // Nothing to persist - remove stale file if present.
            if path.exists() {
                std::fs::remove_file(path)
                    .map_err(|e| format!("remove {}: {e}", path.display()))?;
            }
            return Ok(());
        }

        // Use BTreeMap for deterministic key order in the output.
        let map: BTreeMap<String, Option<String>> = self
            .overrides
            .iter()
            .map(|(id, binding)| (id.as_str().to_string(), binding.map(|kb| kb.canonical())))
            .collect();

        let wrapper = OverridesFile { overrides: map };
        let json = serde_json::to_string_pretty(&wrapper)
            .map_err(|e| format!("serialize overrides: {e}"))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }

        std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))?;

        Ok(())
    }

    /// Load overrides from a JSON file and apply them.
    ///
    /// If the file does not exist, this is a no-op (returns `Ok`).
    /// Invalid entries (unknown command IDs, unparseable bindings) are
    /// silently skipped so a hand-edited file with typos doesn't block
    /// startup.
    pub fn load_overrides_from_file(&mut self, path: &Path) -> Result<(), String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };

        let wrapper: OverridesFile =
            serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;

        let mut overrides = HashMap::new();
        for (key, value) in wrapper.overrides {
            let Some(id) = CommandId::parse(&key) else {
                continue; // unknown command - skip
            };
            match value {
                None => {
                    overrides.insert(id, None);
                }
                Some(ref binding_str) => {
                    if let Ok(kb) = KeyBinding::parse(binding_str) {
                        overrides.insert(id, Some(kb));
                    }
                    // unparseable binding - skip
                }
            }
        }

        self.overrides = overrides;
        self.rebuild_reverse();
        Ok(())
    }

    fn check_chord_conflict(&self, id: CommandId, chord: &Chord) -> Option<CommandId> {
        if let Some(&existing) = self.single_reverse.get(chord)
            && existing != id
        {
            return Some(existing);
        }
        if let Some(sequences) = self.sequence_first_chords.get(chord) {
            for &cmd in sequences.values() {
                if cmd != id {
                    return Some(cmd);
                }
            }
        }
        None
    }

    fn check_sequence_conflict(
        &self,
        id: CommandId,
        first: &Chord,
        second: &Chord,
    ) -> Option<CommandId> {
        if let Some(&existing) = self.single_reverse.get(first)
            && existing != id
        {
            return Some(existing);
        }
        if let Some(sequences) = self.sequence_first_chords.get(first)
            && let Some(&existing) = sequences.get(second)
            && existing != id
        {
            return Some(existing);
        }
        None
    }

    fn rebuild_reverse(&mut self) {
        self.single_reverse.clear();
        self.sequence_first_chords.clear();

        let bindings: Vec<(CommandId, KeyBinding)> = {
            let mut ids: HashSet<CommandId> = self.defaults.keys().copied().collect();
            for &id in self.overrides.keys() {
                ids.insert(id);
            }
            ids.into_iter()
                .filter_map(|id| self.binding_for(id).map(|b| (id, b)))
                .collect()
        };

        for (id, binding) in bindings {
            match binding {
                KeyBinding::Chord(chord) => {
                    self.single_reverse.insert(chord, id);
                }
                KeyBinding::Sequence(first, second) => {
                    self.sequence_first_chords
                        .entry(first)
                        .or_default()
                        .insert(second, id);
                }
            }
        }
    }
}

/// On-disk JSON representation for keybinding overrides.
#[derive(Serialize, Deserialize)]
struct OverridesFile {
    overrides: BTreeMap<String, Option<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parse / display ─────────────────────────────────────────────────

    #[test]
    fn parse_single_char() {
        assert_eq!(KeyBinding::parse("j").expect("parse"), KeyBinding::key('j'));
    }

    #[test]
    fn parse_named_key() {
        assert_eq!(
            KeyBinding::parse("Escape").expect("parse"),
            KeyBinding::named(NamedKey::Escape),
        );
    }

    #[test]
    fn parse_cmd_or_ctrl() {
        assert_eq!(
            KeyBinding::parse("CmdOrCtrl+A").expect("parse"),
            KeyBinding::cmd_or_ctrl('a'),
        );
    }

    #[test]
    fn parse_ctrl_alias() {
        assert_eq!(
            KeyBinding::parse("Ctrl+A").expect("parse"),
            KeyBinding::cmd_or_ctrl('a'),
        );
    }

    #[test]
    fn parse_cmd_or_ctrl_shift() {
        assert_eq!(
            KeyBinding::parse("CmdOrCtrl+Shift+E").expect("parse"),
            KeyBinding::cmd_or_ctrl_shift('e'),
        );
    }

    #[test]
    fn parse_sequence() {
        assert_eq!(
            KeyBinding::parse("g then i").expect("parse"),
            KeyBinding::seq('g', 'i'),
        );
    }

    #[test]
    fn display_resolves_platform() {
        let kb = KeyBinding::cmd_or_ctrl('a');
        assert_eq!(kb.display(Platform::Linux), "Ctrl+A");
        assert_eq!(kb.display(Platform::Windows), "Ctrl+A");
        assert_eq!(kb.display(Platform::Mac), "Cmd+A");
    }

    #[test]
    fn display_sequence() {
        let kb = KeyBinding::seq('g', 'i');
        assert_eq!(kb.display(Platform::Linux), "g then i");
    }

    #[test]
    fn display_bare_char_is_lowercase() {
        assert_eq!(KeyBinding::key('j').display(Platform::Linux), "j");
    }

    #[test]
    fn display_modified_char_is_uppercase() {
        assert_eq!(
            KeyBinding::cmd_or_ctrl_shift('e').display(Platform::Linux),
            "Ctrl+Shift+E",
        );
    }

    #[test]
    fn roundtrip_parse_canonical() {
        let cases = [
            KeyBinding::key('j'),
            KeyBinding::key('#'),
            KeyBinding::named(NamedKey::Escape),
            KeyBinding::named(NamedKey::F5),
            KeyBinding::cmd_or_ctrl('a'),
            KeyBinding::cmd_or_ctrl_shift('e'),
            KeyBinding::seq('g', 'i'),
        ];
        for kb in cases {
            let canonical = kb.canonical();
            let parsed =
                KeyBinding::parse(&canonical).unwrap_or_else(|e| panic!("{canonical}: {e}"));
            assert_eq!(parsed, kb, "roundtrip failed for {canonical}");
        }
    }

    #[test]
    fn parse_rejects_unknown_key() {
        assert!(KeyBinding::parse("FooBar").is_err());
    }

    #[test]
    fn parse_normalizes_case() {
        assert_eq!(KeyBinding::parse("A").expect("parse"), KeyBinding::key('a'));
    }

    // ── Serde ───────────────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip() {
        let cases = [
            KeyBinding::key('j'),
            KeyBinding::named(NamedKey::Escape),
            KeyBinding::cmd_or_ctrl('a'),
            KeyBinding::seq('g', 'i'),
        ];
        for kb in cases {
            let json = serde_json::to_string(&kb).expect("serialize");
            let parsed: KeyBinding = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, kb, "serde roundtrip failed for {kb}");
        }
    }

    // ── BindingTable resolution ─────────────────────────────────────────

    fn make_table() -> BindingTable {
        let registry = CommandRegistry::new();
        BindingTable::new(&registry, Platform::Linux)
    }

    #[test]
    fn resolve_single_chord() {
        let table = make_table();
        let chord = Chord::key('e');
        assert_eq!(
            table.resolve_chord(&chord),
            ResolveResult::Command(CommandId::EmailArchive),
        );
    }

    #[test]
    fn resolve_sequence_pending() {
        let table = make_table();
        let chord = Chord::key('g');
        assert_eq!(table.resolve_chord(&chord), ResolveResult::Pending);
    }

    #[test]
    fn resolve_sequence_complete() {
        let table = make_table();
        let first = Chord::key('g');
        let second = Chord::key('i');
        assert_eq!(
            table.resolve_sequence(&first, &second),
            Some(CommandId::NavGoInbox),
        );
    }

    #[test]
    fn resolve_sequence_no_match() {
        let table = make_table();
        let first = Chord::key('g');
        let second = Chord::key('z');
        assert_eq!(table.resolve_sequence(&first, &second), None);
    }

    #[test]
    fn resolve_no_match() {
        let table = make_table();
        let chord = Chord::key('z');
        assert_eq!(table.resolve_chord(&chord), ResolveResult::NoMatch);
    }

    // ── BindingTable overrides ──────────────────────────────────────────

    #[test]
    fn override_replaces_default() {
        let mut table = make_table();
        let new_binding = KeyBinding::key('x');
        table
            .set_override(CommandId::EmailArchive, new_binding)
            .expect("no conflict");

        assert_eq!(
            table.binding_for(CommandId::EmailArchive),
            Some(new_binding)
        );

        let old_chord = Chord::key('e');
        assert_eq!(table.resolve_chord(&old_chord), ResolveResult::NoMatch);

        let new_chord = Chord::key('x');
        assert_eq!(
            table.resolve_chord(&new_chord),
            ResolveResult::Command(CommandId::EmailArchive),
        );
    }

    #[test]
    fn explicit_unbind() {
        let mut table = make_table();
        table.unbind(CommandId::EmailArchive);

        assert_eq!(table.binding_for(CommandId::EmailArchive), None);

        let chord = Chord::key('e');
        assert_eq!(table.resolve_chord(&chord), ResolveResult::NoMatch);
    }

    #[test]
    fn remove_override_restores_default() {
        let mut table = make_table();
        table.unbind(CommandId::EmailArchive);
        assert_eq!(table.binding_for(CommandId::EmailArchive), None);

        table.remove_override(CommandId::EmailArchive);
        assert_eq!(
            table.binding_for(CommandId::EmailArchive),
            Some(KeyBinding::key('e')),
        );

        let chord = Chord::key('e');
        assert_eq!(
            table.resolve_chord(&chord),
            ResolveResult::Command(CommandId::EmailArchive),
        );
    }

    #[test]
    fn reset_all_clears_overrides() {
        let mut table = make_table();
        table.unbind(CommandId::EmailArchive);
        table
            .set_override(CommandId::EmailStar, KeyBinding::key('x'))
            .expect("no conflict");
        table.reset_all();

        assert_eq!(
            table.binding_for(CommandId::EmailArchive),
            Some(KeyBinding::key('e')),
        );
        assert_eq!(
            table.binding_for(CommandId::EmailStar),
            Some(KeyBinding::key('s')),
        );
        assert!(table.overrides().is_empty());
    }

    // ── Conflict detection ──────────────────────────────────────────────

    #[test]
    fn conflict_single_vs_single() {
        let table = make_table();
        let conflict = table.check_conflict(CommandId::NavNext, &KeyBinding::key('e'));
        assert_eq!(conflict, Some(CommandId::EmailArchive));
    }

    #[test]
    fn conflict_single_vs_sequence_first() {
        let table = make_table();
        // 'g' starts sequences - binding a single chord to 'g' conflicts
        let conflict = table.check_conflict(CommandId::NavNext, &KeyBinding::key('g'));
        assert!(
            conflict.is_some(),
            "should conflict with a 'g then X' sequence"
        );
    }

    #[test]
    fn conflict_sequence_vs_single() {
        let table = make_table();
        // 'e' is EmailArchive - a sequence starting with 'e' conflicts
        let conflict = table.check_conflict(CommandId::NavNext, &KeyBinding::seq('e', 'x'));
        assert_eq!(conflict, Some(CommandId::EmailArchive));
    }

    #[test]
    fn no_conflict_same_command() {
        let table = make_table();
        // Rebinding a command to its own binding is not a conflict
        let conflict = table.check_conflict(CommandId::EmailArchive, &KeyBinding::key('e'));
        assert_eq!(conflict, None);
    }

    #[test]
    fn no_conflict_different_sequence_second_chord() {
        let table = make_table();
        // 'g then z' doesn't conflict with existing 'g then i', 'g then s', etc.
        let conflict = table.check_conflict(CommandId::EmailArchive, &KeyBinding::seq('g', 'z'));
        assert_eq!(conflict, None);
    }

    #[test]
    fn set_override_rejects_conflict() {
        let mut table = make_table();
        let result = table.set_override(CommandId::NavNext, KeyBinding::key('e'));
        assert_eq!(result, Err(CommandId::EmailArchive));
    }

    // ── Display binding ─────────────────────────────────────────────────

    #[test]
    fn display_binding_default() {
        let table = make_table();
        assert_eq!(
            table.display_binding(CommandId::EmailArchive),
            Some("e".to_string()),
        );
    }

    #[test]
    fn display_binding_sequence() {
        let table = make_table();
        assert_eq!(
            table.display_binding(CommandId::NavGoInbox),
            Some("g then i".to_string()),
        );
    }

    #[test]
    fn display_binding_none_for_unbound() {
        let table = make_table();
        // EmailAddLabel has no default binding
        assert_eq!(table.display_binding(CommandId::EmailAddLabel), None);
    }
}
