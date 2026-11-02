//! The Google refresh token in Windows Credential Manager (a generic credential, DPAPI-protected for this user,
//! not roamed). Never in `settings.json` or the cache; the access token stays in memory only.

use serde::{Deserialize, Serialize};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

const TARGET: PCWSTR = w!("Cay/google");
/// `CRED_MAX_CREDENTIAL_BLOB_SIZE`: 5 × 512 bytes.
const MAX_BLOB: usize = 2560;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub refresh_token: String,
    /// The scopes Google granted, space-separated, as the token response said.
    pub scopes: String,
}

pub fn save(saved: &Saved) -> Result<(), String> {
    let blob = serde_json::to_vec(saved).map_err(|e| e.to_string())?;
    if blob.len() > MAX_BLOB {
        return Err("Google's sign-in is too large for Credential Manager".into());
    }
    let mut user: Vec<u16> = "Cay".encode_utf16().chain([0]).collect();
    let mut blob = blob;
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(TARGET.0.cast_mut()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` points at data that outlives the call; Windows copies it.
    unsafe { CredWriteW(&credential, 0) }.map_err(|e| e.message())
}

pub fn load() -> Option<Saved> {
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: on success Windows allocates the credential; it's read below and released with CredFree.
    unsafe { CredReadW(TARGET, CRED_TYPE_GENERIC, None, &mut found) }.ok()?;
    // SAFETY: `found` is valid until CredFree; the blob is `CredentialBlobSize` bytes.
    let bytes = unsafe {
        let c = &*found;
        let bytes = if c.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        CredFree(found.cast());
        bytes
    };
    match serde_json::from_slice(&bytes) {
        Ok(saved) => Some(saved),
        Err(e) => {
            log::warn!("ignoring an unreadable Google sign-in in Credential Manager: {e}");
            None
        }
    }
}

pub fn delete() {
    // SAFETY: deletes our own generic credential by name; a missing one just fails.
    if let Err(e) = unsafe { CredDeleteW(TARGET, CRED_TYPE_GENERIC, None) } {
        log::debug!("no Google sign-in to delete: {}", e.message());
    }
}
