//! Windows: the share sheet through hand-rolled WinRT vtables (as
//! [`crate::ocr`]'s; the `windows` crate supplies only `RoInitialize`,
//! `RoGetActivationFactory`, `HSTRING`, the GUIDs' type and Win32).
//!
//! On its own single-threaded apartment thread: an invisible owner window
//! (never activated), a `StorageFile` from
//! `StorageFile.GetFileFromPathAsync` (and, for a picture, a
//! `RandomAccessStreamReference` of it for the bitmap and the thumbnail),
//! `IDataTransferManagerInterop::GetForWindow` with a `DataRequested`
//! handler filling the package, then `ShowShareUIForWindow`. The thread
//! pumps the window's messages until the package says the share completed
//! (`ShareCompleted`) or was canceled (`ShareCanceled`, Windows 10 2004 and
//! later). The caller is let go then, or at a time limit ([`Clock`]); the
//! thread stays (with the owner and the package) until the share ends or
//! the process does: the sheet may still ask for the data.
//!
//! The handlers (`TypedEventHandler<..>` delegates) and the one-item
//! `IIterable<IStorageItem>` for `SetStorageItems` are small COM objects
//! made here: a vtable, a reference count, agile, with the free-threaded
//! marshaler aggregated (as windows-rs does) for a caller that marshals.
//! Their interface ids are the WinRT parameterized ids of their signatures
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
const IID_MARSHAL: GUID = GUID::from_u128(0x00000003_0000_0000_c000_000000000046);
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
const CANCELED: i32 = 2;

// --- interface pointers ----------------------------------------------------------

/// An owned COM interface pointer, released on drop.
struct Com(*mut c_void);

/// A [`Com`] of an agile object (a `StorageFile`, a stream reference,
/// ours), which any thread may use.
struct Agile(Com);

// SAFETY: only agile objects are wrapped (see above).
unsafe impl Send for Agile {}
unsafe impl Sync for Agile {}

type R<T> = Result<T, String>;

fn check(hr: HRESULT, what: &str) -> R<()> {
    if hr.is_ok() { Ok(()) } else { Err(format!("{what}: {}", hr.message().trim())) }
}

impl Com {
    fn new(p: *mut c_void) -> Option<Com> {
        (!p.is_null()).then(|| Com(p))
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
    // SAFETY: an HSTRING is repr(transparent) over its handle (null when
    // empty); copied without taking a reference.
    unsafe { core::mem::transmute_copy::<HSTRING, *mut c_void>(h) }
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
    // IAsyncInfo::Cancel (9), Close (10).
    let call = |slot: usize| unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> HRESULT = info.slot(slot);
        let _ = f(info.0);
    };
    let t0 = Instant::now();
    loop {
        let st: i32 = info.get(7, what)?;
        match st {
            STARTED if t0.elapsed() > max => {
                call(9);
                call(10);
                return Err(format!("{what}: timed out"));
            }
            STARTED => std::thread::sleep(Duration::from_millis(5)),
            COMPLETED => break,
            CANCELED => {
                call(10);
                return Err(format!("{what}: canceled"));
            }
            _ => {
                let hr: HRESULT = info.get(8, what)?;
                call(10);
                return Err(format!("{what}: {}", hr.message().trim()));
            }
        }
    }
    let r = op.get_obj(8, what);
    call(10);
    r
}

// --- our COM objects -------------------------------------------------------------

