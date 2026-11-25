//! Languages. The Slint interface is translated with gettext files bundled at
//! build time (`translations/<lang>/LC_MESSAGES/bd-app.po`); the few texts
//! built in Rust (statuses, notifications) go through [`t`].

use std::sync::atomic::{AtomicUsize, Ordering};

/// (code, native name), in the order shown in Settings.
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("fr", "Français")];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

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
        "Needs an account to check" => "Vérifiable avec un compte",
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
        _ => return None,
    })
}

/// Translates a fixed English text built in Rust.
pub fn t(text: &str) -> String {
    match LANGUAGES[CURRENT.load(Ordering::Relaxed)].0 {
        "fr" => french(text).map(str::to_owned).unwrap_or_else(|| text.to_owned()),
        _ => text.to_owned(),
    }
}
