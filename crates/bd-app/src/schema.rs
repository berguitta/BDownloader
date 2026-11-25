//! Every option shown in Settings, grouped in sections like the mockup.
//! The UI is generated from this table, so search, "show advanced" and
//! "reset this page" work the same for all options.

use bd_core::{Settings, ThemeMode};

use crate::i18n;

#[derive(Clone, Copy)]
pub enum Kind {
    Toggle(bool),
    Number {
        default: i64,
        min: i64,
        max: i64,
        unit: &'static str,
    },
    /// (stored value, English label, French label)
    Select {
        default: &'static str,
        options: &'static [(&'static str, &'static str, &'static str)],
    },
    Text(&'static str),
    Path,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// Takes effect after restarting BDownloader.
    Restart,
    /// Shown so users see it is coming, not editable yet.
    Planned,
}

pub struct Def {
    pub cat: usize,
    pub id: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
    pub hint_en: &'static str,
    pub hint_fr: &'static str,
    pub kind: Kind,
    pub advanced: bool,
    pub status: Status,
}

/// (English, French). The "Modules" section is filled from the registry.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("General", "Général"),
    ("Connection", "Connexion"),
    ("Downloads", "Téléchargements"),
    ("Hosters & accounts", "Hébergeurs et comptes"),
    ("Captcha", "Captcha"),
    ("Extraction", "Extraction"),
    ("Video (yt-dlp)", "Vidéo (yt-dlp)"),
    ("Torrent", "Torrent"),
    ("Clipboard & Click'n'Load", "Presse-papiers et Click'n'Load"),
    ("Interface", "Interface"),
    ("Language", "Langue"),
    ("Modules", "Modules"),
    ("Advanced", "Avancé"),
];

pub const MODULES_CATEGORY: usize = 11;

pub fn category_name(index: usize) -> &'static str {
    let (en, fr) = CATEGORIES[index.min(CATEGORIES.len() - 1)];
    if i18n::is_french() {
        fr
    } else {
        en
    }
}

const fn d(
    cat: usize,
    id: &'static str,
    en: &'static str,
    fr: &'static str,
    hint_en: &'static str,
    hint_fr: &'static str,
    kind: Kind,
) -> Def {
    Def { cat, id, en, fr, hint_en, hint_fr, kind, advanced: false, status: Status::Ready }
}

const fn adv(mut def: Def) -> Def {
    def.advanced = true;
    def
}

const fn restart(mut def: Def) -> Def {
    def.status = Status::Restart;
    def
}

const fn planned(mut def: Def) -> Def {
    def.status = Status::Planned;
    def
}

const fn num(default: i64, min: i64, max: i64, unit: &'static str) -> Kind {
    Kind::Number { default, min, max, unit }
}

