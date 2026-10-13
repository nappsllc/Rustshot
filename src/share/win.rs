//! Windows: the share sheet through hand-rolled WinRT vtables (as
//! [`crate::ocr`]'s; the `windows` crate supplies only `RoInitialize`,
//! `RoGetActivationFactory`, `HSTRING`, the GUIDs' type and Win32).
//!
//! On its own single-threaded apartment thread: a `StorageFile` from
//! `StorageFile.GetFileFromPathAsync` (and, for a picture, a
//! `RandomAccessStreamReference` of it for the bitmap and the thumbnail),
//! an invisible owner window, `IDataTransferManagerInterop::GetForWindow`
//! with a `DataRequested` handler filling the package, then
//! `ShowShareUIForWindow`. The thread pumps the window's messages until the
//! package says the share completed (`ShareCompleted`) or was cancelled
//! (`ShareCanceled`, Windows 10 2004 and later), or `max` passes.
//!
//! The handlers (`TypedEventHandler<..>` delegates) and the one-item
//! `IIterable<IStorageItem>` for `SetStorageItems` are small COM objects
//! made here: a vtable, a reference count, agile (no marshalling). Their
//! interface ids are the WinRT parameterized ids of their signatures
//! (tested below). Vtable slots: IUnknown 0-2, IInspectable 3-5, then the
//! interface's methods from 6 (delegates: Invoke is 3).

use super::{Kind, TITLE};
use core::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{GUID, HRESULT, HSTRING, IInspectable, Interface};

// --- interface ids ---------------------------------------------------------------

const IID_UNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_INSPECTABLE: GUID = GUID::from_u128(0xaf86e2e0_b12d_4c6a_9c5a_d7aa65101e90);
const IID_AGILE: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
const IID_ASYNC_INFO: GUID = GUID::from_u128(0x00000036_0000_0000_c000_000000000046);
const IID_INTEROP: GUID = GUID::from_u128(0x3a3dcd6c_3eab_43dc_bcde_45671ce800c8);
const IID_DATA_TRANSFER_MANAGER: GUID = GUID::from_u128(0xa5caee9b_8708_49d1_8d36_67d25a8da00c);
const IID_DATA_PACKAGE3: GUID = GUID::from_u128(0x88f31f5d_787b_4d32_965a_a9838105a056);
const IID_DATA_PACKAGE4: GUID = GUID::from_u128(0x13a24ec8_9382_536f_852a_3045e1b29a3b);
const IID_STORAGE_FILE_STATICS: GUID = GUID::from_u128(0x5984c710_daf2_43c8_8bb4_a4d3eacfd03f);
const IID_STORAGE_ITEM: GUID = GUID::from_u128(0x4207a996_ca2f_42f7_bde8_8b10457a7f30);
const IID_STREAM_REF_STATICS: GUID = GUID::from_u128(0x857309dc_3fbf_4e7d_986f_ef3b1a07a964);

/// `TypedEventHandler<DataTransferManager, DataRequestedEventArgs>`.
const IID_H_DATA_REQUESTED: GUID = GUID::from_u128(0xec6f9cc8_46d0_5e0e_b4d2_7d7773ae37a0);
/// `TypedEventHandler<DataTransferManager, TargetApplicationChosenEventArgs>`.
const IID_H_TARGET_CHOSEN: GUID = GUID::from_u128(0xc4ac1ba2_7851_5a44_bc8d_3d7c713f1f41);
/// `TypedEventHandler<DataPackage, ShareCompletedEventArgs>`.
const IID_H_SHARE_COMPLETED: GUID = GUID::from_u128(0xf8f7e24a_56fe_58df_bc15_2365aec03966);
/// `TypedEventHandler<DataPackage, Object>` (ShareCanceled).
const IID_H_SHARE_CANCELED: GUID = GUID::from_u128(0xc156b0c3_1cbc_5ca4_901c_62c5a8ca5cb5);
/// `IIterable<IStorageItem>`, `IIterator<IStorageItem>`.
const IID_ITERABLE_ITEM: GUID = GUID::from_u128(0xbb8b8418_65d1_544b_b083_6d172f568c73);
const IID_ITERATOR_ITEM: GUID = GUID::from_u128(0x05b487c2_3830_5d3c_98da_25fa11542dbd);

