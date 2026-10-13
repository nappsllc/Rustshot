//! Secrets in the OS credential store, never in a file of ours: the S3
//! secret access key (`rustshot:s3:<bucket>`).
//!
//! - Windows: Credential Manager, a generic credential (`CredWriteW`,
//!   `CredReadW`, `CredDeleteW`; the secret as UTF-16).
//! - Linux: the Secret Service (`org.freedesktop.secrets`: GNOME Keyring,
//!   KWallet, KeePassXC) over our own D-Bus client, in the default
//!   collection with the attributes `application=rustshot`,
//!   `rustshot-name=<name>`. Without one on the session bus nothing is
//!   saved: [`Store::set`] fails and says why (the secret is never written
//!   to a file instead).
//! - macOS: the login Keychain, a generic password (`SecItemAdd`,
//!   `SecItemCopyMatching`, `SecItemDelete`).
//!
//! Everything goes through [`Store`]; tests use [`Memory`] (and [`os`] is
//! one in tests), so they never touch the user's credentials.

#[cfg(test)]
use std::sync::Mutex;

/// A place secrets are kept.
pub trait Store: Send + Sync {
    /// The secret named `name` (None: there is none).
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    /// Keep `secret` as `name` (replacing one there), `label` the name a
    /// person sees in the store's own app.
    fn set(&self, name: &str, label: &str, secret: &str) -> Result<(), String>;
    /// Forget `name` (fine when it is not there). Rustshot itself only
    /// replaces secrets; removing one is the store app's (and the tests').
    #[cfg_attr(not(test), allow(dead_code))]
    fn delete(&self, name: &str) -> Result<(), String>;
    /// Whether `name` is kept, without reading it (Settings' helper line;
    /// the Secret Service answers without unlocking anything).
    fn has(&self, name: &str) -> Result<bool, String> {
        self.get(name).map(|s| s.is_some_and(|s| !s.is_empty()))
    }
}

/// The name of the S3 secret for `bucket`.
pub fn s3_name(bucket: &str) -> String {
    format!("rustshot:s3:{}", bucket.trim())
}

/// What a person sees for it in the store's app.
pub fn s3_label(bucket: &str) -> String {
    format!("Rustshot S3 secret access key ({})", bucket.trim())
}

/// Where secrets go on this system, for Settings' helper line.
pub fn place() -> &'static str {
    if cfg!(windows) {
        "Windows Credential Manager"
    } else if cfg!(target_os = "macos") {
        "the macOS Keychain"
    } else {
        "the Secret Service keyring"
    }
}

/// An in-memory store (tests; the fake behind [`os`] in tests).
#[cfg(test)]
#[derive(Default)]
pub struct Memory {
    items: Mutex<Vec<(String, String)>>,
}

#[cfg(test)]
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
impl Store for Memory {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(lock(&self.items).iter().find(|(n, _)| n == name).map(|(_, s)| s.clone()))
    }

    fn set(&self, name: &str, _label: &str, secret: &str) -> Result<(), String> {
        let mut v = lock(&self.items);
        v.retain(|(n, _)| n != name);
        v.push((name.to_string(), secret.to_string()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        lock(&self.items).retain(|(n, _)| n != name);
        Ok(())
    }
}

/// The OS store (tests: one shared [`Memory`]).
pub fn os() -> &'static dyn Store {
    #[cfg(test)]
    {
        static MEM: std::sync::OnceLock<Memory> = std::sync::OnceLock::new();
        MEM.get_or_init(Memory::default)
    }
    #[cfg(all(not(test), windows))]
    {
        &win::Credman
    }
    #[cfg(all(not(test), target_os = "linux"))]
    {
        &secret_service::Session
    }
    #[cfg(all(not(test), target_os = "macos"))]
    {
        &macos::Keychain
    }
}

#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
mod win {
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CredDeleteW, CredFree, CredReadW, CredWriteW};
    use windows::core::{PCWSTR, PWSTR};

    pub struct Credman;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    impl super::Store for Credman {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            let target = wide(name);
            let mut p: *mut CREDENTIALW = std::ptr::null_mut();
            unsafe {
                if let Err(e) = CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut p) {
                    return if not_found(&e) { Ok(None) } else { Err(format!("Credential Manager: {}", e.message())) };
                }
                let c = &*p;
                let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
                let units: Vec<u16> = blob.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
                let s = String::from_utf16(&units).map_err(|_| "Credential Manager: the stored secret is not text".to_string());
                CredFree(p as *const _);
                s.map(Some)
            }
        }

        fn set(&self, name: &str, label: &str, secret: &str) -> Result<(), String> {
            let mut target = wide(name);
            let mut comment = wide(label);
            let mut user = wide("rustshot");
            let mut blob: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let cred = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_mut_ptr()),
                Comment: PWSTR(comment.as_mut_ptr()),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR(user.as_mut_ptr()),
                ..Default::default()
            };
            let r = unsafe { CredWriteW(&cred, 0) };
            // The copy in memory goes as soon as it is written.
            blob.iter_mut().for_each(|b| *b = 0);
            r.map_err(|e| format!("Credential Manager: {}", e.message()))
        }

        fn delete(&self, name: &str) -> Result<(), String> {
            let target = wide(name);
            match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Err(e) if !not_found(&e) => Err(format!("Credential Manager: {}", e.message())),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
pub mod secret_service;

#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
mod macos;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_keeps_replaces_and_forgets() {
        let m = Memory::default();
        assert_eq!(m.get("a"), Ok(None));
        m.set("a", "A", "one").unwrap();
        m.set("b", "B", "two").unwrap();
        m.set("a", "A", "three").unwrap();
        assert_eq!(m.get("a"), Ok(Some("three".into())));
        m.delete("a").unwrap();
        m.delete("a").unwrap();
        assert_eq!((m.get("a"), m.get("b")), (Ok(None), Ok(Some("two".into()))));
    }

    #[test]
    fn names_are_per_bucket() {
        assert_eq!(s3_name(" shots "), "rustshot:s3:shots");
        assert!(s3_label("shots").contains("shots"));
    }
}
