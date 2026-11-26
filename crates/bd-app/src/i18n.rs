//! Languages. The Slint interface is translated with gettext files bundled at
//! build time (`translations/<lang>/LC_MESSAGES/bd-app.po`); the few texts
//! built in Rust (statuses, notifications) go through [`t`].

use std::sync::atomic::{AtomicUsize, Ordering};

/// (code, native name), in the order shown in Settings.
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("fr", "Français")];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn is_french() -> bool {
    LANGUAGES[CURRENT.load(Ordering::Relaxed)].0 == "fr"
}

pub fn index_of(code: &str) -> usize {
    LANGUAGES.iter().position(|(c, _)| *c == code).unwrap_or(0)
}

pub fn select(index: usize) -> &'static str {
    let index = index.min(LANGUAGES.len() - 1);
    CURRENT.store(index, Ordering::Relaxed);
    let code = LANGUAGES[index].0;
    if let Err(e) = slint::select_bundled_translation(code) {
        tracing::warn!(error = ?e, code, "cannot select translation");
    }
    code
}

fn french(text: &str) -> Option<&'static str> {
    Some(match text {
        "Queued" => "En attente",
        "Downloading" => "Téléchargement",
        "Connecting…" => "Connexion…",
        "Resolving link…" => "Résolution du lien…",
        "Waiting for a connection" => "En attente d'une connexion",
        "Paused" => "En pause",
        "Finished" => "Terminé",
        "Checking…" => "Vérification…",
        "Online" => "En ligne",
        "Checked when downloading" => "Vérifié au téléchargement",
        "File not found (404)" => "Fichier introuvable (404)",
        "Access denied (403)" => "Accès refusé (403)",
        "Too many requests" => "Trop de requêtes",
        "Network error" => "Erreur réseau",
        "Queue started" => "File démarrée",
        "All downloads paused" => "Tous les téléchargements en pause",
        "Link copied" => "Lien copié",
        "No link found in the text" => "Aucun lien trouvé dans le texte",
        "Download finished" => "Téléchargement terminé",
        "Download failed" => "Échec du téléchargement",
        "Package finished" => "Paquet terminé",
        "Extraction finished" => "Extraction terminée",
        "Extraction failed" => "Échec de l'extraction",
        "Not installed" => "Non installé",
        "Installed" => "Installé",
        "Premium" => "Premium",
        "Debrid" => "Debrid",
        "Saved" => "Enregistré",
        "Open BDownloader" => "Ouvrir BDownloader",
        "Add links from clipboard" => "Ajouter les liens du presse-papiers",
        "Watch clipboard" => "Surveiller le presse-papiers",
        "Pause all" => "Tout mettre en pause",
        "Resume all" => "Tout reprendre",
        "Speed limit" => "Limite de débit",
        "Unlimited" => "Illimitée",
        "Quit" => "Quitter",
        "Videos" => "Vidéos",
        "Links added:" => "Liens ajoutés :",
        "Documentation coming soon" => "La documentation arrive bientôt",
        "Nothing is downloading" => "Aucun téléchargement en cours",
        "Rename" => "Renommer",
        "Speed limit for this file" => "Limite de débit pour ce fichier",
        "In MB/s, any value. 0 removes the limit." => "En Mo/s, n'importe quelle valeur. 0 retire la limite.",
        "Applies at the next start" => "S'applique au prochain lancement",
        "Free mode…" => "Mode gratuit…",
        _ => return None,
    })
}

/// Statuses with a variable part, like "Free mode, DataNodes (free) wait: 12 s".
fn french_status(text: &str) -> Option<String> {
    if let Some((what, left)) = text.rsplit_once(": ") {
        if let Some(hoster) = what.strip_prefix("Free mode, ").and_then(|w| w.strip_suffix(" wait")) {
            return Some(format!("Mode gratuit, attente {hoster} : {left}"));
        }
        if what == "Free mode, 1fichier asks to wait" {
            return Some(format!("Mode gratuit, 1fichier impose une attente : {left}"));
        }
    }
    Some(
        match text {
            "Captcha: waiting for your answer" => "Captcha : en attente de votre réponse",
            "Browser check: finish it in the BDownloader window" => {
                "Vérification navigateur : à terminer dans la fenêtre de BDownloader"
            }
            "Finish the check in BDownloader" => "Terminez la vérification dans BDownloader",
            "Getting a new IP…" => "Nouvelle adresse IP…",
            "Type the code to continue the download" => "Tapez le code pour continuer le téléchargement",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Translates a fixed English text built in Rust.
pub fn t(text: &str) -> String {
    match LANGUAGES[CURRENT.load(Ordering::Relaxed)].0 {
        "fr" => french(text).map(str::to_owned).or_else(|| french_status(text)).unwrap_or_else(|| text.to_owned()),
        _ => text.to_owned(),
    }
}