const S_OK: HRESULT = HRESULT(0);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002_u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003_u32 as i32);
const E_BOUNDS: HRESULT = HRESULT(0x8000000B_u32 as i32);

/// `AsyncStatus`.
const STARTED: i32 = 0;
const COMPLETED: i32 = 1;

// --- interface pointers ----------------------------------------------------------

/// An owned COM interface pointer, released on drop.
struct Com(*mut c_void);

// The objects used here are agile (WinRT runtime classes, ours).
unsafe impl Send for Com {}
unsafe impl Sync for Com {}

type R<T> = Result<T, String>;

fn check(hr: HRESULT, what: &str) -> R<()> {
    if hr.is_ok() { Ok(()) } else { Err(format!("{what}: {}", hr.message().trim())) }
}

impl Com {
    fn new(p: *mut c_void) -> Option<Com> {
        (!p.is_null()).then_some(Com(p))
    }

    /// Another reference to `p` (AddRef).
    fn borrowed(p: *mut c_void) -> Option<Com> {
        if p.is_null() {
            return None;
        }
        unsafe { add_ref(p) };
        Some(Com(p))
    }

    /// Method `slot` of the vtable, as `F`.
    unsafe fn slot<F: Copy>(&self, slot: usize) -> F {
        debug_assert_eq!(size_of::<F>(), size_of::<usize>());
        unsafe {
            let vt = *(self.0 as *const *const usize);
            core::mem::transmute_copy(&*vt.add(slot))
        }
    }

    fn query(&self, iid: &GUID, what: &str) -> R<Com> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = self.slot(0);
            f(self.0, iid, &mut out)
        };
        check(hr, what)?;
        Com::new(out).ok_or_else(|| format!("{what}: nothing"))
    }

    /// A getter returning an interface (`slot`, no arguments).
    fn get_obj(&self, slot: usize, what: &str) -> R<Com> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Com::new(out).ok_or_else(|| format!("{what}: nothing"))
    }

    /// A getter returning a value (`slot`, no arguments).
    fn get<T: Copy + Default>(&self, slot: usize, what: &str) -> R<T> {
        let mut out = T::default();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut T) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Ok(out)
    }

    /// A method taking one pointer (an object or an `HSTRING`).
    fn call_ptr(&self, slot: usize, arg: *mut c_void, what: &str) -> R<()> {
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT = self.slot(slot);
            f(self.0, arg)
        };
        check(hr, what)
    }

    /// A method taking one pointer and returning an object.
    fn call_ptr_obj(&self, slot: usize, arg: *mut c_void, what: &str) -> R<Com> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT = self.slot(slot);
            f(self.0, arg, &mut out)
        };
        check(hr, what)?;
        Com::new(out).ok_or_else(|| format!("{what}: nothing"))
    }

    /// `add_X(handler, &token)` at `slot`; the token.
    fn add_event(&self, slot: usize, handler: &Com, what: &str) -> R<i64> {
        let mut token = 0i64;
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT = self.slot(slot);
            f(self.0, handler.0, &mut token)
        };
        check(hr, what)?;
        Ok(token)
    }

    /// `remove_X(token)` at `slot`.
    fn remove_event(&self, slot: usize, token: i64) {
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT = self.slot(slot);
            let _ = f(self.0, token);
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { release(self.0) };
    }
}

unsafe fn add_ref(p: *mut c_void) -> u32 {
    unsafe {
        let vt = *(p as *const *const usize);
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(1));
        f(p)
    }
}

unsafe fn release(p: *mut c_void) -> u32 {
    unsafe {
        let vt = *(p as *const *const usize);
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        f(p)
    }
}

/// An `HSTRING` argument: its handle (borrowed for the call).
fn hs(h: &HSTRING) -> *mut c_void {
    // SAFETY: an HSTRING is one handle-sized pointer (null when empty).
    unsafe { *(h as *const HSTRING as *const *mut c_void) }
}

/// The activation factory of runtime class `class`, as interface `iid`.
fn factory(class: &str, iid: &GUID) -> R<Com> {
    use windows::Win32::System::WinRT::RoGetActivationFactory;
    let f: IInspectable = unsafe { RoGetActivationFactory(&HSTRING::from(class)) }.map_err(|e| format!("{class}: {}", e.message().trim()))?;
    let borrowed = core::mem::ManuallyDrop::new(Com(f.as_raw()));
    borrowed.query(iid, class)
}

