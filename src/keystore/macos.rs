//! The login Keychain: a generic password per name (service = the name,
//! account "rustshot"), through Security.framework's `SecItem*` calls.

use std::ffi::c_void;

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type OSStatus = i32;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const NOT_FOUND: OSStatus = -25300; // errSecItemNotFound

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: isize, enc: u32, external: u8) -> CFStringRef;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFTypeRef;
    // Declared as the other modules declare them (one signature per symbol).
    fn CFDataGetLength(d: CFTypeRef) -> i64;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, kcb: *const u8, vcb: *const u8) -> CFDictionaryRef;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CFStringRef;
    static kSecClassGenericPassword: CFStringRef;
    static kSecAttrService: CFStringRef;
    static kSecAttrAccount: CFStringRef;
    static kSecAttrLabel: CFStringRef;
    static kSecValueData: CFStringRef;
    static kSecReturnData: CFStringRef;
    static kSecMatchLimit: CFStringRef;
    static kSecMatchLimitOne: CFStringRef;
    fn SecItemAdd(attrs: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemDelete(query: CFDictionaryRef) -> OSStatus;
}

/// Owned CF objects, released together.
struct Owned(Vec<CFTypeRef>);

impl Owned {
    fn keep(&mut self, r: CFTypeRef) -> CFTypeRef {
        if !r.is_null() {
            self.0.push(r);
        }
        r
    }

    fn string(&mut self, s: &str) -> CFStringRef {
        self.keep(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) })
    }

    fn dict(&mut self, pairs: &[(CFTypeRef, CFTypeRef)]) -> CFDictionaryRef {
        let keys: Vec<CFTypeRef> = pairs.iter().map(|p| p.0).collect();
        let vals: Vec<CFTypeRef> = pairs.iter().map(|p| p.1).collect();
        unsafe {
            let kcb = &raw const kCFTypeDictionaryKeyCallBacks;
            let vcb = &raw const kCFTypeDictionaryValueCallBacks;
            self.keep(CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), vals.as_ptr(), pairs.len() as isize, kcb, vcb))
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        for r in self.0.drain(..).rev() {
            unsafe { CFRelease(r) };
        }
    }
}

fn status(what: &str, s: OSStatus) -> Result<(), String> {
    if s == 0 { Ok(()) } else { Err(format!("Keychain: {what} failed (OSStatus {s})")) }
}

pub struct Keychain;

/// The class, service and account of item `name`.
fn base(o: &mut Owned, name: &str) -> Vec<(CFTypeRef, CFTypeRef)> {
    let (svc, acct) = (o.string(name), o.string("rustshot"));
    unsafe { vec![(kSecClass, kSecClassGenericPassword), (kSecAttrService, svc), (kSecAttrAccount, acct)] }
}

impl super::Store for Keychain {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        let mut o = Owned(Vec::new());
        let mut q = base(&mut o, name);
        unsafe {
            q.push((kSecReturnData, kCFBooleanTrue));
            q.push((kSecMatchLimit, kSecMatchLimitOne));
        }
        let d = o.dict(&q);
        let mut out: CFTypeRef = std::ptr::null();
        let s = unsafe { SecItemCopyMatching(d, &mut out) };
        if s == NOT_FOUND {
            return Ok(None);
        }
        status("reading the secret", s)?;
        o.keep(out);
        let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(out), CFDataGetLength(out).max(0) as usize) };
        String::from_utf8(bytes.to_vec()).map(Some).map_err(|_| "Keychain: the stored secret is not text".into())
    }

    fn set(&self, name: &str, label: &str, secret: &str) -> Result<(), String> {
        self.delete(name)?;
        let mut o = Owned(Vec::new());
        let mut q = base(&mut o, name);
        let lab = o.string(label);
        let data = o.keep(unsafe { CFDataCreate(std::ptr::null(), secret.as_ptr(), secret.len() as isize) });
        unsafe {
            q.push((kSecAttrLabel, lab));
            q.push((kSecValueData, data));
        }
        let d = o.dict(&q);
        status("saving the secret", unsafe { SecItemAdd(d, std::ptr::null_mut()) })
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let mut o = Owned(Vec::new());
        let q = base(&mut o, name);
        let d = o.dict(&q);
        let s = unsafe { SecItemDelete(d) };
        if s == NOT_FOUND { Ok(()) } else { status("deleting the secret", s) }
    }
}