pub const DEFS: &[Def] = &[
    // General
    d(
        0,
        "start-on-launch",
        "Start downloads when BDownloader opens",
        "Lancer les téléchargements à l'ouverture",
        "Otherwise the saved queue comes back paused.",
        "Sinon, la file enregistrée revient en pause.",
        Kind::Toggle(true),
    ),
    d(
        0,
        "launch-with-windows",
        "Launch with Windows",
        "Lancer avec Windows",
        "Starts minimized in the tray.",
        "Démarre réduit dans la zone de notification.",
        Kind::Toggle(false),
    ),
    d(
        0,
        "close-to-tray",
        "Close button keeps BDownloader in the tray",
        "Le bouton fermer garde BDownloader dans la zone de notification",
        "Downloads continue. Quit from the tray menu.",
        "Les téléchargements continuent. Quittez depuis le menu de l'icône.",
        Kind::Toggle(true),
    ),
    d(
        0,
        "minimize-to-tray",
        "Minimize hides the window to the tray",
        "Réduire cache la fenêtre dans la zone de notification",
        "",
        "",
        Kind::Toggle(false),
    ),
    d(0, "download-dir", "Default download folder", "Dossier de téléchargement par défaut", "", "", Kind::Path),
    d(
        0,
        "folder-per-package",
        "Create a folder per package",
        "Créer un dossier par paquet",
        "For packages with several files.",
        "Pour les paquets de plusieurs fichiers.",
        Kind::Toggle(true),
    ),
    d(
        0,
        "when-done",
        "When the queue is finished",
        "Quand la file est terminée",
        "",
        "",
        Kind::Select {
            default: "nothing",
            options: &[
                ("nothing", "Do nothing", "Ne rien faire"),
                ("close", "Close BDownloader", "Fermer BDownloader"),
                ("sleep", "Sleep", "Mettre en veille"),
                ("shutdown", "Shut down", "Éteindre l'ordinateur"),
            ],
        },
    ),
    d(
        0,
        "auto-start-grabbed",
        "Start new links without the link grabber",
        "Lancer les nouveaux liens sans passer par la capture",
        "Click'n'Load links always go to the link grabber.",
        "Les liens Click'n'Load passent toujours par la capture.",
        Kind::Toggle(false),
    ),
    // Connection
    d(
        1,
        "parallel",
        "Parallel downloads",
        "Téléchargements en parallèle",
        "Files downloading at the same time.",
        "Fichiers téléchargés en même temps.",
        num(4, 1, 50, "files"),
    ),
    d(
        1,
        "connections",
        "Connections per file",
        "Connexions par fichier",
        "Used when the hoster or account allows it.",
        "Utilisé quand l'hébergeur ou le compte l'autorise.",
        num(8, 1, 32, "max"),
    ),
    d(
        1,
        "speed-limit",
        "Global speed limit",
        "Limite de débit globale",
        "Any value, 0 means unlimited. Also from the speed button at the bottom right.",
        "N'importe quelle valeur, 0 = illimité. Aussi depuis le bouton de vitesse en bas à droite.",
        num(0, 0, 10_000_000, "KB/s"),
    ),
    restart(d(
        1,
        "proxy",
        "Proxy",
        "Proxy",
        "",
        "",
        Kind::Select {
            default: "system",
            options: &[
                ("none", "None", "Aucun"),
                ("system", "System settings", "Réglages du système"),
                ("http", "HTTP", "HTTP"),
                ("socks5", "SOCKS5", "SOCKS5"),
            ],
        },
    )),
    restart(d(
        1,
        "proxy-address",
        "Proxy address",
        "Adresse du proxy",
        "host:port, for HTTP or SOCKS5.",
        "hôte:port, pour HTTP ou SOCKS5.",
        Kind::Text(""),
    )),
    restart(adv(d(
        1,
        "per-host",
        "Connections per host",
        "Connexions par hébergeur",
        "Starting limit, lowered automatically when a server refuses.",
        "Limite de départ, baissée automatiquement si un serveur refuse.",
        num(16, 1, 64, "max"),
    ))),
    restart(adv(d(1, "timeout", "Connection timeout", "Délai de connexion", "", "", num(30, 5, 300, "s")))),
    restart(adv(d(
        1,
        "retries",
        "Retries before an error",
        "Nouvelles tentatives avant une erreur",
        "",
        "",
        num(5, 0, 50, "times"),
    ))),
    planned(adv(d(
        1,
        "schedule",
        "Speed limit schedule",
        "Planning de limite de débit",
        "For example 1 MB/s from 9:00 to 18:00.",
        "Par exemple 1 Mo/s de 9 h à 18 h.",
        Kind::Toggle(false),
    ))),
    planned(adv(d(
        1,
        "reconnect",
        "Get a new IP when a hoster makes you wait",
        "Changer d'IP quand un hébergeur fait attendre",
        "",
        "",
        Kind::Select {
            default: "off",
            options: &[
                ("off", "Off", "Non"),
                ("upnp", "Router via UPnP", "Routeur via UPnP"),
                ("script", "Custom script", "Script personnalisé"),
            ],
        },
    ))),
    // Downloads
    d(
        2,
        "if-exists",
        "If the file already exists",
        "Si le fichier existe déjà",
        "",
        "",
        Kind::Select {
            default: "rename",
            options: &[
                ("rename", "Rename the new file", "Renommer le nouveau fichier"),
                ("skip", "Skip", "Ignorer"),
                ("overwrite", "Overwrite", "Écraser"),
            ],
        },
    ),
    adv(d(
        2,
        "preallocate",
        "Reserve disk space before downloading",
        "Réserver l'espace disque avant de télécharger",
        "Avoids fragmentation on hard drives.",
        "Évite la fragmentation sur les disques durs.",
        Kind::Toggle(true),
    )),
    planned(d(
        2,
        "verify-hash",
        "Verify checksums when the hoster gives one",
        "Vérifier les sommes de contrôle fournies par l'hébergeur",
        "",
        "",
        Kind::Toggle(true),
    )),
    planned(d(
        2,
        "min-free",
        "Pause when free disk space is under",
        "Mettre en pause sous cet espace disque libre",
        "",
        "",
        num(5, 0, 500, "GB"),
    )),
    // Hosters & accounts
    d(
        3,
        "prefer-debrid",
        "Prefer debrid services over premium accounts",
        "Préférer les services debrid aux comptes premium",
        "",
        "",
        Kind::Toggle(false),
    ),
    d(
        3,
        "free-fallback",
        "Fall back to free mode",
        "Repli sur le mode gratuit",
        "Wait timers when no account can be used. Captchas open in your browser.",
        "Temps d'attente quand aucun compte n'est utilisable. Les captchas s'ouvrent dans votre navigateur.",
        Kind::Toggle(true),
    ),
    planned(adv(d(
        3,
        "account-check",
        "Re-check accounts every",
        "Revérifier les comptes toutes les",
        "",
        "",
        num(6, 1, 72, "hours"),
    ))),
    // Captcha
    d(
        4,
        "captcha-mode",
        "When a captcha is needed",
        "Quand un captcha est demandé",
        "BDownloader never solves captchas by itself.",
        "BDownloader ne résout jamais un captcha tout seul.",
        Kind::Select {
            default: "browser",
            options: &[
                ("browser", "Open the page in my browser", "Ouvrir la page dans mon navigateur"),
                ("notify", "Notify me only", "Me prévenir seulement"),
                ("skip", "Skip the file", "Ignorer le fichier"),
            ],
        },
    ),
    d(
        4,
        "captcha-front",
        "Bring the window to the front",
        "Mettre la fenêtre au premier plan",
        "",
        "",
        Kind::Toggle(true),
    ),
    // Extraction
    d(
        5,
        "after-extract",
        "After a successful extraction",
        "Après une extraction réussie",
        "",
        "",
        Kind::Select {
            default: "delete",
            options: &[
                ("keep", "Keep archives", "Garder les archives"),
                ("delete", "Delete archives", "Supprimer les archives"),
            ],
        },
    ),
    d(
        5,
        "extract-into",
        "Extract into",
        "Extraire dans",
        "",
        "",
        Kind::Select {
            default: "package",
            options: &[
                ("package", "Package folder", "Dossier du paquet"),
                ("subfolder", "Sub-folder per archive", "Sous-dossier par archive"),
            ],
        },
    ),
    d(
        5,
        "try-passwords",
        "Try passwords from the list",
        "Essayer les mots de passe de la liste",
        "The list is on the Extraction screen.",
        "La liste est sur l'écran Extraction.",
        Kind::Toggle(true),
    ),
    planned(adv(d(
        5,
        "nested",
        "Extract archives inside archives",
        "Extraire les archives contenues dans les archives",
        "",
        "",
        Kind::Toggle(true),
    ))),
    // Video
    d(
        6,
        "video-quality",
        "Video quality for automatic starts",
        "Qualité vidéo pour les lancements automatiques",
        "In the link grabber you choose per video.",
        "Dans la capture de liens, vous choisissez par vidéo.",
        Kind::Select {
            default: "best",
            options: &[
                ("best", "Best available", "Meilleure disponible"),
                ("2160", "Best up to 2160p", "Jusqu'à 2160p"),
                ("1080", "Best up to 1080p", "Jusqu'à 1080p"),
                ("720", "Best up to 720p", "Jusqu'à 720p"),
                ("audio", "Audio only", "Audio seul"),
                ("mp3", "Audio only (MP3)", "Audio seul (MP3)"),
            ],
        },
    ),
    d(
        6,
        "video-container",
        "Container",
        "Conteneur",
        "Used when video and audio are merged (needs ffmpeg).",
        "Utilisé quand vidéo et audio sont fusionnés (nécessite ffmpeg).",
        Kind::Select {
            default: "mp4",
            options: &[("mp4", "mp4", "mp4"), ("mkv", "mkv", "mkv"), ("webm", "webm", "webm")],
        },
    ),
    d(6, "subtitles", "Download subtitles", "Télécharger les sous-titres", "", "", Kind::Toggle(false)),
    d(
        6,
        "subtitle-langs",
        "Subtitle languages",
        "Langues des sous-titres",
        "Comma separated, e.g. en, fr.",
        "Séparées par des virgules, par ex. en, fr.",
        Kind::Text("en, fr"),
    ),
    adv(d(
        6,
        "embed-metadata",
        "Embed thumbnail and metadata",
        "Intégrer la miniature et les métadonnées",
        "Needs ffmpeg.",
        "Nécessite ffmpeg.",
        Kind::Toggle(true),
    )),
    d(
        6,
        "update-yt-dlp",
        "Keep yt-dlp up to date",
        "Garder yt-dlp à jour",
        "Checks for a new version when BDownloader starts.",
        "Vérifie une nouvelle version au démarrage.",
        Kind::Toggle(true),
    ),
    // Torrent
    planned(d(7, "torrent-port", "Listening port", "Port d'écoute", "", "", num(51413, 1024, 65535, ""))),
    planned(d(
        7,
        "torrent-upload",
        "Upload limit",
        "Limite d'envoi",
        "0 means unlimited.",
        "0 = illimité.",
        num(2, 0, 1000, "MB/s"),
    )),
    planned(d(7, "torrent-ratio", "Stop seeding at ratio", "Arrêter le partage au ratio", "", "", num(1, 0, 10, ""))),
    // Clipboard & Click'n'Load
    d(
        8,
        "clicknload-port",
        "Click'n'Load port",
        "Port Click'n'Load",
        "9666 is the standard port used by link sites.",
        "9666 est le port standard utilisé par les sites de liens.",
        num(9666, 1024, 65535, ""),
    ),
    // Interface
    d(
        9,
        "theme",
        "Theme",
        "Thème",
        "",
        "",
        Kind::Select {
            default: "system",
            options: &[
                ("system", "Follow Windows", "Comme Windows"),
                ("light", "Light", "Clair"),
                ("dark", "Dark", "Sombre"),
            ],
        },
    ),
    d(
        9,
        "density",
        "List density",
        "Densité des listes",
        "",
        "",
        Kind::Select {
            default: "comfortable",
            options: &[("comfortable", "Comfortable", "Confortable"), ("compact", "Compact", "Compacte")],
        },
    ),
    d(
        9,
        "notifications",
        "Notifications",
        "Notifications",
        "",
        "",
        Kind::Select {
            default: "finished-errors",
            options: &[
                ("all", "Everything", "Tout"),
                ("finished-errors", "Finished and errors", "Terminés et erreurs"),
                ("errors", "Errors only", "Erreurs seulement"),
                ("none", "None", "Aucune"),
            ],
        },
    ),
    adv(d(
        9,
        "speed-in-title",
        "Show speed in the window title",
        "Afficher la vitesse dans le titre de la fenêtre",
        "",
        "",
        Kind::Toggle(false),
    )),
    // Language
    d(
        10,
        "language",
        "Interface language",
        "Langue de l'interface",
        "More languages are being added.",
        "D'autres langues arrivent.",
        Kind::Select { default: "en", options: &[("en", "English", "English"), ("fr", "Français", "Français")] },
    ),
    // Advanced
    restart(adv(d(
        12,
        "log-level",
        "Log level",
        "Niveau de journal",
        "",
        "",
        Kind::Select {
            default: "info",
            options: &[
                ("error", "Error", "Erreur"),
                ("warn", "Warning", "Avertissement"),
                ("info", "Info", "Info"),
                ("debug", "Debug", "Débogage"),
            ],
        },
    ))),
    planned(adv(d(
        12,
        "portable",
        "Portable mode",
        "Mode portable",
        "Keep settings next to the .exe.",
        "Garder les réglages à côté du .exe.",
        Kind::Toggle(false),
    ))),
];

