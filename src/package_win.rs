//! The MSIX (Store) package, Windows: what only a packaged process can ask
//! through WinRT, with hand-rolled vtables as in `ocr/win.rs` (the
//! `windows` crate supplies `RoGetActivationFactory`, `HSTRING` and GUIDs):
//!
//! - its startup task ("Start at login", the manifest's `RustshotDaemon`):
//!   `Windows.ApplicationModel.StartupTask` (`GetAsync`, `State`,
//!   `RequestEnableAsync`, `Disable`);
//! - how this process was started: `AppInstance.GetActivatedEventArgs()`'s
//!   `Kind` (`StartupTask` at sign-in);
//! - its package family name (`GetCurrentPackageFamilyName`), which names
//!   the package's `AppX…` ProgID for `ms-screenclip`.
//!
//! The calls take the vtable slot of each method: IUnknown 0-2,
//! IInspectable 3-5, then the interface's own methods from 6 (as the
//! `windows` crate's generated vtables list them).

use core::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize, RoUninitialize};
use windows::core::{GUID, HRESULT, HSTRING, IInspectable, Interface};

/// The manifest's startup task (`desktop:StartupTask TaskId`).
pub const TASK_ID: &str = "RustshotDaemon";

const IID_STARTUP_TASK_STATICS: GUID = GUID::from_u128(0xee5b60bd_a148_41a7_b26e_e8b88a1e62f8);
const IID_APP_INSTANCE_STATICS: GUID = GUID::from_u128(0x9d11e77f_9ea6_47af_a6ec_46784c5ba254);
const IID_ASYNC_INFO: GUID = GUID::from_u128(0x00000036_0000_0000_c000_000000000046);

/// `Windows.ApplicationModel.Activation.ActivationKind.StartupTask`.
pub const KIND_STARTUP_TASK: i32 = 1020;

/// `AsyncStatus`.
const STARTED: i32 = 0;
const COMPLETED: i32 = 1;
const CANCELED: i32 = 2;
/// `RequestEnableAsync` may ask the user (a first enable after a Store
/// install); a few seconds more than that question takes.
const TIMEOUT: Duration = Duration::from_secs(120);

/// An owned COM interface pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    fn new(p: *mut c_void) -> Option<Com> {
        (!p.is_null()).then(|| Com(p))
    }

    /// Method `slot` of the vtable, as `F`.
    unsafe fn slot<F: Copy>(&self, slot: usize) -> F {
        debug_assert_eq!(size_of::<F>(), size_of::<usize>());
        unsafe {
            let vt = *(self.0 as *const *const usize);
            core::mem::transmute_copy(&*vt.add(slot))
        }
    }

    fn query(&self, iid: &GUID) -> Result<Com, String> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = self.slot(0);
            f(self.0, iid, &mut out)
        };
        check(hr, "QueryInterface")?;
        Com::new(out).ok_or_else(|| "QueryInterface returned nothing".into())
    }

    /// A method returning an interface (`slot`, no arguments).
    fn get_obj(&self, slot: usize, what: &str) -> Result<Option<Com>, String> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Ok(Com::new(out))
    }

    /// A getter returning a value (`slot`, no arguments).
    fn get<T: Copy + Default>(&self, slot: usize, what: &str) -> Result<T, String> {
        let mut out = T::default();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut T) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Ok(out)
    }

    /// A method with no arguments and no result.
    fn call(&self, slot: usize, what: &str) -> Result<(), String> {
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void) -> HRESULT = self.slot(slot);
            f(self.0)
        };
        check(hr, what)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void) -> u32 = self.slot(2);
            f(self.0);
        }
    }
}

fn check(hr: HRESULT, what: &str) -> Result<(), String> {
    if hr.is_ok() { Ok(()) } else { Err(format!("{what}: {}", hr.message().trim())) }
}

/// The activation factory of runtime class `class`, as interface `iid`.
fn factory(class: &str, iid: &GUID) -> Result<Com, String> {
    let f: IInspectable = unsafe { RoGetActivationFactory(&HSTRING::from(class)) }.map_err(|e| format!("{class}: {}", e.message().trim()))?;
    // Borrowed from `f` (which releases its own reference on drop).
    let borrowed = core::mem::ManuallyDrop::new(Com(f.as_raw()));
    borrowed.query(iid)
}

/// The WinRT apartment of this thread, left on drop (when it entered one).
struct Apartment(bool);