/// `QueryInterface` of our objects: IUnknown, IAgileObject and `own`;
/// IMarshal from the aggregated free-threaded marshaler `ftm` (its inner,
/// non-delegating IUnknown; null when it could not be made).
unsafe fn qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void, own: &[GUID], ftm: *mut c_void) -> HRESULT {
    unsafe {
        if out.is_null() || riid.is_null() {
            return E_POINTER;
        }
        let iid = *riid;
        if iid == IID_UNKNOWN || iid == IID_AGILE || own.contains(&iid) {
            add_ref(this);
            *out = this;
            S_OK
        } else if iid == IID_MARSHAL && !ftm.is_null() {
            // Its IMarshal counts on (and answers for) `this`.
            let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = core::mem::transmute(*(*(ftm as *const *const usize)));
            f(ftm, riid, out)
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// The free-threaded marshaler aggregated by `outer` (its inner IUnknown,
/// released with `outer`), or null.
fn ftm_for(outer: *mut c_void) -> *mut c_void {
    use windows::Win32::System::Com::CoCreateFreeThreadedMarshaler;
    use windows::core::IUnknown;
    // Borrowed: aggregation takes no reference on the outer object.
    let o = core::mem::ManuallyDrop::new(unsafe { IUnknown::from_raw(outer) });
    match unsafe { CoCreateFreeThreadedMarshaler(&*o) } {
        Ok(inner) => inner.into_raw(),
        Err(e) => {
            trace(&format!("no free-threaded marshaler: {}", e.message()));
            core::ptr::null_mut()
        }
    }
}

/// Let go of an aggregated marshaler (from [`ftm_for`]).
unsafe fn drop_ftm(ftm: *mut c_void) {
    if !ftm.is_null() {
        unsafe { release(ftm) };
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
    ftm: *mut c_void,
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
        let d = &*(this as *const Delegate);
        qi(this, riid, out, &[d.iid], d.ftm)
    }
}

unsafe extern "system" fn d_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Delegate)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn d_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Delegate)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            let d = Box::from_raw(this as *mut Delegate);
            drop_ftm(d.ftm);
            drop(d);
        }
        n
    }
}

unsafe extern "system" fn d_invoke(this: *mut c_void, sender: *mut c_void, args: *mut c_void) -> HRESULT {
    unsafe { ((*(this as *const Delegate)).f)(sender, args) }
}

/// A new delegate for interface `iid` calling `f(sender, args)`.
fn delegate(iid: GUID, f: impl Fn(*mut c_void, *mut c_void) -> HRESULT + Send + Sync + 'static) -> Com {
    let d = Box::into_raw(Box::new(Delegate { vt: &DELEGATE_VT, refs: AtomicU32::new(1), iid, f: Box::new(f), ftm: core::ptr::null_mut() }));
    unsafe { (*d).ftm = ftm_for(d as *mut c_void) };
    Com(d as *mut c_void)
}

/// A one-item `IIterable<IStorageItem>` (and its iterators).
#[repr(C)]
struct Items {
    vt: &'static IterableVtbl,
    refs: AtomicU32,
    item: Com,
    ftm: *mut c_void,
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
    ftm: *mut c_void,
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
    unsafe { qi(this, riid, out, &[IID_INSPECTABLE, IID_ITERABLE_ITEM], (*(this as *const Items)).ftm) }
}

unsafe extern "system" fn it_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Items)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn it_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Items)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            let b = Box::from_raw(this as *mut Items);
            drop_ftm(b.ftm);
            drop(b);
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
        let it = Box::into_raw(Box::new(Iter { vt: &ITERATOR_VT, refs: AtomicU32::new(1), item, at: AtomicU32::new(0), ftm: core::ptr::null_mut() }));
        (*it).ftm = ftm_for(it as *mut c_void);
        *out = it as *mut c_void;
    }
    S_OK
}

unsafe extern "system" fn iter_query(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe { qi(this, riid, out, &[IID_INSPECTABLE, IID_ITERATOR_ITEM], (*(this as *const Iter)).ftm) }
}

unsafe extern "system" fn iter_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Iter)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn iter_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Iter)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            let b = Box::from_raw(this as *mut Iter);
            drop_ftm(b.ftm);
            drop(b);
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
    let b = Box::into_raw(Box::new(Items { vt: &ITERABLE_VT, refs: AtomicU32::new(1), item, ftm: core::ptr::null_mut() }));
    unsafe { (*b).ftm = ftm_for(b as *mut c_void) };
    Com(b as *mut c_void)
}

// --- the owner window ------------------------------------------------------------

mod owner {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect, IsWindow, LWA_ALPHA, RegisterClassExW, SW_SHOWNA,
        SetLayeredWindowAttributes, ShowWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    /// The owner's size (px): small, centered where the sheet should go.
    const SIDE: i32 = 64;

    unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(h, m, w, l) }
    }

    /// The middle of the window that asked, else of the pointer's
    /// monitor's work area.
    fn center(asker: Option<usize>) -> (i32, i32) {
        unsafe {
            if let Some(h) = asker.filter(|h| *h != 0).map(|h| HWND(h as *mut core::ffi::c_void))
                && IsWindow(Some(h)).as_bool()
            {
                let mut r = RECT::default();
                if GetWindowRect(h, &mut r).is_ok() && r.right > r.left && r.bottom > r.top {
                    return (r.left / 2 + r.right / 2, r.top / 2 + r.bottom / 2);
                }
            }
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                let r = mi.rcWork;
                return (r.left / 2 + r.right / 2, r.top / 2 + r.bottom / 2);
            }
            (p.x, p.y)
        }
    }

    /// An invisible, click-through window owning the sheet, centered on
    /// `asker` (or the pointer's monitor); never activated (it takes no
    /// keys); destroyed on drop.
    pub struct Owner(pub HWND);

    impl Owner {
        pub fn new(asker: Option<usize>) -> Result<Owner, String> {
            unsafe {
                let inst = GetModuleHandleW(None).map_err(|e| e.message())?;
                let class = w!("RustshotShareOwner");
                let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
                // A second share finds the class registered: fine.
                let _ = RegisterClassExW(&wc);
                let (cx, cy) = center(asker);
                let h = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                    class,
                    w!("Rustshot"),
                    WS_POPUP,
                    cx - SIDE / 2,
                    cy - SIDE / 2,
                    SIDE,
                    SIDE,
                    None,
                    None,
                    Some(inst.into()),
                    None,
                )
                .map_err(|e| format!("owner window: {}", e.message()))?;
                // Fully transparent: shown (the sheet wants a real window),
                // never seen, never hit, never active. The sheet's host may
                // come to the front (this process was just clicked).
                let _ = SetLayeredWindowAttributes(h, windows::Win32::Foundation::COLORREF(0), 0, LWA_ALPHA);
                let _ = ShowWindow(h, SW_SHOWNA);
                let _ = AllowSetForegroundWindow(ASFW_ANY);
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

/// How long the sheet may take to ask for the package: longer the first
/// time in a process (the share host may start cold). It never asks when
/// another sheet is already up (that one stays; it outlives its owner).
pub const ASK_WAIT_COLD: Duration = Duration::from_secs(15);
pub const ASK_WAIT: Duration = Duration::from_secs(5);

/// How long the caller waits after the target was chosen, or at all when
/// the package cannot say when the share ends.
pub const SHORT_WAIT: Duration = Duration::from_secs(60);

/// What the share thread does next ([`Clock::step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Go on pumping.
    Pump,
    /// The sheet never asked for the package: give up (an error).
    GiveUp,
    /// Let the caller go (the time limit); go on pumping until the share
    /// ends or the process does.
    Release,
}

/// The share's times: when the sheet was shown, asked for the package, a
/// target was chosen; whether the package says when the share ends
/// (`tracked`).
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub shown: Instant,
    pub ask_wait: Duration,
    pub max: Duration,
    pub asked: Option<Instant>,
    pub chosen: Option<Instant>,
    pub tracked: bool,
}

impl Clock {
    /// When the caller is let go (asked for the package), else `None`.
    pub fn release_at(&self) -> Option<Instant> {
        let asked = self.asked?;
        // Past what an Instant holds: a year (as good as never).
        let far = |t: Instant, d: Duration| t.checked_add(d).or_else(|| t.checked_add(Duration::from_secs(365 * 86400))).unwrap_or(t);
        let mut at = if self.tracked { far(self.shown, self.max) } else { far(asked, SHORT_WAIT.min(self.max)) };
        if let Some(c) = self.chosen {
            at = at.min(far(c, SHORT_WAIT));
        }
        Some(at)
    }

    pub fn step(&self, now: Instant) -> Step {
        match self.release_at() {
            None if now >= self.shown.checked_add(self.ask_wait).unwrap_or(self.shown) => Step::GiveUp,
            None => Step::Pump,
            Some(at) if now >= at => Step::Release,
            Some(_) => Step::Pump,
        }
    }
}

/// Handle this thread's messages; wait up to 100 ms for more.
fn pump_once() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage};
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
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
    item: Agile,
    /// The picture's stream (bitmap and thumbnail); none for a recording.
    bitmap: Option<Agile>,
    description: HSTRING,
}

