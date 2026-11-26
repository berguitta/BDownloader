//! Module registry.
//!
//! Optional features of BDownloader (a link source, a protocol, a
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
    PostProcessing,
    Interface,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Core => "Core",
            Category::LinkSource => "Link sources",
            Category::Protocol => "Protocols",
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
        description: "Downloads plain web links.",
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
        name: "Speed limit",
        description: "Global bandwidth limit, also from the tray menu.",
        category: Category::Core,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "clipboard",
        name: "Clipboard watcher",
        description:
            "Links you copy appear in the link grabber. Reacts to clipboard changes, no polling of the content.",
        category: Category::LinkSource,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "click-n-load",
        name: "Click'n'Load",
        description: "Lets download sites send links to BDownloader with their Click'n'Load button. Listens on this computer only.",
        category: Category::LinkSource,
        status: ModuleStatus::Available,
        default_enabled: false,
        requires: &["http"],
    },
    ModuleInfo {
        id: "page-scan",
        name: "Web page scan",
        description: "A link to a web page becomes a pool of the file and hoster links it contains.",
        category: Category::LinkSource,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &["http"],
    },
    ModuleInfo {
        id: "video",
        name: "Video sites (yt-dlp)",
        description: "YouTube and similar sites: pick the quality or audio only. yt-dlp is fetched on demand.",
        category: Category::Protocol,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "torrent",
        name: "BitTorrent and magnet links",
        description: "Downloads torrents next to your DDL queue, then seeds up to the ratio you chose.",
        category: Category::Protocol,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "decrypters",
        name: "Folder and list decrypters",
        description: "Expands 1fichier folders, MediaFire folders and Pixeldrain lists into their files.",
        category: Category::LinkSource,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &["http"],
    },
    ModuleInfo {
        id: "scheduler",
        name: "Scheduler",
        description: "Starts, pauses, limits the speed, reconnects or turns the computer off on a schedule.",
        category: Category::Core,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "reconnect",
        name: "Reconnect",
        description: "Gets a new IP from your internet box (UPnP or a command) to skip free hoster wait timers.",
        category: Category::Core,
        status: ModuleStatus::Available,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "power",
        name: "Power actions",
        description: "Closes BDownloader, puts the computer to sleep or turns it off when the queue is finished.",
        category: Category::Interface,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "anti-standby",
        name: "Anti-standby",
        description: "Keeps the computer awake while downloads or extractions are running.",
        category: Category::Interface,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "event-scripts",
        name: "Event scripts",
        description: "Sends Discord or Telegram messages and runs your own command when downloads finish or fail.",
        category: Category::PostProcessing,
        status: ModuleStatus::Available,
        default_enabled: false,
        requires: &[],
    },
    ModuleInfo {
        id: "checksum",
        name: "Checksum verification",
        description: "Checks finished packages against their .sfv, .md5, .sha1 or .sha256 files.",
        category: Category::PostProcessing,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "extraction",
        name: "Automatic extraction",
        description: "Unpacks finished RAR, ZIP and 7z packages, tries the package password and your list.",
        category: Category::PostProcessing,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "notifications",
        name: "Windows notifications",
        description: "Notifies when a package finishes, fails or is extracted.",
        category: Category::Interface,
        status: ModuleStatus::Available,
        default_enabled: true,
        requires: &[],
    },
    ModuleInfo {
        id: "tray",
        name: "Tray icon",
        description: "Icon in the notification area with a quick menu. Close or minimize to it.",
        category: Category::Interface,
        status: ModuleStatus::Available,
        default_enabled: true,
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
        assert!(reg.is_enabled("torrent"));
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
        assert!(changed.contains(&"http") && changed.contains(&"resume"));
        assert!(reg.is_enabled("http") && reg.is_enabled("multi-connection"));
    }

    #[test]
    fn planned_modules_cannot_be_enabled() {
        let mut reg = ModuleRegistry::builtin();
        assert!(reg.set_enabled("torrent", true).is_ok());
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
