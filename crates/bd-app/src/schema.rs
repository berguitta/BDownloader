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
    /// A button running an action (English, French label).
    Button(&'static str, &'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// Takes effect after restarting BDownloader.
    Restart,
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

/// (English, French). Some sections have extra content built in the UI
/// (tools for video, the rule editor for the scheduler, modules).
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
    ("Scheduler", "Planificateur"),
    ("Reconnect", "Reconnexion"),
    ("Event scripts", "Scripts d'événements"),
    ("Checksums", "Sommes de contrôle"),
    ("Interface", "Interface"),
    ("Language", "Langue"),
    ("Modules", "Modules"),
    ("Advanced", "Avancé"),
];

pub const MODULES_CATEGORY: usize = 15;

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
        "Power actions module.",
        "Module Actions d'alimentation.",
        Kind::Select {
            default: "nothing",
            options: &[
                ("nothing", "Do nothing", "Ne rien faire"),
                ("close", "Close BDownloader", "Fermer BDownloader"),
                ("sleep", "Sleep", "Mettre en veille"),
                ("hibernate", "Hibernate", "Mettre en veille prolongée"),
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
    d(
        1,
        "secure-dns",
        "Secure DNS",
        "DNS sécurisé",
        "Only for BDownloader. Windows and your other programs are not changed.",
        "Seulement pour BDownloader. Windows et vos autres programmes ne changent pas.",
        Kind::Select {
            default: "off",
            options: &[
                ("off", "Off (DNS of the network card)", "Désactivé (DNS de la carte réseau)"),
                ("cloudflare", "Cloudflare (1.1.1.1)", "Cloudflare (1.1.1.1)"),
                ("quad9", "Quad9 (9.9.9.9)", "Quad9 (9.9.9.9)"),
                ("google", "Google (8.8.8.8)", "Google (8.8.8.8)"),
                ("custom", "My own server", "Mon propre serveur"),
            ],
        },
    ),
    adv(d(
        1,
        "secure-dns-url",
        "Own secure DNS server",
        "Mon serveur DNS sécurisé",
        "DNS over HTTPS address, like https://dns.example/dns-query",
        "Adresse DNS over HTTPS, par exemple https://dns.example/dns-query",
        Kind::Text(""),
    )),
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
    d(
        2,
        "min-free",
        "Pause when free disk space is under",
        "Mettre en pause sous cet espace disque libre",
        "Checked on the download folder's drive. 0 turns it off.",
        "Vérifié sur le disque du dossier de téléchargement. 0 désactive.",
        num(5, 0, 2000, "GB"),
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
        "Wait timers and site steps handled in the background when no account can be used.",
        "Temps d'attente et étapes du site gérés en arrière-plan quand aucun compte n'est utilisable.",
        Kind::Toggle(true),
    ),
    adv(d(
        3,
        "account-check",
        "Re-check accounts every",
        "Revérifier les comptes toutes les",
        "Keeps the premium status and expiry up to date.",
        "Garde à jour le statut premium et l'échéance.",
        num(6, 1, 72, "hours"),
    )),
    // Captcha
    d(
        4,
        "captcha-mode",
        "When a site asks \"I am not a robot\"",
        "Quand un site demande « je ne suis pas un robot »",
        "Codes and images to copy are always typed in BDownloader.",
        "Les codes et images à recopier se tapent toujours dans BDownloader.",
        Kind::Select {
            default: "browser",
            options: &[
                ("browser", "Open the page in my browser", "Ouvrir la page dans mon navigateur"),
                ("notify", "Notify me only", "Me prévenir seulement"),
                ("skip", "Skip the file (no captcha at all)", "Ignorer le fichier (aucun captcha)"),
            ],
        },
    ),
    d(
        4,
        "captcha-timeout",
        "Wait for my answer",
        "Attendre ma réponse",
        "After that, the download is retried later.",
        "Ensuite, le téléchargement sera réessayé plus tard.",
        num(120, 20, 900, "s"),
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
    adv(d(
        5,
        "nested",
        "Extract archives inside archives",
        "Extraire les archives contenues dans les archives",
        "One level deep.",
        "Sur un niveau.",
        Kind::Toggle(true),
    )),
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
    restart(d(7, "torrent-port", "Listening port", "Port d'écoute", "", "", num(51413, 1024, 65535, ""))),
    restart(d(
        7,
        "torrent-upload",
        "Upload limit",
        "Limite d'envoi",
        "0 means unlimited.",
        "0 = illimité.",
        num(2048, 0, 1_000_000, "KB/s"),
    )),
    d(
        7,
        "torrent-ratio",
        "Stop seeding at ratio",
        "Arrêter le partage au ratio",
        "1 means you give back as much as you downloaded. 0 stops at once.",
        "1 = vous rendez autant que vous avez téléchargé. 0 arrête tout de suite.",
        num(1, 0, 10, "×"),
    ),
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
    // Scheduler: rules are edited in their own editor.
    // Reconnect
    d(
        10,
        "reconnect-method",
        "How to get a new IP",
        "Comment obtenir une nouvelle IP",
        "",
        "",
        Kind::Select {
            default: "upnp",
            options: &[
                ("upnp", "Internet box (UPnP)", "Box internet (UPnP)"),
                ("script", "Custom command", "Commande personnalisée"),
            ],
        },
    ),
    d(
        10,
        "reconnect-script",
        "Custom command",
        "Commande personnalisée",
        "Run with cmd /C, e.g. a script that restarts your connection.",
        "Lancée avec cmd /C, par ex. un script qui relance votre connexion.",
        Kind::Text(""),
    ),
    d(
        10,
        "reconnect-after",
        "Reconnect when a hoster asks to wait at least",
        "Reconnecter quand un hébergeur fait attendre au moins",
        "",
        "",
        num(5, 1, 120, "min"),
    ),
    d(
        10,
        "reconnect-now",
        "Test the reconnection",
        "Tester la reconnexion",
        "Your internet is cut for a few seconds.",
        "Votre connexion est coupée quelques secondes.",
        Kind::Button("Reconnect now", "Reconnecter maintenant"),
    ),
    // Event scripts
    d(
        11,
        "discord-webhook",
        "Discord webhook URL",
        "URL du webhook Discord",
        "Server settings › Integrations › Webhooks.",
        "Paramètres du serveur › Intégrations › Webhooks.",
        Kind::Text(""),
    ),
    d(
        11,
        "telegram-token",
        "Telegram bot token",
        "Jeton du bot Telegram",
        "Given by @BotFather.",
        "Fourni par @BotFather.",
        Kind::Text(""),
    ),
    d(11, "telegram-chat", "Telegram chat id", "Identifiant de la conversation Telegram", "", "", Kind::Text("")),
    d(
        11,
        "script-command",
        "Command to run",
        "Commande à lancer",
        "{event}, {name}, {path} and {package} are replaced.",
        "{event}, {name}, {path} et {package} sont remplacés.",
        Kind::Text(""),
    ),
    d(
        11,
        "event-finished",
        "When a download finishes",
        "Quand un téléchargement se termine",
        "",
        "",
        Kind::Toggle(false),
    ),
    d(11, "event-package", "When a package finishes", "Quand un paquet se termine", "", "", Kind::Toggle(true)),
    d(11, "event-failed", "When a download fails", "Quand un téléchargement échoue", "", "", Kind::Toggle(true)),
    d(11, "event-extracted", "When an extraction ends", "Quand une extraction se termine", "", "", Kind::Toggle(true)),
    d(11, "event-queue-done", "When the queue is finished", "Quand la file est terminée", "", "", Kind::Toggle(true)),
    d(11, "event-test", "Send a test message", "Envoyer un message de test", "", "", Kind::Button("Test", "Tester")),
    // Checksums
    d(
        12,
        "verify-hash",
        "Verify packages with their checksum files",
        "Vérifier les paquets avec leurs fichiers de sommes de contrôle",
        "Uses .sfv, .md5, .sha1 and .sha256 files found next to the downloads.",
        "Utilise les fichiers .sfv, .md5, .sha1 et .sha256 trouvés à côté des téléchargements.",
        Kind::Toggle(true),
    ),
    // Interface
    d(
        13,
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
        13,
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
        13,
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
    d(
        13,
        "mini-corner",
        "Mini window corner",
        "Coin de la mini fenêtre",
        "",
        "",
        Kind::Select {
            default: "top-right",
            options: &[
                ("top-right", "Top right", "En haut à droite"),
                ("top-left", "Top left", "En haut à gauche"),
                ("bottom-right", "Bottom right", "En bas à droite"),
                ("bottom-left", "Bottom left", "En bas à gauche"),
            ],
        },
    ),
    adv(d(
        13,
        "speed-in-title",
        "Show speed in the window title",
        "Afficher la vitesse dans le titre de la fenêtre",
        "",
        "",
        Kind::Toggle(false),
    )),
    adv(d(
        13,
        "keep-screen-on",
        "Keep the screen on during downloads",
        "Garder l'écran allumé pendant les téléchargements",
        "Anti-standby module. By default only the computer stays awake.",
        "Module anti-veille. Par défaut seul l'ordinateur reste éveillé.",
        Kind::Toggle(false),
    )),
    // Language
    d(
        14,
        "language",
        "Interface language",
        "Langue de l'interface",
        "More languages are being added.",
        "D'autres langues arrivent.",
        Kind::Select { default: "en", options: &[("en", "English", "English"), ("fr", "Français", "Français")] },
    ),
    // Advanced
    restart(adv(d(
        16,
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
    restart(adv(d(
        16,
        "portable",
        "Portable mode",
        "Mode portable",
        "Keep settings and data next to the .exe, for a USB stick.",
        "Garder réglages et données à côté du .exe, pour une clé USB.",
        Kind::Toggle(false),
    ))),
];

/// Longer explanations shown behind the "i" button: (English, French).
pub fn info(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "clicknload-port" => (
            "Click'n'Load lets download sites send their links straight to BDownloader: when you click the site's Click'n'Load button, the links appear in the link grabber. BDownloader listens only on your own computer (127.0.0.1), never on the network.",
            "Click'n'Load permet aux sites de téléchargement d'envoyer leurs liens directement à BDownloader : quand vous cliquez sur le bouton Click'n'Load du site, les liens arrivent dans la capture de liens. BDownloader n'écoute que sur votre propre ordinateur (127.0.0.1), jamais sur le réseau.",
        ),
        "proxy" => (
            "A proxy sends all traffic through another server. Keep \"System settings\" unless you know you need one.",
            "Un proxy fait passer tout le trafic par un autre serveur. Gardez « Réglages du système » sauf si vous savez en avoir besoin.",
        ),
        "per-host" => (
            "Some servers refuse too many simultaneous connections. BDownloader starts with this limit and lowers it by itself when a server answers \"too many requests\".",
            "Certains serveurs refusent trop de connexions simultanées. BDownloader part de cette limite et la baisse tout seul quand un serveur répond « trop de requêtes ».",
        ),
        "connections" => (
            "A file is cut into parts downloaded at the same time, which is often much faster. Free hosters usually allow only one.",
            "Le fichier est découpé en parties téléchargées en même temps, souvent bien plus vite. Les hébergeurs gratuits n'en autorisent en général qu'une.",
        ),
        "preallocate" => (
            "The full file size is reserved on disk at the start: the file is not fragmented and you know at once if there is enough space.",
            "La taille complète du fichier est réservée sur le disque dès le début : le fichier n'est pas fragmenté et vous savez tout de suite s'il y a assez de place.",
        ),
        "secure-dns" => (
            "To open a site, BDownloader first asks a DNS server for its address. Some internet providers answer with a block page for some file hosters. With secure DNS, BDownloader asks the server you choose, encrypted (DNS over HTTPS). Only BDownloader's downloads use it: Windows, your network card and your other programs keep their DNS. Turn it off to go back. Videos (yt-dlp) and torrents keep the system DNS.",
            "Pour ouvrir un site, BDownloader demande d'abord son adresse à un serveur DNS. Certains fournisseurs d'accès répondent par une page de blocage pour certains hébergeurs. Avec le DNS sécurisé, BDownloader interroge le serveur choisi, en chiffré (DNS over HTTPS). Seuls les téléchargements de BDownloader l'utilisent : Windows, la carte réseau et vos autres programmes gardent leur DNS. Désactivez pour revenir comme avant. Les vidéos (yt-dlp) et les torrents gardent le DNS du système.",
        ),
        "prefer-debrid" => (
            "With both a premium account and a debrid service for the same hoster, use the debrid service first.",
            "Avec à la fois un compte premium et un service debrid pour le même hébergeur, utiliser d'abord le service debrid.",
        ),
        "free-fallback" => (
            "Without an account, BDownloader downloads like a visitor of the site, in the background: it clicks through the free download pages and waits the timers the site imposes (the countdown shows in the list). When the site asks to copy a code, a small window lets you type it in BDownloader.",
            "Sans compte, BDownloader télécharge comme un visiteur du site, en arrière-plan : il passe les pages de téléchargement gratuit et attend les délais imposés (le compte à rebours s'affiche dans la liste). Quand le site demande de recopier un code, une petite fenêtre vous permet de le taper dans BDownloader.",
        ),
        "captcha-mode" => (
            "BDownloader never solves a captcha for you. Codes and images to copy are shown in BDownloader. The \"I am not a robot\" boxes (reCAPTCHA, hCaptcha, Turnstile) only work in a browser: BDownloader opens the page, notifies you, or skips the file. A debrid account avoids them.",
            "BDownloader ne résout jamais un captcha à votre place. Les codes et images à recopier s'affichent dans BDownloader. Les cases « je ne suis pas un robot » (reCAPTCHA, hCaptcha, Turnstile) ne marchent que dans un navigateur : BDownloader ouvre la page, vous prévient, ou ignore le fichier. Un compte debrid les évite.",
        ),
        "video-container" => (
            "The file format of merged videos. mp4 plays everywhere; mkv supports more audio and subtitle tracks.",
            "Le format des vidéos fusionnées. mp4 se lit partout ; mkv accepte plus de pistes audio et de sous-titres.",
        ),
        "torrent-ratio" => (
            "After downloading, BDownloader keeps sharing until it has sent this many times the size of the torrent, then stops. Sharing back keeps torrents alive.",
            "Après le téléchargement, BDownloader continue de partager jusqu'à avoir envoyé ce multiple de la taille du torrent, puis s'arrête. Partager en retour fait vivre les torrents.",
        ),
        "reconnect-method" => (
            "Free hosters count the downloads per IP address. Getting a new IP from your internet box lets you skip their wait timer. UPnP must be enabled on the box.",
            "Les hébergeurs gratuits comptent les téléchargements par adresse IP. Obtenir une nouvelle IP de votre box permet d'éviter leur temps d'attente. L'UPnP doit être activé sur la box.",
        ),
        "script-command" => (
            "Any program or script, started after the chosen events. Example: a script that converts videos with ffmpeg, or that moves files.",
            "N'importe quel programme ou script, lancé après les événements choisis. Exemple : un script qui convertit des vidéos avec ffmpeg, ou qui déplace des fichiers.",
        ),
        "verify-hash" => (
            "A checksum is a fingerprint of a file. When the uploader adds .sfv or .md5 files, BDownloader compares them to detect damaged downloads.",
            "Une somme de contrôle est l'empreinte d'un fichier. Quand l'auteur fournit des fichiers .sfv ou .md5, BDownloader les compare pour détecter les téléchargements abîmés.",
        ),
        "if-exists" => (
            "When a file with the same name is already in the folder: keep both (the new one becomes \"name (1)\"), skip the download, or replace the old file.",
            "Quand un fichier du même nom est déjà dans le dossier : garder les deux (le nouveau devient « nom (1) »), ignorer le téléchargement, ou remplacer l'ancien.",
        ),
        "portable" => (
            "Everything (settings, queue, accounts list) is kept in a data folder next to BDownloader.exe, so you can carry it on a USB stick.",
            "Tout (réglages, file, liste des comptes) est gardé dans un dossier data à côté de BDownloader.exe, pour l'emporter sur une clé USB.",
        ),
        _ => return None,
    })
}

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

    pub fn info(&self) -> &'static str {
        info(self.id).map(|(en, fr)| if i18n::is_french() { fr } else { en }).unwrap_or_default()
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
        "portable" => Settings::portable_dir().is_some().to_string(),
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
        "portable" => {
            if let Err(e) = s.set_portable(b) {
                tracing::warn!(error = %e, "cannot change portable mode");
            }
        }
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
    for def in DEFS.iter().filter(|d| d.cat == cat && !matches!(d.kind, Kind::Button(..)) && d.id != "portable") {
        if def.id == "download-dir" {
            s.download_dir = Settings::default().download_dir;
            continue;
        }
        let value = default_text(def.id);
        set(s, def.id, &value);
    }
}