/// What the handlers report to the share thread.
#[derive(Default)]
struct Events {
    /// The share ended (completed or canceled).
    done: AtomicBool,
    /// The package says when the share ends (ShareCanceled registered).
    tracked: AtomicBool,
    asked: std::sync::Mutex<Option<Instant>>,
    chosen: std::sync::Mutex<Option<Instant>>,
    /// Filling the package failed: why (the sheet says so too).
    failed: std::sync::Mutex<Option<String>>,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Fill the requested package from `c`. A failure is told to the sheet
/// (`FailWithDisplayText`), which then closes as canceled.
fn fill(args: *mut c_void, c: &Content, ev: &Arc<Events>) -> R<()> {
    let args = Com::borrowed(args).ok_or("no request")?;
    let request = args.get_obj(6, "Request")?; // IDataRequestedEventArgs::Request
    let r = fill_package(&request, c, ev);
    if let Err(e) = &r {
        // IDataRequest::FailWithDisplayText (9).
        let text = HSTRING::from(format!("Rustshot could not share this capture: {e}"));
        let _ = request.call_ptr(9, hs(&text), "FailWithDisplayText");
    }
    r
}

fn fill_package(request: &Com, c: &Content, ev: &Arc<Events>) -> R<()> {
    let pkg = request.get_obj(6, "Data")?; // IDataRequest::Data
    // First the end of the share: completed (IDataPackage3) or canceled
    // (IDataPackage4); the handlers stay with the package.
    let end = |iid: GUID| {
        let e = ev.clone();
        delegate(iid, move |_, _| {
            e.done.store(true, Ordering::SeqCst);
            S_OK
        })
    };
    let completed = pkg.query(&IID_DATA_PACKAGE3, "IDataPackage3").and_then(|p| p.add_event(6, &end(IID_H_SHARE_COMPLETED), "ShareCompleted"));
    let canceled = pkg.query(&IID_DATA_PACKAGE4, "IDataPackage4").and_then(|p| p.add_event(6, &end(IID_H_SHARE_CANCELED), "ShareCanceled"));
    if let Err(e) = &completed {
        trace(&format!("ShareCompleted not registered: {e}"));
    }
    if let Err(e) = &canceled {
        trace(&format!("ShareCanceled not registered: {e}"));
    }
    ev.tracked.store(canceled.is_ok(), Ordering::SeqCst);
    let props = pkg.get_obj(7, "Properties")?; // IDataPackage::Properties
    let title = HSTRING::from(TITLE);
    props.call_ptr(7, hs(&title), "SetTitle")?;
    props.call_ptr(9, hs(&c.description), "SetDescription")?;
    if let Some(b) = &c.bitmap {
        let _ = props.call_ptr(11, b.0.0, "SetThumbnail");
        pkg.call_ptr(21, b.0.0, "SetBitmap")?;
    }
    let items = one_item(Com::borrowed(c.item.0.0).ok_or("no item")?);
    // IDataPackage::SetStorageItems(items, readOnly: boolean).
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u8) -> HRESULT = pkg.slot(23);
        f(pkg.0, items.0, 1)
    };
    check(hr, "SetStorageItems")
}

/// With `RUSTSHOT_SHARE_TRACE` set: say how the share goes (live tests).
fn trace(what: &str) {
    if std::env::var_os("RUSTSHOT_SHARE_TRACE").is_some() {
        eprintln!("rustshot: share: {what}");
    }
}

/// Lets the share's caller go, once, with the result, and frees the one
/// sheet ([`super::Sheet`]) then. Dropped unsent: a failure.
pub struct Release(Option<(crate::chan::Sender<Result<(), String>>, super::Sheet)>);

impl Release {
    pub fn new(tx: crate::chan::Sender<Result<(), String>>, sheet: super::Sheet) -> Release {
        Release(Some((tx, sheet)))
    }

    fn send(&mut self, r: Result<(), String>) {
        if let Some((tx, sheet)) = self.0.take() {
            drop(sheet);
            let _ = tx.send(r);
        }
    }

    fn sent(&self) -> bool {
        self.0.is_none()
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.send(Err("Share failed".into()));
    }
}

/// The first share of this process (the host may start cold).
static COLD: AtomicBool = AtomicBool::new(true);

/// What the sheet says when it never asked for the package.
pub const NOT_OPENED: &str = "The share sheet did not open";

/// Share `file` through a sheet over `asker` (a raw window handle), on
/// this (new) thread. The caller is let go through `release` when the
/// share ends, at the time limit ([`Clock`]), or on a failure; the thread
/// stays while the sheet may still ask for the data.
pub fn share(file: &Path, asker: Option<usize>, kind: Kind, max: Duration, mut release: Release) {
    if let Err(e) = run(file, asker, kind, max, &mut release) {
        trace(&e);
        release.send(Err(e));
    }
}

