//! SQL Server passwords in the OS credential store (macOS Keychain, Windows Credential
//! Manager, Linux kernel keyring), never in settings.json.

const SERVICE: &str = "DotCraft SQL Server";

pub trait Secrets: Send + Sync {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, password: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The OS store through the `keyring` crate.
pub struct Keychain;

impl Secrets for Keychain {
    fn get(&self, account: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, account).ok()?.get_password().ok()
    }
    fn set(&self, account: &str, password: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account).and_then(|e| e.set_password(password)).map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, account).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// An in-memory store for tests and captures.
#[cfg(test)]
#[derive(Default)]
pub struct Memory(pub std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(test)]
impl Secrets for Memory {
    fn get(&self, account: &str) -> Option<String> { self.0.lock().ok()?.get(account).cloned() }
    fn set(&self, account: &str, password: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(account.into(), password.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.remove(account);
        Ok(())
    }
}