/// Wait for an `IAsyncOperation<T>` (at most `max`) and take its result.
fn wait(op: &Com, max: Duration, what: &str) -> R<Com> {
    let info = op.query(&IID_ASYNC_INFO, what)?;
    let t0 = Instant::now();
    loop {
        let st: i32 = info.get(7, what)?;
        match st {
            STARTED if t0.elapsed() > max => return Err(format!("{what}: timed out")),
            STARTED => std::thread::sleep(Duration::from_millis(5)),
            COMPLETED => break,
            _ => {
                let hr: HRESULT = info.get(8, what)?;
                return Err(format!("{what}: {}", hr.message().trim()));
            }
        }
    }
    op.get_obj(8, what)
}

// --- our COM objects -------------------------------------------------------------

/// `QueryInterface` of our objects: IUnknown, IAgileObject and `own`.
unsafe fn qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void, own: &[GUID]) -> HRESULT {
    unsafe {
        if out.is_null() || riid.is_null() {
            return E_POINTER;
        }
        let iid = *riid;
        if iid == IID_UNKNOWN || iid == IID_AGILE || own.contains(&iid) {
            add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

type Invoke = dyn Fn(*mut c_void, *mut c_void) -> HRESULT + Send + Sync;

/// A `TypedEventHandler<S, A>` delegate calling a closure.
#[repr(C)]
struct Delegate {
    vt: &'static DelegateVtbl,
    refs: AtomicU32,
    iid: GUID,
    f: Box<Invoke>,
}

#[repr(C)]
struct DelegateVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
}

static DELEGATE_VT: DelegateVtbl = DelegateVtbl { query: d_query, add_ref: d_add_ref, release: d_release, invoke: d_invoke };

unsafe extern "system" fn d_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        let iid = (*(this as *const Delegate)).iid;
        qi(this, riid, out, &[iid])
    }
}

unsafe extern "system" fn d_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Delegate)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn d_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Delegate)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            drop(Box::from_raw(this as *mut Delegate));
        }
        n
    }
}

unsafe extern "system" fn d_invoke(this: *mut c_void, sender: *mut c_void, args: *mut c_void) -> HRESULT {
    unsafe { ((*(this as *const Delegate)).f)(sender, args) }
}

/// A new delegate for interface `iid` calling `f(sender, args)`.
fn delegate(iid: GUID, f: impl Fn(*mut c_void, *mut c_void) -> HRESULT + Send + Sync + 'static) -> Com {
    let d = Box::new(Delegate { vt: &DELEGATE_VT, refs: AtomicU32::new(1), iid, f: Box::new(f) });
    Com(Box::into_raw(d) as *mut c_void)
}

/// A one-item `IIterable<IStorageItem>` (and its iterators).
#[repr(C)]
struct Items {
    vt: &'static IterableVtbl,
    refs: AtomicU32,
    item: Com,
}

#[repr(C)]
struct IterableVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    class_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    trust: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    first: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

static ITERABLE_VT: IterableVtbl = IterableVtbl {
    query: it_query,
    add_ref: it_add_ref,
    release: it_release,
    iids: no_iids,
    class_name: no_name,
    trust: base_trust,
    first: it_first,
};

/// An iterator over [`Items`]: at the item (`at` 0) or past it.
#[repr(C)]
struct Iter {
    vt: &'static IteratorVtbl,
    refs: AtomicU32,
    item: Com,
    at: AtomicU32,
}

#[repr(C)]
struct IteratorVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    class_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    trust: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    current: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    has_current: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    move_next: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    get_many: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> HRESULT,
}

static ITERATOR_VT: IteratorVtbl = IteratorVtbl {
    query: iter_query,
    add_ref: iter_add_ref,
    release: iter_release,
    iids: no_iids,
    class_name: no_name,
    trust: base_trust,
    current: iter_current,
    has_current: iter_has_current,
    move_next: iter_move_next,
    get_many: iter_get_many,
};

unsafe extern "system" fn no_iids(_this: *mut c_void, n: *mut u32, out: *mut *mut GUID) -> HRESULT {
    unsafe {
        if n.is_null() || out.is_null() {
            return E_POINTER;
        }
        *n = 0;
        *out = core::ptr::null_mut();
    }
    S_OK
}

