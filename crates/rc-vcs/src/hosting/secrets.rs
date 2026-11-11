//! Personal access tokens in the OS keychain: the macOS login keychain (Security framework),
//! the Windows Credential Manager, and the Secret Service on Linux (`secret-tool`). Items are
//! stored under the service `DotCraft` with accounts like `github:github.com`.

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain service name of DotCraft's items.
pub const SERVICE: &str = "DotCraft";

/// Where tokens are kept. The app uses [`Keychain`]; tests use [`MemoryStore`].
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// In-memory store (tests, and a fallback when there is no keychain).
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Option<String> {
        self.0.lock().ok()?.get(account).cloned()
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.remove(account);
        Ok(())
    }
}

/// The OS keychain.
#[derive(Clone, Copy, Default, Debug)]
pub struct Keychain;

#[cfg(target_os = "macos")]
impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Option<String> {
        let bytes = security_framework::passwords::get_generic_password(SERVICE, account).ok()?;
        String::from_utf8(bytes).ok()
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, account, secret.as_bytes()).map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            // errSecItemNotFound
            Err(e) if e.code() == -25300 => Ok(()),
            r => r.map_err(|e| e.to_string()),
        }
    }
}

#[cfg(windows)]
mod wincred {
    use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn target(account: &str) -> Vec<u16> {
        wide(&format!("{}:{account}", super::SERVICE))
    }

    pub fn get(account: &str) -> Option<String> {
        let t = target(account);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `t` is NUL-terminated; on success `cred` points to a CREDENTIALW owned by the
        // system until CredFree.
        unsafe {
            if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
                return None;
            }
            let c = &*cred;
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let text = String::from_utf8(blob.to_vec()).ok();
            CredFree(cred as *const _);
            text
        }
    }

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let mut t = target(account);
        let mut user = wide(account);
        let mut blob = secret.as_bytes().to_vec();
        // SAFETY: every pointer refers to a live, NUL-terminated (strings) buffer for the call.
        let ok = unsafe {
            let mut cred = CREDENTIALW::default();
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = t.as_mut_ptr();
            cred.UserName = user.as_mut_ptr();
            cred.CredentialBlobSize = blob.len() as u32;
            cred.CredentialBlob = blob.as_mut_ptr();
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            CredWriteW(&cred, 0)
        };
        if ok == 0 { Err(std::io::Error::last_os_error().to_string()) } else { Ok(()) }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let t = target(account);
        // SAFETY: `t` is NUL-terminated.
        if unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            let e = std::io::Error::last_os_error();
            // ERROR_NOT_FOUND
            if e.raw_os_error() == Some(1168) {
                return Ok(());
            }
            return Err(e.to_string());
        }
        Ok(())
    }
}

#[cfg(windows)]
impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Option<String> {
        wincred::get(account)
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        wincred::set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        wincred::delete(account)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Option<String> {
        let out = std::process::Command::new("secret-tool").args(["lookup", "service", SERVICE, "account", account]).stdin(std::process::Stdio::null()).output().ok()?;
        let s = String::from_utf8(out.stdout).ok()?;
        (out.status.success() && !s.is_empty()).then(|| s.trim_end_matches('\n').to_string())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        use std::io::Write as _;
        let mut child = std::process::Command::new("secret-tool")
            .args(["store", &format!("--label={SERVICE} ({account})"), "service", SERVICE, "account", account])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("secret-tool (libsecret) is needed to store tokens: {e}"))?;
        // The secret goes through stdin, never the command line.
        child.stdin.take().ok_or("no stdin")?.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let out = std::process::Command::new("secret-tool").args(["clear", "service", SERVICE, "account", account]).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    }
}
