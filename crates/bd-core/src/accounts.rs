//! Premium and debrid accounts.
//!
//! The list (service, user name, enabled) is stored in `accounts.toml`; the
//! secret (password, API key or token) goes to the Windows Credential
//! Manager, never to a plain file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "BDownloader";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    Debrid,
    Premium,
}

/// A service accounts can be added for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ServiceKind,
    /// Whether a user name is needed besides the secret.
    pub needs_username: bool,
    pub secret_label: &'static str,
    pub help: &'static str,
}

pub const SERVICES: &[Service] = &[
    Service {
        id: "real-debrid",
        name: "Real-Debrid",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API token",
        help: "real-debrid.com/apitoken",
    },
    Service {
        id: "alldebrid",
        name: "AllDebrid",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API key",
        help: "alldebrid.com/apikeys",
    },
    Service {
        id: "premiumize",
        name: "Premiumize",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API key",
        help: "premiumize.me/account",
    },
    Service {
        id: "debrid-link",
        name: "Debrid-Link",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API key",
        help: "debrid-link.com/webapp/apikey",
    },
    Service {
        id: "torbox",
        name: "TorBox",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API key",
        help: "torbox.app/settings",
    },
    Service {
        id: "offcloud",
        name: "Offcloud",
        kind: ServiceKind::Debrid,
        needs_username: false,
        secret_label: "API key",
        help: "offcloud.com/#/account",
    },
    Service {
        id: "linksnappy",
        name: "LinkSnappy",
        kind: ServiceKind::Debrid,
        needs_username: true,
        secret_label: "Password",
        help: "Your LinkSnappy login",
    },
    Service {
        id: "mega-debrid",
        name: "Mega-Debrid",
        kind: ServiceKind::Debrid,
        needs_username: true,
        secret_label: "Password",
        help: "Your Mega-Debrid login",
    },
    Service {
        id: "1fichier",
        name: "1fichier",
        kind: ServiceKind::Premium,
        needs_username: false,
        secret_label: "API key",
        help: "1fichier.com/console/params.pl",
    },
    Service {
        id: "rapidgator",
        name: "Rapidgator",
        kind: ServiceKind::Premium,
        needs_username: true,
        secret_label: "Password",
        help: "Your Rapidgator login",
    },
];

pub fn service(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub service: String,
    /// User name, or a label for token-only services.
    pub username: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Account {
    fn keyring_user(&self) -> String {
        format!("{}:{}", self.service, self.username)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AccountsFile {
    #[serde(default)]
    accounts: Vec<Account>,
}

/// Where secrets are kept. Swappable for tests.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn get(&self, key: &str) -> Option<String>;
    fn delete(&self, key: &str);
}

/// Windows Credential Manager (or the platform keychain).
pub struct SystemKeyring;

impl SecretStore for SystemKeyring {
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.set_password(secret)).map_err(|e| e.to_string())
    }

    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, key).ok()?.get_password().ok()
    }

    fn delete(&self, key: &str) {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
            let _ = entry.delete_credential();
        }
    }
}

pub struct AccountStore {
    path: Option<PathBuf>,
    accounts: Vec<Account>,
    secrets: Box<dyn SecretStore>,
}

impl AccountStore {
    pub fn load(path: Option<PathBuf>, secrets: Box<dyn SecretStore>) -> Self {
        let accounts = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str::<AccountsFile>(&t).ok())
            .map(|f| f.accounts)
            .unwrap_or_default();
        AccountStore { path, accounts, secrets }
    }

    pub fn default_path() -> Option<PathBuf> {
        crate::settings::Settings::default_path().map(|p| p.with_file_name("accounts.toml"))
    }

    pub fn list(&self) -> &[Account] {
        &self.accounts
    }

    pub fn secret(&self, account: &Account) -> Option<String> {
        self.secrets.get(&account.keyring_user())
    }

    /// Adds or replaces an account.
    pub fn upsert(&mut self, account: Account, secret: &str) -> Result<(), String> {
        if service(&account.service).is_none() {
            return Err(format!("unknown service `{}`", account.service));
        }
        self.secrets.set(&account.keyring_user(), secret)?;
        self.accounts.retain(|a| !(a.service == account.service && a.username == account.username));
        self.accounts.push(account);
        self.save()
    }

    pub fn remove(&mut self, service: &str, username: &str) -> Result<(), String> {
        if let Some(pos) = self.accounts.iter().position(|a| a.service == service && a.username == username) {
            let account = self.accounts.remove(pos);
            self.secrets.delete(&account.keyring_user());
        }
        self.save()
    }

    pub fn set_enabled(&mut self, service: &str, username: &str, enabled: bool) -> Result<(), String> {
        if let Some(a) = self.accounts.iter_mut().find(|a| a.service == service && a.username == username) {
            a.enabled = enabled;
        }
        self.save()
    }

    /// Enabled accounts with their secret, for the link resolver.
    pub fn credentials(&self) -> Vec<(Account, String)> {
        self.accounts.iter().filter(|a| a.enabled).filter_map(|a| Some((a.clone(), self.secret(a)?))).collect()
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        write_file(path, &AccountsFile { accounts: self.accounts.clone() }).map_err(|e| e.to_string())
    }
}

fn write_file(path: &Path, file: &AccountsFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string_pretty(file).map_err(std::io::Error::other)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct MemorySecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemorySecrets {
        fn set(&self, key: &str, secret: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(key.into(), secret.into());
            Ok(())
        }
        fn get(&self, key: &str) -> Option<String> {
            self.0.lock().unwrap().get(key).cloned()
        }
        fn delete(&self, key: &str) {
            self.0.lock().unwrap().remove(key);
        }
    }

    #[test]
    fn stores_list_in_file_and_secret_aside() {
        let dir = std::env::temp_dir().join(format!("bd-accounts-{}", std::process::id()));
        let path = dir.join("accounts.toml");
        let mut store = AccountStore::load(Some(path.clone()), Box::new(MemorySecrets::default()));
        let account = Account { service: "real-debrid".into(), username: "me".into(), enabled: true };
        store.upsert(account.clone(), "token-123").unwrap();
        assert_eq!(store.credentials(), vec![(account.clone(), "token-123".to_owned())]);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("real-debrid") && !text.contains("token-123"), "secret must not be in the file");

        store.set_enabled("real-debrid", "me", false).unwrap();
        assert!(store.credentials().is_empty());
        store.remove("real-debrid", "me").unwrap();
        assert!(store.list().is_empty());
        assert!(store.upsert(Account { service: "nope".into(), username: "x".into(), enabled: true }, "s").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