fn run(file: &Path, asker: Option<usize>, kind: Kind, max: Duration, release: &mut Release) -> R<()> {
    let failed = |e: String| format!("Share failed: {e}");
    let _apt = Apartment::enter();
    // The owner first, while this process may still bring a window up.
    let owner = owner::Owner::new(asker).map_err(failed)?;
    let hwnd = owner.0;
    let path = HSTRING::from(file.as_os_str());
    let statics = factory("Windows.Storage.StorageFile", &IID_STORAGE_FILE_STATICS).map_err(failed)?;
    let op = statics.call_ptr_obj(6, hs(&path), "GetFileFromPathAsync").map_err(failed)?;
    let sfile = wait(&op, Duration::from_secs(10), "GetFileFromPathAsync").map_err(failed)?;
    let item = sfile.query(&IID_STORAGE_ITEM, "IStorageItem").map_err(failed)?;
    let bitmap = match kind {
        Kind::Picture => {
            let refs = factory("Windows.Storage.Streams.RandomAccessStreamReference", &IID_STREAM_REF_STATICS).map_err(failed)?;
            Some(Agile(refs.call_ptr_obj(6, sfile.0, "CreateFromFile").map_err(failed)?))
        }
        Kind::File => None,
    };
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let content = Arc::new(Content { item: Agile(item), bitmap, description: HSTRING::from(name) });

    let interop = factory("Windows.ApplicationModel.DataTransfer.DataTransferManager", &IID_INTEROP).map_err(failed)?;
    // IDataTransferManagerInterop::GetForWindow (3).
    let mut dtm = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = interop.slot(3);
        f(interop.0, hwnd.0, &IID_DATA_TRANSFER_MANAGER, &mut dtm)
    };
    check(hr, "GetForWindow").map_err(failed)?;
    let dtm = Com::new(dtm).ok_or("GetForWindow: nothing").map_err(|e| failed(e.into()))?;

    let ev: Arc<Events> = Arc::default();
    let on_request = {
        let (c, ev) = (content.clone(), ev.clone());
        delegate(IID_H_DATA_REQUESTED, move |_, args| {
            *lock(&ev.asked) = Some(Instant::now());
            match fill(args, &c, &ev) {
                Ok(()) => trace("package filled"),
                Err(e) => {
                    trace(&format!("package failed: {e}"));
                    *lock(&ev.failed) = Some(format!("Share failed: {e}"));
                }
            }
            S_OK
        })
    };
    let on_chosen = {
        let ev = ev.clone();
        delegate(IID_H_TARGET_CHOSEN, move |_, _| {
            *lock(&ev.chosen) = Some(Instant::now());
            S_OK
        })
    };
    let t_request = dtm.add_event(6, &on_request, "DataRequested").map_err(failed)?;
    let t_chosen = match dtm.add_event(8, &on_chosen, "TargetApplicationChosen") {
        Ok(t) => Some(t),
        Err(e) => {
            trace(&format!("TargetApplicationChosen not registered: {e}"));
            None
        }
    };
    // IDataTransferManagerInterop::ShowShareUIForWindow (4).
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT = interop.slot(4);
        f(interop.0, hwnd.0)
    };
    let shown = check(hr, "ShowShareUIForWindow");
    let ask_wait = if COLD.swap(false, Ordering::SeqCst) { ASK_WAIT_COLD } else { ASK_WAIT };
    let mut clock = Clock { shown: Instant::now(), ask_wait, max, asked: None, chosen: None, tracked: false };
    let mut outcome = shown.clone().map_err(failed);
    let mut said_untracked = false;
    if shown.is_ok() {
        trace(&format!("sheet shown for {}", file.display()));
        loop {
            pump_once();
            clock.asked = *lock(&ev.asked);
            clock.chosen = *lock(&ev.chosen);
            clock.tracked = ev.tracked.load(Ordering::SeqCst);
            if clock.asked.is_some() && !clock.tracked && !said_untracked {
                said_untracked = true;
                eprintln!("rustshot: share: the sheet cannot say when the share ends; waiting at most {} s", SHORT_WAIT.as_secs());
            }
            let result = || lock(&ev.failed).clone().map_or(Ok(()), Err);
            if ev.done.load(Ordering::SeqCst) {
                release.send(result());
                break;
            }
            match clock.step(Instant::now()) {
                Step::Pump => {}
                Step::GiveUp => {
                    outcome = Err(NOT_OPENED.into());
                    break;
                }
                Step::Release if !release.sent() => {
                    trace("caller let go; the sheet may still ask for the data");
                    release.send(result());
                }
                Step::Release => {}
            }
        }
    }
    dtm.remove_event(7, t_request);
    if let Some(t) = t_chosen {
        dtm.remove_event(9, t);
    }
    trace(&format!(
        "ended (asked: {}, done: {}, tracked: {}, target chosen: {})",
        clock.asked.is_some(),
        ev.done.load(Ordering::SeqCst),
        clock.tracked,
        clock.chosen.is_some()
    ));
    drop(owner);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A null interface is `None`, and nothing is released (an eager
    /// `then_some(Com(p))` dropped a `Com(null)`, which crashes).
    #[test]
    fn a_null_interface_is_none_and_never_released() {
        assert!(Com::new(core::ptr::null_mut()).is_none());
    }

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
            assert!(list.query(&IID_MARSHAL, "q").is_ok(), "free-threaded");
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
        // IMarshal: the aggregated free-threaded marshaler; it holds the
        // delegate while it lives.
        let m = d.query(&IID_MARSHAL, "IMarshal").expect("IMarshal");
        let back = m.query(&IID_H_SHARE_CANCELED, "back to the delegate").unwrap();
        assert_eq!(back.0, d.0, "its IUnknown is the delegate's");
        drop((m, back));
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

    /// Manual: the sheet comes up for a PNG; press Esc on it. With
    /// `RUSTSHOT_SHARE_TRACE=1` the trace says "package filled", then
    /// "ended (asked: true, done: true, …)", and the call returns Ok well
    /// before the limit. Run by hand:
    /// `cargo test share::win::tests::manual -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual: opens a system sheet; press Esc"]
    fn manual_sheet_opens_and_esc_cancels() {
        let dir = std::env::temp_dir().join(format!("rustshot-share-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("live.png");
        std::fs::write(&f, crate::pixbuf::PixBuf::from_pixel(64, 48, [30, 120, 220, 255]).to_png().unwrap()).unwrap();
        let sheet = crate::share::Sheet::claim().unwrap();
        let (tx, rx) = crate::chan::channel();
        let t0 = Instant::now();
        let p = f.clone();
        std::thread::spawn(move || share(&p, None, Kind::Picture, Duration::from_secs(120), Release::new(tx, sheet)));
        assert_eq!(rx.recv().unwrap(), Ok(()));
        assert!(t0.elapsed() < Duration::from_secs(110), "Esc ended the share (not the limit)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The caller goes when the sheet never asks (cold: 15 s, else 5 s);
    /// asked, at the limit (tracked) or 60 s after (not tracked), sooner
    /// once a target was chosen; never with an overflow.
    #[test]
    fn the_clock_lets_go_in_time() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut c = Clock { shown: t0, ask_wait: ASK_WAIT_COLD, max: Duration::from_secs(600), asked: None, chosen: None, tracked: true };
        assert_eq!(c.step(s(14)), Step::Pump, "a cold host gets 15 s");
        assert_eq!(c.step(s(15)), Step::GiveUp);
        c.ask_wait = ASK_WAIT;
        assert_eq!(c.step(s(5)), Step::GiveUp, "later shares 5 s");
        c.asked = Some(s(1));
        assert_eq!(c.step(s(300)), Step::Pump, "asked and tracked: until the limit");
        assert_eq!(c.step(s(600)), Step::Release);
        c.chosen = Some(s(10));
        assert_eq!(c.step(s(69)), Step::Pump);
        assert_eq!(c.step(s(70)), Step::Release, "60 s after a target was chosen");
        c.chosen = None;
        c.tracked = false;
        assert_eq!(c.step(s(60)), Step::Pump);
        assert_eq!(c.step(s(61)), Step::Release, "no end events: 60 s");
        c.max = Duration::MAX;
        c.tracked = true;
        assert_eq!(c.step(s(10_000)), Step::Pump, "no overflow");
    }

    /// The caller hears once: the first result; dropped unsent, a failure;
    /// the sheet is free when it has heard.
    #[test]
    fn release_tells_the_caller_once() {
        let (tx, rx) = crate::chan::channel();
        let mut r = Release::new(tx, crate::share::Sheet::claim().unwrap());
        assert!(crate::share::Sheet::claim().is_none());
        r.send(Ok(()));
        assert!(r.sent() && crate::share::Sheet::claim().is_some());
        r.send(Err("late".into()));
        drop(r);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [Ok(())]);
        let (tx, rx) = crate::chan::channel();
        drop(Release::new(tx, crate::share::Sheet::claim().unwrap()));
        assert!(rx.recv().unwrap().is_err());
    }
}