unsafe extern "system" fn no_name(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = core::ptr::null_mut() };
    S_OK
}

unsafe extern "system" fn base_trust(_this: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = 0 };
    S_OK
}

unsafe extern "system" fn it_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe { qi(this, riid, out, &[IID_INSPECTABLE, IID_ITERABLE_ITEM]) }
}

unsafe extern "system" fn it_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Items)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn it_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Items)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            drop(Box::from_raw(this as *mut Items));
        }
        n
    }
}

unsafe extern "system" fn it_first(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        let item = Com::borrowed((*(this as *const Items)).item.0).expect("an item");
        let it = Box::new(Iter { vt: &ITERATOR_VT, refs: AtomicU32::new(1), item, at: AtomicU32::new(0) });
        *out = Box::into_raw(it) as *mut c_void;
    }
    S_OK
}

unsafe extern "system" fn iter_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe { qi(this, riid, out, &[IID_INSPECTABLE, IID_ITERATOR_ITEM]) }
}

unsafe extern "system" fn iter_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Iter)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn iter_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Iter)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            drop(Box::from_raw(this as *mut Iter));
        }
        n
    }
}

unsafe extern "system" fn iter_current(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        let it = &*(this as *const Iter);
        if it.at.load(Ordering::SeqCst) == 0 {
            add_ref(it.item.0);
            *out = it.item.0;
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_BOUNDS
        }
    }
}

unsafe extern "system" fn iter_has_current(this: *mut c_void, out: *mut u8) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = ((*(this as *const Iter)).at.load(Ordering::SeqCst) == 0) as u8 };
    S_OK
}

unsafe extern "system" fn iter_move_next(this: *mut c_void, out: *mut u8) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        (*(this as *const Iter)).at.store(1, Ordering::SeqCst);
        *out = 0;
    }
    S_OK
}

unsafe extern "system" fn iter_get_many(this: *mut c_void, cap: u32, items: *mut *mut c_void, got: *mut u32) -> HRESULT {
    if got.is_null() || (cap > 0 && items.is_null()) {
        return E_POINTER;
    }
    unsafe {
        let it = &*(this as *const Iter);
        if cap > 0 && it.at.swap(1, Ordering::SeqCst) == 0 {
            add_ref(it.item.0);
            *items = it.item.0;
            *got = 1;
        } else {
            *got = 0;
        }
    }
    S_OK
}

/// An `IIterable<IStorageItem>` of `item` alone.
fn one_item(item: Com) -> Com {
    let b = Box::new(Items { vt: &ITERABLE_VT, refs: AtomicU32::new(1), item });
    Com(Box::into_raw(b) as *mut c_void)
}

// --- the owner window ------------------------------------------------------------