pub fn def(id: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.id == id)
}

impl Def {
    pub fn label(&self) -> &'static str {
        if i18n::is_french() {
            self.fr
        } else {
            self.en
        }
    }

    pub fn hint(&self) -> &'static str {
        if i18n::is_french() {
            self.hint_fr
        } else {
            self.hint_en
        }
    }
}

/// Current value as text (toggles are "true"/"false", selects the stored value).
pub fn get(s: &Settings, id: &str) -> String {
    match id {
        "start-on-launch" => s.start_on_launch.to_string(),
        "close-to-tray" => s.close_to_tray.to_string(),
        "minimize-to-tray" => s.minimize_to_tray.to_string(),
        "download-dir" => s.download_dir.display().to_string(),
        "auto-start-grabbed" => s.auto_start_grabbed.to_string(),
        "parallel" => s.max_parallel.to_string(),
        "connections" => s.connections_per_download.to_string(),
        "speed-limit" => s.speed_limit_kib.to_string(),
        "clicknload-port" => s.clicknload_port.to_string(),
        "language" => s.language.clone(),
        "after-extract" => if s.delete_archives { "delete" } else { "keep" }.to_owned(),
        "theme" => match s.theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
        .to_owned(),
        other => match s.option(other) {
            Some(v) => v.to_owned(),
            None => default_text(other),
        },
    }
}

