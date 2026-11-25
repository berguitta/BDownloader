//! Module registry.
//!
//! Every feature of BDownloader (a link source, a hoster, a debrid service, a
//! post-processing step…) is described by a [`ModuleInfo`] and can be turned
//! on or off by the user. Dependencies are enforced: enabling a module enables
//! what it requires, disabling a module disables what depends on it.

use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Core,
    LinkSource,
    Protocol,
    Hoster,
    Debrid,
    PostProcessing,
    Interface,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Core => "Core",
            Category::LinkSource => "Link sources",
            Category::Protocol => "Protocols",
            Category::Hoster => "Hosters",
            Category::Debrid => "Debrid services",
            Category::PostProcessing => "Post-processing",
            Category::Interface => "Interface",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleStatus {
    /// Implemented and usable.
    Available,
    /// On the roadmap, shown so users see what is coming.
    Planned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: Category,
    pub status: ModuleStatus,
    pub default_enabled: bool,
    /// Ids of modules that must be enabled for this one to work.
    pub requires: &'static [&'static str],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModuleError {
    #[error("unknown module `{0}`")]
    Unknown(String),
    #[error("module `{0}` is not available yet")]
    NotAvailable(&'static str),
}

/// Modules shipped with BDownloader.
pub const BUILTIN_MODULES: &[ModuleInfo] = &[
    ModuleInfo {
        id: "http",
        name: "Direct HTTP/HTTPS links",
        description: "Downloads plain web links. Required by every hoster.",
        category: Category::Protocol,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "multi-connection",
        name: "Multi-connection downloads",
        description: "Splits files into segments downloaded in parallel when the server allows it.",
        category: Category::Core,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &["http"],
    },
    ModuleInfo {
        id: "resume",
        name: "Resume interrupted downloads",
        description: "Saves progress next to the partial file and continues at the exact byte.",
        category: Category::Core,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &["http"],
    },
    ModuleInfo {
        id: "speed-limit",
        name: "Speed limit and scheduler",
        description: "Global and per-download bandwidth limits, time windows.",
        category: Category::Core,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "clipboard",
        name: "Clipboard watcher",
        description: "Adds links as soon as you copy them, without polling.",
        category: Category::LinkSource,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "click-n-load",
        name: "Click'n'Load",
        description: "Local listener on 127.0.0.1:9666 for link sites. No web interface.",
        category: Category::LinkSource,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "hoster-1fichier",
        name: "1fichier",
        description: "Free mode with wait timers, premium through the official API.",
        category: Category::Hoster,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &["http"],
    },
    ModuleInfo {
        id: "hoster-rapidgator",
        name: "Rapidgator",
        description: "Free mode with wait timers, premium through the official API.",
        category: Category::Hoster,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &["http"],
    },
    ModuleInfo {
        id: "debrid-real-debrid",
        name: "Real-Debrid",
        description: "Unlocks many hosters with one Real-Debrid account.",
        category: Category::Debrid,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &["http"],
    },
    ModuleInfo {
        id: "debrid-alldebrid",
        name: "AllDebrid",
        description: "Unlocks many hosters with one AllDebrid account.",
        category: Category::Debrid,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &["http"],
    },
    ModuleInfo {
        id: "torrent",
        name: "BitTorrent and magnet links",
        description: "Downloads torrents next to your DDL queue.",
        category: Category::Protocol,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "video",
        name: "Video sites (yt-dlp)",
        description: "Downloads videos from YouTube and similar sites. yt-dlp is fetched on demand.",
        category: Category::Protocol,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "extraction",
        name: "Automatic extraction",
        description: "Unpacks RAR, ZIP and 7z archives, tries your password list.",
        category: Category::PostProcessing,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "notifications",
        name: "Windows notifications",
        description: "Notifies when a package finishes or needs your attention.",
        category: Category::Interface,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "tray",
        name: "Tray icon",
        description: "Keeps BDownloader running in the notification area.",
        category: Category::Interface,
        status: ModuleStatus::Planned,
        default_enabled: false,
        requires: &[],
    },
];

/// Which modules exist and which ones the user turned on.
#[derive(Clone, Debug)]
pub struct ModuleRegistry {
    modules: Vec<ModuleInfo>,
    overrides: BTreeMap<String, bool>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ModuleRegistry {
    pub fn new(modules: Vec<ModuleInfo>) -> Self {
        ModuleRegistry { modules, overrides: BTreeMap::new() }
    }

    pub fn builtin() -> Self {
        Self::new(BUILTIN_MODULES.to_vec())
    }

    /// Applies the user's saved choices. Unknown ids are kept so a module
    /// coming back in a later version remembers its state.
    pub fn with_states(mut self, states: &BTreeMap<String, bool>) -> Self {
        self.overrides = states.clone();
        self
    }

    pub fn get(&self, id: &str) -> Option<&ModuleInfo> {
        self.modules.iter().find(|m| m.id == id)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        let Some(info) = self.get(id) else { return false };
        if info.status != ModuleStatus::Available {
            return false;
        }
        let own = self.overrides.get(id).copied().unwrap_or(info.default_enabled);
        own && info.requires.iter().all(|dep| self.is_enabled(dep))
    }

    /// All modules with their effective state, grouped by category.
    pub fn list(&self) -> Vec<(ModuleInfo, bool)> {
        let mut list: Vec<_> = self.modules.iter().map(|m| (*m, self.is_enabled(m.id))).collect();
        list.sort_by_key(|(m, _)| m.category);
        list
    }

    /// Turns a module on or off and returns the ids of the other modules
    /// whose state changed because of dependencies.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<Vec<&'static str>, ModuleError> {
        let info = *self.get(id).ok_or_else(|| ModuleError::Unknown(id.to_owned()))?;
        if enabled && info.status != ModuleStatus::Available {
            return Err(ModuleError::NotAvailable(info.id));
        }
        let before: Vec<bool> = self.modules.iter().map(|m| self.is_enabled(m.id)).collect();
        if enabled {
            self.enable_with_requirements(info.id);
        } else {
            self.overrides.insert(info.id.to_owned(), false);
        }
        let changed = self
            .modules
            .iter()
            .zip(before)
            .filter(|(m, was)| m.id != info.id && self.is_enabled(m.id) != *was)
            .map(|(m, _)| m.id)
            .collect();
        Ok(changed)
    }

    fn enable_with_requirements(&mut self, id: &'static str) {
        self.overrides.insert(id.to_owned(), true);
        let requires = self.get(id).map(|m| m.requires).unwrap_or_default();
        for dep in requires {
            if !self.is_enabled(dep) {
                self.enable_with_requirements(dep);
            }
        }
    }

    /// Choices to persist in the settings file.
    pub fn states(&self) -> BTreeMap<String, bool> {
        self.overrides.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_module_definitions() {
        let reg = ModuleRegistry::builtin();
        assert!(reg.is_enabled("http"));
        assert!(reg.is_enabled("multi-connection"));
        assert!(!reg.is_enabled("clipboard"), "planned modules are off");
        assert!(!reg.is_enabled("nope"));
    }

    #[test]
    fn disabling_a_requirement_disables_dependents() {
        let mut reg = ModuleRegistry::builtin();
        let changed = reg.set_enabled("http", false).unwrap();
        assert!(changed.contains(&"multi-connection"));
        assert!(changed.contains(&"resume"));
        assert!(!reg.is_enabled("multi-connection"));
    }

    #[test]
    fn enabling_a_dependent_enables_its_requirements() {
        let mut reg = ModuleRegistry::builtin();
        reg.set_enabled("http", false).unwrap();
        reg.set_enabled("multi-connection", false).unwrap();
        let changed = reg.set_enabled("multi-connection", true).unwrap();
        assert_eq!(changed, vec!["http", "resume"]);
        assert!(reg.is_enabled("http") && reg.is_enabled("multi-connection"));
    }

    #[test]
    fn planned_modules_cannot_be_enabled() {
        let mut reg = ModuleRegistry::builtin();
        assert_eq!(reg.set_enabled("torrent", true), Err(ModuleError::NotAvailable("torrent")));
    }

    #[test]
    fn states_round_trip() {
        let mut reg = ModuleRegistry::builtin();
        reg.set_enabled("resume", false).unwrap();
        let restored = ModuleRegistry::builtin().with_states(&reg.states());
        assert!(!restored.is_enabled("resume"));
        assert!(restored.is_enabled("http"));
    }
}
