//! `dlopen` for the optional recording libraries: OpenH264 (downloaded,
//! opened by its absolute path only), libopus and libpulse-simple (the
//! system's, by soname). Nothing is linked at build time, so a missing
//! library is an error at the time of use, not a failure to start.

use anyhow::{anyhow, Result};
use core::ffi::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *mut c_char;
}

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

/// An open library; closed on drop (after everything taken from it).
pub struct Lib {
    h: *mut c_void,
    name: String,
}

// SAFETY: a dlopen handle may be used from any thread (dlsym/dlclose are
// thread-safe); the functions taken from it are used by one owner at a time.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

fn last_error() -> String {
    // SAFETY: dlerror returns a thread-local message or null.
    let e = unsafe { dlerror() };
    if e.is_null() { "unknown error".into() } else { unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned() }
}

impl Lib {
    /// Open `name` (an absolute path, or a soname looked up the system's
    /// way) with every symbol bound now and nothing exported to others.
    pub fn open(name: &str) -> Result<Lib> {
        let c = CString::new(name).map_err(|_| anyhow!("bad library name"))?;
        // SAFETY: a valid C string; the library's initialisers run here.
        let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if h.is_null() {
            return Err(anyhow!("cannot load {name}: {}", last_error()));
        }
        Ok(Lib { h, name: name.into() })
    }

    /// The address of `sym` as a function pointer of type `F`.
    ///
    /// # Safety
    /// `F` must be the symbol's real (C ABI) function type.
    pub unsafe fn sym<F: Copy>(&self, sym: &str) -> Result<F> {
        assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>(), "a function pointer");
        let c = CString::new(sym).map_err(|_| anyhow!("bad symbol name"))?;
        // SAFETY: a live handle and a valid C string.
        let p = unsafe { dlsym(self.h, c.as_ptr()) };
        if p.is_null() {
            return Err(anyhow!("{} has no {sym}", self.name));
        }
        // SAFETY: same size (asserted); the caller vouches for the type.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        // SAFETY: opened by us; the owner drops what it took from it first.
        unsafe {
            dlclose(self.h);
        }
    }
}
