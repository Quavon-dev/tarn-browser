//! Generic credentials of the current user in the Windows Credential Manager, which protects
//! them with DPAPI for that user.

use std::io;

use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn last_error() -> io::Error {
    // SAFETY: reads the calling thread's last-error value.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn missing() -> bool {
    // SAFETY: as above.
    unsafe { GetLastError() == ERROR_NOT_FOUND }
}

pub fn write(target: &str, secret: &[u8]) -> io::Result<()> {
    let mut name = wide(target);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: name.as_mut_ptr(),
        CredentialBlobSize: u32::try_from(secret.len()).map_err(|_| io::Error::other("secret too large"))?,
        CredentialBlob: secret.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` outlives the call, which only reads them.
    if unsafe { CredWriteW(&credential, 0) } == 0 { Err(last_error()) } else { Ok(()) }
}

pub fn read(target: &str) -> io::Result<Option<Vec<u8>>> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; on success `found` points to a block freed below.
    if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) } == 0 {
        return if missing() { Ok(None) } else { Err(last_error()) };
    }
    // SAFETY: `found` is null or the live block CredReadW returned; its blob holds `CredentialBlobSize` bytes.
    let secret = unsafe {
        let Some(c) = found.as_ref() else { return Ok(None) };
        let bytes = if c.CredentialBlob.is_null() { Vec::new() } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
        CredFree(found.cast());
        bytes
    };
    Ok(Some(secret))
}

pub fn delete(target: &str) -> io::Result<()> {
    let name = wide(target);
    // SAFETY: `name` is NUL-terminated.
    if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 && !missing() { Err(last_error()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "touches the user's Credential Manager"]
    fn round_trip() {
        let target = "de.quavon.tarn/test";
        super::write(target, b"secret").unwrap();
        assert_eq!(super::read(target).unwrap().as_deref(), Some(&b"secret"[..]));
        super::delete(target).unwrap();
        assert_eq!(super::read(target).unwrap(), None);
        super::delete(target).unwrap();
    }
}