mod owner {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect, IsWindow, LWA_ALPHA, RegisterClassExW, SW_SHOWNA, SetForegroundWindow, SetLayeredWindowAttributes,
        ShowWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(h, m, w, l) }
    }

    /// Where the sheet goes: over the window that asked, else the work
    /// area of the pointer's monitor.
    fn place(asker: Option<usize>) -> RECT {
        unsafe {
            if let Some(h) = asker.filter(|h| *h != 0).map(|h| HWND(h as *mut core::ffi::c_void))
                && IsWindow(Some(h)).as_bool()
            {
                let mut r = RECT::default();
                if GetWindowRect(h, &mut r).is_ok() && r.right > r.left && r.bottom > r.top {
                    return r;
                }
            }
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                return mi.rcWork;
            }
            RECT { left: p.x - 400, top: p.y - 300, right: p.x + 400, bottom: p.y + 300 }
        }
    }

    /// An invisible, click-through window over `asker` (or the pointer's
    /// monitor) owning the sheet; destroyed on drop.
    pub struct Owner(pub HWND);

    impl Owner {
        pub fn new(asker: Option<usize>) -> Result<Owner, String> {
            unsafe {
                let inst = GetModuleHandleW(None).map_err(|e| e.message())?;
                let class = w!("RustshotShareOwner");
                let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
                // A second share finds the class registered: fine.
                let _ = RegisterClassExW(&wc);
                let r = place(asker);
                let h = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                    class,
                    w!("Rustshot"),
                    WS_POPUP,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    None,
                    None,
                    Some(inst.into()),
                    None,
                )
                .map_err(|e| format!("owner window: {}", e.message()))?;
                // Fully transparent: shown (the sheet wants a real window),
                // never seen, never hit.
                let _ = SetLayeredWindowAttributes(h, windows::Win32::Foundation::COLORREF(0), 0, LWA_ALPHA);
                let _ = ShowWindow(h, SW_SHOWNA);
                let _ = SetForegroundWindow(h);
                Ok(Owner(h))
            }
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

/// How long the sheet may take to ask for the package. It never asks
/// when another sheet is already up (that one stays; it outlives its
/// owner window): then there is nothing to wait for.
pub const ASK_WAIT: Duration = Duration::from_secs(5);

/// Whether the pump goes on at `now`: not `done`, before `until`, and
/// asked for the package (`requested`) or still within [`ASK_WAIT`] of
/// `t0`.
pub fn keep_pumping(done: bool, requested: bool, t0: Instant, now: Instant, until: Instant) -> bool {
    !done && now < until && (requested || now < t0 + ASK_WAIT)
}

/// Pump this thread's messages while [`keep_pumping`].
fn pump(done: &AtomicBool, requested: &AtomicBool, until: Instant) {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage};
    let mut msg = MSG::default();
    let t0 = Instant::now();
    while keep_pumping(done.load(Ordering::SeqCst), requested.load(Ordering::SeqCst), t0, Instant::now(), until) {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
        }
    }
}

/// The single-threaded apartment of this (share) thread, left on drop.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Apartment {
        use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize};
        Apartment(unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::WinRT::RoUninitialize() };
        }
    }
}

/// What the `DataRequested` handler puts in the package.
struct Content {
    item: Com,
    /// The picture's stream (bitmap and thumbnail); none for a recording.
    bitmap: Option<Com>,
    description: HSTRING,
}

/// Fill `request`'s package from `c`; `done` is set when the share ends
/// (the package says so).
fn fill(args: *mut c_void, c: &Content, done: &Arc<AtomicBool>) -> R<()> {
    let args = Com::borrowed(args).ok_or("no request")?;
    let request = args.get_obj(6, "Request")?; // IDataRequestedEventArgs::Request
    let pkg = request.get_obj(6, "Data")?; // IDataRequest::Data
    let props = pkg.get_obj(7, "Properties")?; // IDataPackage::Properties
    let title = HSTRING::from(TITLE);
    props.call_ptr(7, hs(&title), "SetTitle")?;
    props.call_ptr(9, hs(&c.description), "SetDescription")?;
    if let Some(b) = &c.bitmap {
        let _ = props.call_ptr(11, b.0, "SetThumbnail");
        pkg.call_ptr(21, b.0, "SetBitmap")?;
    }
    let items = one_item(Com::borrowed(c.item.0).ok_or("no item")?);
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, bool) -> HRESULT = pkg.slot(23);
        f(pkg.0, items.0, true)
    };
    check(hr, "SetStorageItems")?;
    // The end of the share: completed (IDataPackage3) or cancelled
    // (IDataPackage4). The handlers stay with the package.
    let end = |iid: GUID| {
        let d = done.clone();
        delegate(iid, move |_, _| {
            d.store(true, Ordering::SeqCst);
            S_OK
        })
    };
    if let Ok(p3) = pkg.query(&IID_DATA_PACKAGE3, "IDataPackage3") {
        let _ = p3.add_event(6, &end(IID_H_SHARE_COMPLETED), "ShareCompleted");
    }
    if let Ok(p4) = pkg.query(&IID_DATA_PACKAGE4, "IDataPackage4") {
        let _ = p4.add_event(6, &end(IID_H_SHARE_CANCELED), "ShareCanceled");
    }
    Ok(())
}

/// With `RUSTSHOT_SHARE_TRACE` set: say how the share goes (live tests).
fn trace(what: &str) {
    if std::env::var_os("RUSTSHOT_SHARE_TRACE").is_some() {
        eprintln!("rustshot: share: {what}");
    }
}

