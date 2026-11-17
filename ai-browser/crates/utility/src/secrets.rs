use std::path::Path;
use sha2::{Digest, Sha256};

fn account(profile: &Path, name: &str) -> String {
    let path = profile.canonicalize().unwrap_or_else(|_| profile.to_path_buf());
    let path = path.to_string_lossy();
    #[cfg(windows)]
    let path = path.to_lowercase();
    format!("{:x}:{name}", Sha256::digest(path.as_bytes()))
}

#[cfg(not(test))]
pub fn get(profile: &Path, name: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new("ai-browser", &account(profile, name)).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("cannot access OS credential store: {error}")),
    }
}

#[cfg(not(test))]
pub fn set(profile: &Path, name: &str, value: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new("ai-browser", &account(profile, name)).map_err(|e| e.to_string())?;
    match value.filter(|value| !value.is_empty()) {
        Some(value) => entry.set_password(value),
        None => match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            result => result,
        },
    }.map_err(|error| format!("cannot update OS credential store: {error}"))
}

#[cfg(test)]
mod testing {
    use super::*;
    use std::{collections::HashMap, sync::{Mutex, OnceLock}};
    fn vault() -> &'static Mutex<HashMap<String, String>> {
        static VAULT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        VAULT.get_or_init(Default::default)
    }
    pub fn get(profile: &Path, name: &str) -> Result<Option<String>, String> {
        Ok(vault().lock().map_err(|e| e.to_string())?.get(&account(profile, name)).cloned())
    }
    pub fn set(profile: &Path, name: &str, value: Option<&str>) -> Result<(), String> {
        let mut vault = vault().lock().map_err(|e| e.to_string())?;
        let key = account(profile, name);
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            vault.insert(key, value.to_string());
        } else { vault.remove(&key); }
        Ok(())
    }
}
#[cfg(test)]
pub use testing::{get, set};

pub fn migrate_provider(profile: &Path) -> Result<(), String> {
    let path = profile.join("ai.key");
    match std::fs::read_to_string(&path) {
        Ok(value) => {
            if get(profile, "provider.custom")?.is_none() && !value.trim().is_empty() {
                set(profile, "provider.custom", Some(value.trim()))?;
            }
            std::fs::remove_file(path).map_err(|e| format!("cannot remove migrated legacy key: {e}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot read legacy credential: {error}")),
    }
}
