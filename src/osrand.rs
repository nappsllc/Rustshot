//! Random bytes from the OS CSPRNG: `BCryptGenRandom` (Windows),
//! `getrandom(2)` (Linux), `SecRandomCopyBytes` (macOS). Used for S3
//! object keys (a public link's only protection) and retry jitter.

/// Fill `buf` with random bytes. The OS generators do not fail in
/// practice; if one ever does, this panics rather than hand out
/// predictable bytes.
pub fn fill(buf: &mut [u8]) {
    imp(buf).expect("the OS random generator failed");
}

#[cfg(windows)]
fn imp(buf: &mut [u8]) -> Result<(), String> {
    use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    let st = unsafe { BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if st.0 >= 0 { Ok(()) } else { Err(format!("BCryptGenRandom: {:#x}", st.0)) }
}

#[cfg(target_os = "linux")]
fn imp(buf: &mut [u8]) -> Result<(), String> {
    unsafe extern "C" {
        fn getrandom(buf: *mut u8, len: usize, flags: u32) -> isize;
    }
    let mut done = 0;
    while done < buf.len() {
        let n = unsafe { getrandom(buf[done..].as_mut_ptr(), buf.len() - done, 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("getrandom: {e}"));
        }
        done += n as usize;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn imp(buf: &mut [u8]) -> Result<(), String> {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecRandomCopyBytes(rnd: *const std::ffi::c_void, count: usize, bytes: *mut u8) -> i32;
    }
    let st = unsafe { SecRandomCopyBytes(std::ptr::null(), buf.len(), buf.as_mut_ptr()) };
    if st == 0 { Ok(()) } else { Err(format!("SecRandomCopyBytes: {st}")) }
}

/// A random u64.
pub fn u64() -> u64 {
    let mut b = [0u8; 8];
    fill(&mut b);
    u64::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bytes_vary() {
        let (mut a, mut b) = ([0u8; 16], [0u8; 16]);
        super::fill(&mut a);
        super::fill(&mut b);
        assert_ne!(a, b);
        assert_ne!(a, [0; 16]);
        let mut empty = [];
        super::fill(&mut empty);
    }
}