/// Share `file` through a sheet over `asker` (a raw window handle) and
/// wait until it closes (at most `max`).
pub fn share(file: &Path, asker: Option<usize>, kind: Kind, max: Duration) -> Result<(), String> {
    let _apt = Apartment::enter();
    let path = HSTRING::from(file.as_os_str());
    let statics = factory("Windows.Storage.StorageFile", &IID_STORAGE_FILE_STATICS)?;
    let op = statics.call_ptr_obj(6, hs(&path), "GetFileFromPathAsync")?;
    let sfile = wait(&op, Duration::from_secs(10), "GetFileFromPathAsync")?;
    let item = sfile.query(&IID_STORAGE_ITEM, "IStorageItem")?;
    let bitmap = match kind {
        Kind::Picture => {
            let refs = factory("Windows.Storage.Streams.RandomAccessStreamReference", &IID_STREAM_REF_STATICS)?;
            Some(refs.call_ptr_obj(6, sfile.0, "CreateFromFile")?)
        }
        Kind::File => None,
    };
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let content = Arc::new(Content { item, bitmap, description: HSTRING::from(name) });

    let owner = owner::Owner::new(asker)?;
    let hwnd = owner.0;
    let interop = factory("Windows.ApplicationModel.DataTransfer.DataTransferManager", &IID_INTEROP)?;
    // IDataTransferManagerInterop::GetForWindow (3).
    let mut dtm = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = interop.slot(3);
        f(interop.0, hwnd.0, &IID_DATA_TRANSFER_MANAGER, &mut dtm)
    };
    check(hr, "GetForWindow")?;
    let dtm = Com::new(dtm).ok_or("GetForWindow: nothing")?;

    let done = Arc::new(AtomicBool::new(false));
    let failed: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let requested = Arc::new(AtomicBool::new(false));
    let on_request = {
        let (c, d, failed, asked) = (content.clone(), done.clone(), failed.clone(), requested.clone());
        delegate(IID_H_DATA_REQUESTED, move |_, args| {
            asked.store(true, Ordering::SeqCst);
            match fill(args, &c, &d) {
                Ok(()) => trace("package filled"),
                Err(e) => {
                    trace(&format!("package failed: {e}"));
                    *failed.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
                    d.store(true, Ordering::SeqCst);
                }
            }
            S_OK
        })
    };
    let chosen = Arc::new(AtomicBool::new(false));
    let on_chosen = {
        let ch = chosen.clone();
        delegate(IID_H_TARGET_CHOSEN, move |_, _| {
            ch.store(true, Ordering::SeqCst);
            S_OK
        })
    };
    let t_request = dtm.add_event(6, &on_request, "DataRequested")?;
    let t_chosen = dtm.add_event(8, &on_chosen, "TargetApplicationChosen").ok();
    // IDataTransferManagerInterop::ShowShareUIForWindow (4).
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT = interop.slot(4);
        f(interop.0, hwnd.0)
    };
    let shown = check(hr, "ShowShareUIForWindow");
    if shown.is_ok() {
        trace(&format!("sheet shown for {}", file.display()));
        pump(&done, &requested, Instant::now() + max);
    }
    dtm.remove_event(7, t_request);
    if let Some(t) = t_chosen {
        dtm.remove_event(9, t);
    }
    trace(&format!(
        "ended (asked: {}, done: {}, target chosen: {})",
        requested.load(Ordering::SeqCst),
        done.load(Ordering::SeqCst),
        chosen.load(Ordering::SeqCst)
    ));
    drop(owner);
    shown?;
    match failed.lock().unwrap_or_else(|p| p.into_inner()).take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-1 (FIPS 180-4), for the interface ids below.
    fn sha1(data: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
        let mut m = data.to_vec();
        let bits = (data.len() as u64) * 8;
        m.push(0x80);
        while m.len() % 64 != 56 {
            m.push(0);
        }
        m.extend_from_slice(&bits.to_be_bytes());
        for block in m.as_chunks::<64>().0 {
            let mut w = [0u32; 80];
            for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
                w[i] = u32::from_be_bytes(*c);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = h;
            for (i, wi) in w.iter().enumerate() {
                let (f, k) = match i {
                    0..=19 => ((b & c) | (!b & d), 0x5A827999),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                    _ => (b ^ c ^ d, 0xCA62C1D6),
                };
                let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
                (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
            }
            for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
                *x = x.wrapping_add(y);
            }
        }
        let mut out = [0u8; 20];
        for (o, x) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
            *o = x.to_be_bytes();
        }
        out
    }

    /// The WinRT id of a parameterized interface: a name-based (v5) UUID
    /// of its signature in the WinRT namespace.
    fn pinterface(sig: &str) -> GUID {
        let mut data = 0x11f47ad5_7b73_42c0_abae_878b1e16adee_u128.to_be_bytes().to_vec();
        data.extend_from_slice(sig.as_bytes());
        let mut b: [u8; 16] = sha1(&data)[..16].try_into().unwrap();
        b[6] = (b[6] & 0x0f) | 0x50;
        b[8] = (b[8] & 0x3f) | 0x80;
        GUID::from_u128(u128::from_be_bytes(b))
    }

    /// The pump stops when the share ends, at the time limit, or when the
    /// sheet never asks for the package (another one was up).
    #[test]
    fn the_pump_stops_in_time() {
        let t0 = Instant::now();
        let until = t0 + Duration::from_secs(600);
        assert!(keep_pumping(false, false, t0, t0 + Duration::from_secs(1), until));
        assert!(!keep_pumping(false, false, t0, t0 + ASK_WAIT, until), "never asked");
        assert!(keep_pumping(false, true, t0, t0 + Duration::from_secs(60), until), "asked: until it ends");
        assert!(!keep_pumping(true, true, t0, t0 + Duration::from_secs(2), until), "ended");
        assert!(!keep_pumping(false, true, t0, until, until), "time limit");
    }

    #[test]
    fn sha1_known_answers() {
        let hex = |b: [u8; 20]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    /// The handlers' and the item list's ids are those of their signatures.
    #[test]
    fn parameterized_ids() {
        const TEH: &str = "{9de1c534-6ae1-11e0-84e1-18a905bcc53f}";
        let rc = |n: &str, i: &str| format!("rc(Windows.ApplicationModel.DataTransfer.{n};{{{i}}})");
        let dtm = rc("DataTransferManager", "a5caee9b-8708-49d1-8d36-67d25a8da00c");
        let dp = rc("DataPackage", "61ebf5c7-efea-4346-9554-981d7e198ffe");
        let h = |s: &str, a: &str| pinterface(&format!("pinterface({TEH};{s};{a})"));
        assert_eq!(h(&dtm, &rc("DataRequestedEventArgs", "cb8ba807-6ac5-43c9-8ac5-9ba232163182")), IID_H_DATA_REQUESTED);
        assert_eq!(h(&dtm, &rc("TargetApplicationChosenEventArgs", "ca6fb8ac-2987-4ee3-9c54-d8afbcb86c1d")), IID_H_TARGET_CHOSEN);
        assert_eq!(h(&dp, &rc("ShareCompletedEventArgs", "4574c442-f913-4f60-9df7-cc4060ab1916")), IID_H_SHARE_COMPLETED);
        assert_eq!(h(&dp, "cinterface(IInspectable)"), IID_H_SHARE_CANCELED);
        assert_eq!(pinterface("pinterface({faa585ea-6214-4217-afda-7f46de5869b3};{4207a996-ca2f-42f7-bde8-8b10457a7f30})"), IID_ITERABLE_ITEM);
        assert_eq!(pinterface("pinterface({6a79e863-4300-459a-9966-cbb660963ee1};{4207a996-ca2f-42f7-bde8-8b10457a7f30})"), IID_ITERATOR_ITEM);
    }

    /// A stand-in COM object counting its references (for the item list).
    #[repr(C)]
    struct Counted {
        vt: &'static CountedVt,
        refs: AtomicU32,
    }

    #[repr(C)]
    struct CountedVt {
        query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    unsafe extern "system" fn c_query(_: *mut c_void, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
        E_NOINTERFACE
    }
    unsafe extern "system" fn c_add(this: *mut c_void) -> u32 {
        unsafe { (*(this as *const Counted)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
    }
    unsafe extern "system" fn c_rel(this: *mut c_void) -> u32 {
        unsafe { (*(this as *const Counted)).refs.fetch_sub(1, Ordering::SeqCst) - 1 }
    }

    /// The one-item list iterates once, hands out counted references and
    /// answers only its interfaces; every reference comes back.
    #[test]
    fn one_item_list_iterates_once() {
        static VT: CountedVt = CountedVt { query: c_query, add_ref: c_add, release: c_rel };
        let obj = Box::new(Counted { vt: &VT, refs: AtomicU32::new(1) });
        let p = &*obj as *const Counted as *mut c_void;
        {
            let list = one_item(Com::borrowed(p).unwrap());
            assert_eq!(obj.refs.load(Ordering::SeqCst), 2);
            assert!(list.query(&IID_ITERABLE_ITEM, "q").is_ok());
            assert!(list.query(&IID_AGILE, "q").is_ok());
            assert!(list.query(&IID_ITERATOR_ITEM, "q").is_err());
            let it = list.get_obj(6, "First").unwrap();
            assert!(it.query(&IID_ITERATOR_ITEM, "q").is_ok());
            assert_eq!(it.get::<u8>(7, "HasCurrent").unwrap(), 1);
            let cur = it.get_obj(6, "Current").unwrap();
            assert_eq!(cur.0, p);
            drop(cur);
            assert_eq!(it.get::<u8>(8, "MoveNext").unwrap(), 0);
            assert_eq!(it.get::<u8>(7, "HasCurrent").unwrap(), 0);
            assert!(it.get_obj(6, "Current").is_err(), "past the end");
            // GetMany from a fresh iterator: the item, then nothing.
            let it2 = list.get_obj(6, "First").unwrap();
            let get_many = |it: &Com| {
                let (mut out, mut n) = ([core::ptr::null_mut::<c_void>(); 2], 0u32);
                let hr = unsafe {
                    let f: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> HRESULT = it.slot(9);
                    f(it.0, 2, out.as_mut_ptr(), &mut n)
                };
                assert!(hr.is_ok());
                for q in out.iter().take(n as usize) {
                    unsafe { release(*q) };
                }
                n
            };
            assert_eq!(get_many(&it2), 1);
            assert_eq!(get_many(&it2), 0);
        }
        assert_eq!(obj.refs.load(Ordering::SeqCst), 1, "all references back");
    }

    /// A delegate answers its own id (and IUnknown, IAgileObject), calls
    /// its closure, and frees it with the last reference.
    #[test]
    fn delegates_call_and_free() {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let d = delegate(IID_H_SHARE_CANCELED, move |s, a| {
            // (No assert here: a panic cannot unwind out of a COM call.)
            h.fetch_add(if s.is_null() && a.is_null() { 1 } else { 100 }, Ordering::SeqCst);
            S_OK
        });
        assert!(d.query(&IID_H_SHARE_CANCELED, "q").is_ok());
        assert!(d.query(&IID_UNKNOWN, "q").is_ok());
        assert!(d.query(&IID_H_DATA_REQUESTED, "q").is_err());
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT = d.slot(3);
            f(d.0, core::ptr::null_mut(), core::ptr::null_mut())
        };
        assert!(hr.is_ok());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(Arc::strong_count(&hits), 2);
        drop(d);
        assert_eq!(Arc::strong_count(&hits), 1, "the closure went with the delegate");
    }

    /// Live: the sheet comes up for a PNG (`RUSTSHOT_SHARE_TRACE=1` says
    /// "package filled") and the share thread lets go after
    /// `RUSTSHOT_SHARE_LIVE_SECS` (4) with nothing shared. The sheet itself
    /// stays until dismissed by hand (Esc or a click elsewhere). Run by
    /// hand: `cargo test share::win -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_sheet_opens_and_closes() {
        let dir = std::env::temp_dir().join(format!("rustshot-share-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("live.png");
        std::fs::write(&f, crate::pixbuf::PixBuf::from_pixel(64, 48, [30, 120, 220, 255]).to_png().unwrap()).unwrap();
        let t0 = Instant::now();
        let secs = std::env::var("RUSTSHOT_SHARE_LIVE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
        let r = share(&f, None, Kind::Picture, Duration::from_secs(secs));
        assert_eq!(r, Ok(()));
        // Up until the limit, or 5 s when another sheet was already up
        // (the sheet outlives its owner: dismiss it by hand, Esc).
        assert!(t0.elapsed() >= Duration::from_secs(secs.min(ASK_WAIT.as_secs()).saturating_sub(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