impl Apartment {
    fn enter() -> Apartment {
        Apartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { RoUninitialize() };
        }
    }
}

/// Wait for an `IAsyncOperation<T>` to complete (polling `IAsyncInfo`).
fn wait(op: &Com) -> Result<(), String> {
    let info = op.query(&IID_ASYNC_INFO)?;
    let t0 = Instant::now();
    loop {
        let st: i32 = info.get(7, "Status")?;
        match st {
            STARTED if t0.elapsed() > TIMEOUT => {
                let _ = info.call(9, "Cancel");
                return Err("timed out".into());
            }
            STARTED => std::thread::sleep(Duration::from_millis(5)),
            COMPLETED => return Ok(()),
            CANCELED => return Err("cancelled".into()),
            _ => {
                let hr: HRESULT = info.get(8, "ErrorCode")?;
                return Err(hr.message().trim().to_string());
            }
        }
    }
}

/// The package's startup task [`TASK_ID`] (`StartupTask.GetAsync`).
fn startup_task() -> Result<Com, String> {
    let statics = factory("Windows.ApplicationModel.StartupTask", &IID_STARTUP_TASK_STATICS)?;
    let id = HSTRING::from(TASK_ID);
    let mut op = core::ptr::null_mut();
    // IStartupTaskStatics::GetAsync (slot 7).
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT = statics.slot(7);
        f(statics.0, core::mem::transmute_copy(&id), &mut op)
    };
    check(hr, "StartupTask.GetAsync")?;
    let op = Com::new(op).ok_or("StartupTask.GetAsync returned nothing")?;
    wait(&op)?;
    // IAsyncOperation<StartupTask>::GetResults (slot 8).
    op.get_obj(8, "GetResults")?.ok_or_else(|| format!("no startup task {TASK_ID} in the package"))
}

/// The startup task's `StartupTaskState` (0 Disabled, 1 DisabledByUser,
/// 2 Enabled, 3 DisabledByPolicy, 4 EnabledByPolicy).
pub fn startup_state() -> Result<i32, String> {
    let _apt = Apartment::enter();
    // IStartupTask::State (slot 8).
    startup_task()?.get::<i32>(8, "StartupTask.State")
}

/// Turn the startup task on (`RequestEnableAsync`, which may ask the
/// user) or off (`Disable`); the state it is in afterwards.
pub fn set_startup(on: bool) -> Result<i32, String> {
    let _apt = Apartment::enter();
    let task = startup_task()?;
    if on {
        // IStartupTask::RequestEnableAsync (slot 6): IAsyncOperation<StartupTaskState>.
        let op = task.get_obj(6, "RequestEnableAsync")?.ok_or("RequestEnableAsync returned nothing")?;
        wait(&op)?;
        // GetResults (slot 8) of an enum: an i32.
        op.get::<i32>(8, "GetResults")
    } else {
        // IStartupTask::Disable (slot 7).
        task.call(7, "StartupTask.Disable")?;
        task.get::<i32>(8, "StartupTask.State")
    }
}

/// How Windows started this process: `ActivationKind`
/// (`AppInstance.GetActivatedEventArgs().Kind`); `None` unpackaged or
/// when Windows says nothing.
pub fn activation_kind() -> Option<i32> {
    let _apt = Apartment::enter();
    let statics = factory("Windows.ApplicationModel.AppInstance", &IID_APP_INSTANCE_STATICS).ok()?;
    // IAppInstanceStatics::GetActivatedEventArgs (slot 7) -> IActivatedEventArgs.
    let args = statics.get_obj(7, "GetActivatedEventArgs").ok()??;
    // IActivatedEventArgs::Kind (slot 6).
    args.get::<i32>(6, "Kind").ok()
}

/// This process's package family name (`<Name>_<PublisherId>`); `None`
/// unpackaged.
pub fn family_name() -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;
    let mut len = 0u32;
    let _ = unsafe { GetCurrentPackageFamilyName(&mut len, None) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    let r = unsafe { GetCurrentPackageFamilyName(&mut len, Some(windows::core::PWSTR(buf.as_mut_ptr()))) };
    if r != ERROR_SUCCESS {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

#[cfg(test)]
mod tests {
    /// Unpackaged (the test binary): no package, no activation to read,
    /// and no startup task (the API needs the package's identity).
    #[test]
    fn unpackaged_has_no_package() {
        assert_eq!(super::family_name(), None);
        assert!(super::startup_state().is_err());
    }
}