pub fn default_text(id: &str) -> String {
    match def(id).map(|d| d.kind) {
        Some(Kind::Toggle(v)) => v.to_string(),
        Some(Kind::Number { default, .. }) => default.to_string(),
        Some(Kind::Select { default, .. }) => default.to_owned(),
        Some(Kind::Text(v)) => v.to_owned(),
        _ => String::new(),
    }
}

/// Stores a value given as text.
pub fn set(s: &mut Settings, id: &str, value: &str) {
    let b = value == "true";
    let n = value.parse::<i64>().unwrap_or(0).max(0);
    match id {
        "start-on-launch" => s.start_on_launch = b,
        "close-to-tray" => s.close_to_tray = b,
        "minimize-to-tray" => s.minimize_to_tray = b,
        "download-dir" => {
            if !value.trim().is_empty() {
                s.download_dir = value.trim().into();
            }
        }
        "auto-start-grabbed" => s.auto_start_grabbed = b,
        "parallel" => s.max_parallel = n.clamp(1, 50) as usize,
        "connections" => s.connections_per_download = n.clamp(1, 32) as usize,
        "speed-limit" => s.speed_limit_kib = n as u64,
        "clicknload-port" => s.clicknload_port = n.clamp(1024, 65535) as u16,
        "language" => s.language = value.to_owned(),
        "after-extract" => s.delete_archives = value == "delete",
        "theme" => {
            s.theme = match value {
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                _ => ThemeMode::System,
            }
        }
        other => s.set_option(other, value),
    }
}

/// Puts every option of a section back to its default.
pub fn reset_category(s: &mut Settings, cat: usize) {
    for def in DEFS.iter().filter(|d| d.cat == cat && d.status != Status::Planned) {
        if def.id == "download-dir" {
            s.download_dir = Settings::default().download_dir;
            continue;
        }
        let value = default_text(def.id);
        set(s, def.id, &value);
    }
}
