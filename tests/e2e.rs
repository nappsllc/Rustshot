//! End-to-end interaction tests: launch the real binary and drive it with
//! posted Win32 messages (keys, mouse, chars). Ignored by default like the
//! other live tests; run with:
//!
//!     cargo test --test e2e -- --ignored
//!
//! The child gets its own config dir (APPDATA) so the user's config is
//! never read or written; `RUSTSHOT_E2E_RENDERER=gdi|software` picks the
//! overlay renderer (default gdi).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CHAR, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

const BIN_BUILT: &str = env!("CARGO_BIN_EXE_rustshot");

/// The binary under test (`RUSTSHOT_E2E_BIN` overrides, e.g. an older build).
fn bin() -> String {
    std::env::var("RUSTSHOT_E2E_BIN").unwrap_or_else(|_| BIN_BUILT.into())
}

#[repr(C)]
struct Slot {
    pid: u32,
    hwnd: HWND,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let slot = &mut *(lparam.0 as *mut Slot);
        let mut p = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut p));
        if p == slot.pid && IsWindowVisible(hwnd).as_bool() {
            slot.hwnd = hwnd;
            BOOL(0)
        } else {
            BOOL(1)
        }
    }
}

/// Find the visible top-level window of process `pid`.
fn find_window(pid: u32) -> Option<HWND> {
    let mut slot = Slot {
        pid,
        hwnd: HWND::default(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut slot as *mut Slot as isize));
    }
    (!slot.hwnd.0.is_null()).then_some(slot.hwnd)
}

fn wait_window(child: &Child, ms: u64) -> Option<HWND> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if let Some(h) = find_window(child.id()) {
            return Some(h);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_exit(child: &mut Child, ms: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            return st.code();
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A temp APPDATA root, removed on drop.
struct TempRoot(PathBuf);

impl std::ops::Deref for TempRoot {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ROOT_PREFIX: &str = "rustshot-e2e-";

/// Remove `rustshot-e2e-<pid>-*` dirs earlier runs left in %TEMP% (only
/// that prefix, and not this process' own).
fn sweep_old_roots() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let own = format!("{ROOT_PREFIX}{}-", std::process::id());
        let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(ROOT_PREFIX) && !name.starts_with(&own) && e.path().is_dir() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    });
}

/// A fresh APPDATA root whose `rustshot/config.toml` selects `renderer`.
fn config_root(tag: &str, renderer: &str) -> TempRoot {
    sweep_old_roots();
    let root = std::env::temp_dir().join(format!("{ROOT_PREFIX}{}-{tag}", std::process::id()));
    let dir = root.join("rustshot");
    std::fs::create_dir_all(&dir).expect("config dir");
    std::fs::write(dir.join("config.toml"), format!("renderer = \"{renderer}\"\ncheck_updates = false\n"))
        .expect("config");
    TempRoot(root)
}

fn renderer() -> String {
    std::env::var("RUSTSHOT_E2E_RENDERER").unwrap_or_else(|_| "gdi".into())
}

fn rustshot(appdata: &Path) -> Command {
    let mut c = Command::new(bin());
    c.env("APPDATA", appdata);
    c
}

/// The editor on an 800x600 region; keep the root until the child exits.
fn launch_editor(tag: &str) -> (Child, HWND, TempRoot) {
    let root = config_root(tag, &renderer());
    let child = rustshot(&root)
        .args(["gui", "--region", "800x600+20+20", "--clip"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rustshot");
    let hwnd = wait_window(&child, 8000).expect("editor window");
    (child, hwnd, root)
}

fn post(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
    let _ = unsafe { PostMessageW(Some(hwnd), msg, w, l) };
}

fn pack(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u32) << 16) | (x as u32 & 0xFFFF)) as isize)
}

fn key_down(hwnd: HWND, vk: u32) {
    post(hwnd, WM_KEYDOWN, WPARAM(vk as usize), LPARAM(0));
}
fn key_up(hwnd: HWND, vk: u32) {
    post(hwnd, WM_KEYUP, WPARAM(vk as usize), LPARAM(0));
}
fn key(hwnd: HWND, vk: u32) {
    key_down(hwnd, vk);
    key_up(hwnd, vk);
}
fn mouse_down(hwnd: HWND, x: i32, y: i32) {
    post(hwnd, WM_LBUTTONDOWN, WPARAM(1), pack(x, y));
}
fn mouse_up(hwnd: HWND, x: i32, y: i32) {
    post(hwnd, WM_LBUTTONUP, WPARAM(0), pack(x, y));
}
fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    post(hwnd, WM_MOUSEMOVE, WPARAM(1), pack(x, y));
}
fn char_msg(hwnd: HWND, c: char) {
    post(hwnd, WM_CHAR, WPARAM(c as usize), LPARAM(0));
}

#[test]
#[ignore = "live display access"]
fn escape_cancels_with_code_2() {
    let (mut child, hwnd, _root) = launch_editor("escape");
    key(hwnd, 0x1B); // VK_ESCAPE
    assert_eq!(wait_exit(&mut child, 5000), Some(2), "esc should cancel");
}

#[test]
#[ignore = "live display access"]
fn rect_draw_then_enter_exports_zero() {
    let (mut child, hwnd, _root) = launch_editor("rect");
    key(hwnd, 'R' as u32);
    mouse_move(hwnd, 400, 300);
    mouse_down(hwnd, 400, 300);
    mouse_move(hwnd, 500, 400);
    mouse_move(hwnd, 600, 380);
    mouse_up(hwnd, 600, 380);
    key(hwnd, 0x0D); // VK_RETURN accepts
    assert_eq!(
        wait_exit(&mut child, 5000),
        Some(0),
        "draw + accept should succeed"
    );
}

#[test]
#[ignore = "live display access"]
fn text_tool_types_and_accepts() {
    let (mut child, hwnd, _root) = launch_editor("text");
    key(hwnd, 'T' as u32);
    mouse_down(hwnd, 300, 300);
    mouse_up(hwnd, 300, 300);
    for c in "Hello".chars() {
        char_msg(hwnd, c);
    }
    key(hwnd, 0x0D); // commit text draft
    key(hwnd, 0x0D); // accept
    assert_eq!(
        wait_exit(&mut child, 5000),
        Some(0),
        "text + accept should succeed"
    );
}

/// Copy text (O) in a one-shot editor: the overlay closes, the area's
/// text is recognized (Windows OCR) and the outcome is printed. Whatever
/// is on screen there: "Copied text · N lines" (the clipboard then holds
/// that text) or "No text found"; without an OCR language, the note
/// saying so (exit 1).
#[test]
#[ignore = "live display access; may replace the clipboard's text"]
fn copy_text_key_recognizes_the_area() {
    let root = config_root("ocr", &renderer());
    // All at once (the default lets you select words: `text_mode_*`).
    append_config(&root, "copy_text_mode = \"all\"\n");
    let mut child = rustshot(&root)
        .env("RUSTSHOT_INSTANCE", format!("e2e-ocr-{}", std::process::id()))
        .env("LOCALAPPDATA", &*root)
        .args(["gui", "--region", "800x600+20+20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rustshot");
    let hwnd = wait_window(&child, 8000).expect("editor window");
    key(hwnd, 'O' as u32);
    let code = wait_exit(&mut child, 20000);
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    let line = out.lines().last().unwrap_or_default().to_string();
    eprintln!("copy text: {line:?} (exit {code:?})");
    match code {
        Some(0) => assert!(line.starts_with("Copied text \u{b7} ") || line == "No text found", "{out:?}"),
        Some(1) => assert!(line.contains("language"), "only a missing language may fail: {out:?}"),
        c => panic!("unexpected exit {c:?}: {out:?}"),
    }
}

fn append_config(root: &Path, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(root.join("rustshot").join("config.toml")).expect("config");
    f.write_all(text.as_bytes()).expect("config");
}

/// A one-shot overlay with its own instance, config and history (nothing
/// of the user's is read or written).
fn isolated(tag: &str, extra: &str, args: &[&str]) -> (Child, HWND, TempRoot) {
    let root = config_root(tag, &renderer());
    append_config(&root, extra);
    let child = rustshot(&root)
        .env("RUSTSHOT_INSTANCE", format!("e2e-{tag}-{}", std::process::id()))
        .env("LOCALAPPDATA", &*root)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rustshot");
    let hwnd = wait_window(&child, 8000).expect("editor window");
    (child, hwnd, root)
}

/// Copy text (O) with "Let me select": the overlay stays up in Text mode
/// (whatever OCR finds on screen), Esc leaves it, a second Esc cancels.
#[test]
#[ignore = "live display access"]
fn text_mode_keeps_the_overlay_until_escape() {
    let (mut child, hwnd, _root) = isolated("textmode", "", &["gui", "--region", "800x600+20+20"]);
    key(hwnd, 'O' as u32);
    assert_eq!(wait_exit(&mut child, 3000), None, "Text mode: the overlay stays");
    key(hwnd, 0x1B);
    key(hwnd, 0x1B);
    assert_eq!(wait_exit(&mut child, 5000), Some(2), "Esc leaves Text mode, then cancels");
}

/// Freeform (`selection_mode = "freeform"`): a lasso drawn with the mouse,
/// Enter saves its box with the outside transparent.
#[test]
#[ignore = "live display access"]
fn freeform_lasso_saves_a_transparent_outside() {
    let out = std::env::temp_dir().join(format!("{ROOT_PREFIX}{}-lasso.png", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let path = out.to_string_lossy().into_owned();
    let (mut child, hwnd, _root) = isolated("lasso", "selection_mode = \"freeform\"\nsnap_windows = false\n", &["gui", "--path", &path]);
    let pts = [(300, 200), (700, 200), (500, 600), (300, 200)];
    mouse_move(hwnd, pts[0].0, pts[0].1);
    mouse_down(hwnd, pts[0].0, pts[0].1);
    for w in pts.windows(2) {
        for i in 1..=20 {
            let (x, y) = (w[0].0 + (w[1].0 - w[0].0) * i / 20, w[0].1 + (w[1].1 - w[0].1) * i / 20);
            mouse_move(hwnd, x, y);
            // At a hand's pace: the overlay draws (and the lasso takes)
            // the newest move once per display refresh.
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    mouse_up(hwnd, pts[0].0, pts[0].1);
    key(hwnd, 0x0D);
    assert_eq!(wait_exit(&mut child, 8000), Some(0), "saved");
    let bytes = std::fs::read(&out).expect("the PNG");
    let _ = std::fs::remove_file(&out);
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).expect("frame");
    assert_eq!((info.width, info.height), (400, 400), "the lasso's box");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let a = |x: usize, y: usize| buf[(y * 400 + x) * 4 + 3];
    assert_eq!(a(200, 100), 255, "inside");
    assert_eq!((a(5, 395), a(395, 395)), (0, 0), "outside: transparent");
}
/// The window classes of process `pid`'s visible top-level windows.
fn window_classes(pid: u32) -> Vec<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    struct Acc {
        pid: u32,
        out: Vec<String>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let acc = &mut *(l.0 as *mut Acc);
            let mut p = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut p));
            if p == acc.pid && IsWindowVisible(hwnd).as_bool() {
                let mut buf = [0u16; 128];
                let n = GetClassNameW(hwnd, &mut buf) as usize;
                acc.out.push(String::from_utf16_lossy(&buf[..n]));
            }
            BOOL(1)
        }
    }
    let mut acc = Acc { pid, out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut acc as *mut Acc as isize));
    }
    acc.out
}

/// Manual: Share in a one-shot capture (its key rebound to F9: posted keys
/// carry no Ctrl/Shift; only the test's own overlay gets a message). The
/// overlay closes, the share sheet comes up with the capture, and the
/// process stays. Press Esc on the sheet: the share ends (`done: true`)
/// and the process exits 0, long before the 120 s limit. Nothing is
/// shared. No input is sent to the sheet or the desktop.
///
///     cargo test --test e2e manual_share -- --ignored --nocapture
#[test]
#[ignore = "manual: opens a system sheet; press Esc"]
fn manual_share_keeps_a_one_shot_capture_alive_until_esc() {
    let root = config_root("share", &renderer());
    let cfg = root.join("rustshot").join("config.toml");
    let mut text = std::fs::read_to_string(&cfg).unwrap();
    text.push_str("\n[shortcuts]\nshare = \"F9\"\n");
    std::fs::write(&cfg, text).unwrap();
    let mut child = rustshot(&root)
        .env("RUSTSHOT_INSTANCE", format!("e2e-share-{}", std::process::id()))
        .env("LOCALAPPDATA", &*root)
        .env("RUSTSHOT_SHARE_TRACE", "1")
        .env("RUSTSHOT_SHARE_MAX_SECS", "120")
        .args(["gui", "--region", "400x300+40+40"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rustshot");
    let hwnd = wait_window(&child, 8000).expect("editor window");
    key(hwnd, 0x78); // VK_F9
    let t0 = Instant::now();
    // The overlay goes at once; the process waits for the sheet.
    std::thread::sleep(Duration::from_millis(2500));
    // Left: only the invisible owner window of the sheet.
    let classes = window_classes(child.id());
    assert_eq!(classes, ["RustshotShareOwner"], "the overlay closed, the owner stays");
    assert!(child.try_wait().unwrap().is_none(), "still running while the sheet is up");
    eprintln!("Press Esc on the share sheet now.");
    let code = wait_exit(&mut child, 130_000);
    let mut err = String::new();
    use std::io::Read;
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    eprintln!("{err}");
    assert_eq!(code, Some(0), "{err}");
    assert!(err.contains("share: sheet shown for") && err.contains(".png"), "{err}");
    assert!(err.contains("share: package filled"), "{err}");
    assert!(err.contains("share: ended (asked: true, done: true"), "Esc ended it: {err}");
    assert!(!err.contains("target chosen: true"), "nothing shared: {err}");
    assert!(t0.elapsed() < Duration::from_secs(115), "not the time limit");
}

/// Run locally, this briefly shows the tray icon of its own named daemon
/// instance (it never runs beside, or touches, the default one).
#[test]
#[ignore = "live display access"]
fn daemon_stays_alive() {
    if daemon_running() {
        // Belt and braces: never start a daemon beside the user's.
        println!("skipped: another Rustshot daemon is running in this session");
        return;
    }
    let root = config_root("daemon", &renderer());
    // Its own instance and unusual hotkeys: nothing it does can reach
    // another daemon (or take the user's hotkeys) even if one starts meanwhile.
    std::fs::write(
        root.join("rustshot").join("config.toml"),
        format!(
            "renderer = \"{}\"\ncheck_updates = false\ncapture_hotkey = \"Ctrl+Alt+Shift+F11\"\nquit_hotkey = \"Ctrl+Alt+Shift+F10\"\n{NO_OTHER_HOTKEYS}",
            renderer()
        ),
    )
    .unwrap();
    let instance = format!("e2e-dmn-{}", std::process::id());
    let mut child = rustshot(&root)
        .env("RUSTSHOT_INSTANCE", &instance)
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    // Up once it holds its instance mutex; then it must keep running.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut up = false;
    while !up && Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
        up = mutex_held(Some(&instance));
        std::thread::sleep(Duration::from_millis(50));
    }
    if up {
        std::thread::sleep(Duration::from_secs(1));
    }
    let status = child.try_wait().expect("try_wait");
    let _ = child.kill(); // only the daemon this test started
    let _ = child.wait();
    assert!(up, "the daemon never took its instance (exit: {status:?})");
    assert!(status.is_none(), "daemon exited early: {status:?}");
}

/// Whether the user's (default-instance) daemon is running.
fn daemon_running() -> bool {
    mutex_held(None)
}

/// Whether a daemon holds the single-instance mutex of `instance` (`None`:
/// the default one); opened, never created, so nothing is signalled.
fn mutex_held(instance: Option<&str>) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let name = match instance {
        Some(n) => format!("Local\\rustshot-daemon-{n}"),
        None => "Local\\rustshot-daemon".to_string(),
    };
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(wide.as_ptr())) } {
        Ok(h) => {
            let _ = unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}

/// Peak private bytes of a one-shot full-desktop capture saved as PNG,
/// per renderer (and with one rectangle drawn), from `RUSTSHOT_MEMLOG`.
///
///     cargo test --release --test e2e memory_one_shot -- --ignored --nocapture
#[test]
#[ignore = "live display access; measurement"]
fn memory_one_shot() {
    // (label, renderer, tool key + drag from/to)
    type Draw = Option<(char, (i32, i32), (i32, i32))>;
    let cases: [(&str, &str, Draw); 5] = [
        ("software", "software", None),
        ("software + 2000x900 pixelate", "software", Some(('B', (400, 300), (2400, 1200)))),
        ("gdi", "gdi", None),
        ("gdi + rect", "gdi", Some(('R', (400, 300), (900, 600)))),
        ("gdi + 2000x900 pixelate", "gdi", Some(('B', (400, 300), (2400, 1200)))),
    ];
    for (i, (label, renderer, draw)) in cases.into_iter().enumerate() {
        let root = config_root(&format!("mem-{i}"), renderer);
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let log = root.join("mem.log");
        let _ = std::fs::remove_file(&log);
        let mut child = rustshot(&root)
            .env("RUSTSHOT_MEMLOG", &log)
            .args(["gui", "--region", "all", "--path"])
            .arg(&out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rustshot");
        let hwnd = wait_window(&child, 8000).expect("editor window");
        std::thread::sleep(Duration::from_millis(500));
        if let Some((tool, (x0, y0), (x1, y1))) = draw {
            key(hwnd, tool as u32);
            mouse_move(hwnd, x0, y0);
            mouse_down(hwnd, x0, y0);
            mouse_move(hwnd, (x0 + x1) / 2, (y0 + y1) / 2);
            mouse_move(hwnd, x1, y1);
            mouse_up(hwnd, x1, y1);
            std::thread::sleep(Duration::from_millis(500));
        }
        key(hwnd, 0x0D);
        assert_eq!(wait_exit(&mut child, 15000), Some(0), "{label}: save should succeed");
        let saved = std::fs::read_dir(&out).unwrap().count();
        assert_eq!(saved, 1, "{label}: one PNG saved");
        let text = std::fs::read_to_string(&log).expect("memlog written");
        let line = |phase: &str| text.lines().find(|l| l.contains(&format!("phase={phase} "))).unwrap_or("").to_string();
        let field = |line: &str, k: &str| -> u64 {
            line.split_whitespace()
                .find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        let (ov, ex) = (line("overlay"), line("exit"));
        println!(
            "MEM {label}: peak private {:.1} MiB before export, {:.1} MiB overall (export crop + PNG included); peak working set {:.1} MiB",
            mb(field(&ov, "peak_private")),
            mb(field(&ex, "peak_private")),
            mb(field(&ex, "peak_working_set"))
        );
    }
}

/// Screen pixels of `w` x `h` at (`x`, `y`), BGRA (what DWM shows).
fn screen_pixels(x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("dib");
        let old = SelectObject(dc, HGDIOBJ(bmp.0));
        let _ = BitBlt(dc, 0, 0, w, h, Some(screen), x, y, SRCCOPY);
        let _ = GdiFlush();
        let v = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        v
    }
}

/// A still backdrop for the screen: a top-most window of one color over
/// the primary monitor, on its own thread (with a message loop), taken
/// down on drop. The overlay captures it, so both renderers show the same
/// picture whatever the desktop does between the runs.
struct Backdrop {
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

unsafe extern "system" fn backdrop_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> windows::Win32::Foundation::LRESULT {
    unsafe { windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(h, m, w, l) }
}

impl Backdrop {
    fn new(rgb: u32) -> Backdrop {
        use windows::core::w;
        use windows::Win32::Graphics::Gdi::CreateSolidBrush;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::*;
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(backdrop_proc),
                hInstance: inst.into(),
                lpszClassName: w!("rustshot_e2e_backdrop"),
                hbrBackground: CreateSolidBrush(windows::Win32::Foundation::COLORREF(rgb)),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("rustshot_e2e_backdrop"),
                w!("rustshot e2e backdrop"),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                sw,
                sh,
                None,
                None,
                Some(inst.into()),
                None,
            )
            .expect("backdrop window");
            let _ = tx.send(GetCurrentThreadId());
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = DestroyWindow(hwnd);
        });
        let thread = rx.recv().expect("backdrop up");
        // Painted (its class brush) before the overlay takes its shot.
        std::thread::sleep(Duration::from_millis(500));
        Backdrop { thread, join: Some(join) }
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// The screen once it shows something other than `before` (the overlay
/// is drawn: a window can be visible a while before its first frame) and
/// then holds still for a whole second (reads every 150 ms: the fade-in
/// and any late re-layout are over); at most `ms`.
fn settled_pixels(x: i32, y: i32, w: i32, h: i32, before: &[u8], ms: u64) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut last = screen_pixels(x, y, w, h);
    while last == before && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        last = screen_pixels(x, y, w, h);
    }
    let mut since = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let now = screen_pixels(x, y, w, h);
        if now != last {
            since = Instant::now();
            last = now;
        } else if since.elapsed() >= Duration::from_secs(1) || Instant::now() >= deadline {
            return now;
        }
        if Instant::now() >= deadline {
            return last;
        }
    }
}

/// The overlay as shown on screen is the same with either renderer:
/// open it, read the screen back, compare. The desktop behind is a still
/// backdrop of one color (so each run's screenshot is the same), and each
/// read waits for the screen to settle (no fade-in frame), so the two
/// differ only where the renderers do. Run against a release build
/// (`RUSTSHOT_E2E_BIN`) for speed. Writes both captures as PNGs to
/// `RUSTSHOT_E2E_OUT` when set.
#[test]
#[ignore = "live display access"]
fn renderers_show_the_same_overlay() {
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let backdrop = Backdrop::new(0x0080_6040);
    let (x, y, w, h) = (0, 0, 1400, 900);
    let mut shots = Vec::new();
    for renderer in ["software", "gdi"] {
        let before = screen_pixels(x, y, w, h);
        let root = config_root(&format!("same-{renderer}"), renderer);
        let mut child = rustshot(&root)
            .args(["gui", "--region", "800x600+200+150", "--clip"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rustshot");
        let hwnd = wait_window(&child, 8000).expect("editor window");
        mouse_move(hwnd, 600, 450);
        let shot = settled_pixels(x, y, w, h, &before, 10_000);
        assert!(shot != before, "{renderer}: the overlay never showed");
        shots.push(shot);
        key(hwnd, 0x1B);
        assert_eq!(wait_exit(&mut child, 5000), Some(2));
        // The overlay gone: the backdrop alone again before the next shot.
        std::thread::sleep(Duration::from_millis(300));
    }
    drop(backdrop);
    if let Some(dir) = std::env::var_os("RUSTSHOT_E2E_OUT") {
        for (name, px) in ["software", "gdi"].iter().zip(&shots) {
            let mut rgba = px.clone();
            for p in rgba.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
                p[3] = 255;
            }
            let f = std::fs::File::create(Path::new(&dir).join(format!("overlay-{name}.png"))).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgba).unwrap();
        }
    }
    let (a, b) = (&shots[0], &shots[1]);
    let (mut n, mut max, mut at) = (0usize, 0u8, (0, 0));
    for i in 0..(w * h) as usize {
        let d = (0..3).map(|c| a[i * 4 + c].abs_diff(b[i * 4 + c])).max().unwrap();
        if d > 2 {
            n += 1;
        }
        if d > max {
            max = d;
            at = ((i % w as usize) as i32, (i / w as usize) as i32);
        }
    }
    println!("pixels differing by more than 2 levels: {n} of {}, max {max} at {at:?}", w * h);
    // Rounding in the two blend paths: a level or two at most.
    assert!(n == 0, "overlays differ in {n} pixels (max {max} at {at:?})");
}

/// Whether this process can register `mods`+`vk` as a global hotkey now
/// (released right away): false while another process holds it.
fn hotkey_free(mods: u32, vk: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};
    unsafe {
        let ok = RegisterHotKey(None, 0x5e7, HOT_KEY_MODIFIERS(mods), vk).is_ok();
        if ok {
            let _ = UnregisterHotKey(None, 0x5e7);
        }
        ok
    }
}

/// Wait for the visible window of `pid` to go away.
fn wait_gone(pid: u32, ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while find_window(pid).is_some() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

fn wait_pid_window(pid: u32, ms: u64) -> Option<HWND> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if let Some(h) = find_window(pid) {
            return Some(h);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The daemon's other global hotkeys off (their defaults, Win+Shift+G /
/// F / E / Z, would take keys from the desktop while a test runs).
const NO_OTHER_HOTKEYS: &str =
    "record_hotkey = \"\"\ncapture_full_hotkey = \"\"\ncapture_window_hotkey = \"\"\nrepeat_hotkey = \"\"\ncapture_text_hotkey = \"\"\n";

/// A control id as the Settings window names it (`crate::ui::id_of`:
/// FNV-1a of the name).
fn ui_id(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The focused control the Settings window last reported
/// (`RUSTSHOT_TEST_FOCUS_FILE`): its id, `none`, or nothing yet.
fn reported_focus(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok()
}

/// Wait until the window has reported a focus other than `old`.
fn wait_focus_change(file: &Path, old: &Option<String>, ms: u64) -> Option<String> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        let now = reported_focus(file);
        if now.is_some() && now != *old {
            return now;
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Tab until control `name` has the focus, each press waited for (the
/// window reports its focus), so neither the rows before it nor the
/// machine's speed matter. Panics after `max` presses.
fn tab_to(hwnd: HWND, file: &Path, name: &str, max: usize) {
    const TAB: u32 = 0x09;
    let want = ui_id(name).to_string();
    let mut seen = reported_focus(file);
    for _ in 0..max {
        if seen.as_deref() == Some(want.as_str()) {
            return;
        }
        key(hwnd, TAB);
        seen = Some(wait_focus_change(file, &seen, 3000).unwrap_or_else(|| panic!("Tab moved no focus on the way to {name}")));
    }
    assert_eq!(seen.as_deref(), Some(want.as_str()), "{name} not reached in {max} Tab presses");
}

/// Keys posted one by one with a short pause (each is its own frame).
fn keys(hwnd: HWND, vks: &[u32]) {
    for &vk in vks {
        key(hwnd, vk);
        std::thread::sleep(Duration::from_millis(60));
    }
}

/// The Settings window in a private daemon (own instance name, config dir
/// and unusual hotkeys): Saving → JPG into another folder (with the
/// rewrite question), Shortcuts › Overlay actions → Accept on J (a key no
/// overlay action has; the rail keys are checked by `e2e_rail_keys` in
/// src/settings_ui/tests.rs); the daemon reloads (its capture hotkey
/// moves to the one now in the file), and a capture
/// accepted with J writes `<folder>/<date>/<name>.jpg`.
///
///     cargo test --test e2e settings_window -- --ignored --nocapture
#[test]
#[ignore = "live display access"]
fn settings_window_saves_and_the_daemon_reloads() {
    const RET: u32 = 0x0D;
    // Rail keys: General → Saving (Capture, Overlay, Saving); End (About), Up → Shortcuts › Overlay actions.
    const TO_SAVING: [u32; 3] = [0x28; 3];
    const TO_ACTIONS: [u32; 2] = [0x23, 0x26];
    const MODS: u32 = 1 | 2 | 4; // Alt+Ctrl+Shift
    const F9: u32 = 0x78;
    const F11: u32 = 0x7A;
    let root = config_root("settings", &renderer());
    let cfg = root.join("rustshot").join("config.toml");
    let shots = root.join("shots");
    let base = format!("check_updates = false\ncapture_hotkey = \"Ctrl+Alt+Shift+F11\"\nquit_hotkey = \"Ctrl+Alt+Shift+F10\"\n{NO_OTHER_HOTKEYS}");
    std::fs::write(&cfg, format!("# e2e\n{base}")).unwrap();
    let instance = format!("e2e-set-{}", std::process::id());
    // The Settings window reports its focused control here (by id).
    let focus = root.join("focus.txt");
    let run = |args: &[&str]| {
        let mut c = rustshot(&root);
        c.env("RUSTSHOT_INSTANCE", &instance).env("RUSTSHOT_TEST_FOCUS_FILE", &focus).args(args).stdout(Stdio::null()).stderr(Stdio::null());
        c
    };
    let mut daemon = run(&["daemon"]).spawn().expect("spawn daemon");
    let pid = daemon.id();
    println!("daemon pid {pid}");
    std::thread::sleep(Duration::from_millis(1500));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(!hotkey_free(MODS, F11), "the daemon holds its capture hotkey");
        // Session 1: Saving tab.
        let mut c = run(&["settings"]).spawn().unwrap();
        println!("settings pid {}", c.id());
        assert_eq!(wait_exit(&mut c, 5000), Some(0), "`rustshot settings` hands over to the daemon");
        let w = wait_pid_window(pid, 5000).expect("settings window in the daemon");
        std::thread::sleep(Duration::from_millis(500));
        // The rail → Saving (Capture, Overlay, Saving), then its Save to
        // field: End, clear, type.
        tab_to(w, &focus, "rail", 80);
        keys(w, &TO_SAVING);
        tab_to(w, &focus, "folder", 60);
        keys(w, &[0x23]);
        keys(w, &[0x08; 120]); // clear the folder
        for ch in shots.display().to_string().chars() {
            char_msg(w, ch);
        }
        std::thread::sleep(Duration::from_millis(200));
        // The format choice: PNG → JPG.
        tab_to(w, &focus, "format_all", 5);
        keys(w, &[0x27]);
        // Meanwhile the file gets a new capture hotkey (as if edited by hand):
        // the window's untouched values follow the file.
        std::fs::write(&cfg, format!("# e2e\n{}", base.replace("F11", "F9"))).unwrap();
        key(w, RET); // OK: asks to rewrite (the comment)
        std::thread::sleep(Duration::from_millis(300));
        assert!(find_window(pid).is_some(), "the question is up");
        key(w, RET); // Save
        assert!(wait_gone(pid, 5000), "OK closes the window");
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("save_format = \"jpg\""), "{text}");
        assert!(text.contains(&format!("save_path = {:?}", shots.display().to_string())), "{text}");
        assert!(text.contains("capture_hotkey = \"Ctrl+Alt+Shift+F9\"") && !text.contains("# e2e"), "{text}");
        std::thread::sleep(Duration::from_millis(500));
        assert!(hotkey_free(MODS, F11), "the old capture hotkey is released");
        assert!(!hotkey_free(MODS, F9), "the new one is registered");
        // Session 2: Shortcuts tab, Accept → J.
        let _ = std::fs::remove_file(&focus);
        assert_eq!(wait_exit(&mut run(&["settings"]).spawn().unwrap(), 5000), Some(0));
        let w = wait_pid_window(pid, 5000).expect("settings window again");
        std::thread::sleep(Duration::from_millis(500));
        // The rail → Shortcuts › Overlay actions; Accept's cap: record J; then OK.
        tab_to(w, &focus, "rail", 80);
        keys(w, &TO_ACTIONS);
        tab_to(w, &focus, "key_accept", 200);
        keys(w, &[RET, 'J' as u32]);
        tab_to(w, &focus, "ok", 200);
        keys(w, &[RET]);
        assert!(wait_gone(pid, 5000), "OK closes the window");
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("[shortcuts]\naccept = \"J\""), "{text}");
        std::thread::sleep(Duration::from_millis(500));
        // A capture in the daemon: click selects the screen, J accepts (saves).
        let _ = run(&[]).spawn().unwrap().wait();
        let o = wait_pid_window(pid, 8000).expect("overlay");
        std::thread::sleep(Duration::from_millis(800));
        mouse_move(o, 300, 300);
        mouse_down(o, 300, 300);
        mouse_up(o, 300, 300);
        std::thread::sleep(Duration::from_millis(300));
        key(o, 'J' as u32);
        assert!(wait_gone(pid, 8000), "J accepted the capture");
        std::thread::sleep(Duration::from_millis(1000));
        let mut found = Vec::new();
        for d in std::fs::read_dir(&shots).expect("save folder created").flatten() {
            for f in std::fs::read_dir(d.path()).unwrap().flatten() {
                found.push(f.path());
            }
        }
        println!("saved: {found:?}");
        assert_eq!(found.len(), 1, "{found:?}");
        let f = &found[0];
        let day = f.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        assert!(day.len() == 10 && day.as_bytes()[4] == b'-' && day.as_bytes()[7] == b'-', "{day}");
        assert_eq!(f.extension().unwrap(), "jpg");
        let b = std::fs::read(f).unwrap();
        assert_eq!(&b[..2], &[0xFF, 0xD8], "a JPEG");
    }));
    let _ = daemon.kill(); // only the daemon this test started
    let _ = daemon.wait();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// A tool's size changed with the wheel is written to the (isolated)
/// config when the overlay closes, without touching the rest of the file.
#[test]
#[ignore = "live display access"]
fn tool_size_is_remembered_in_the_config() {
    let (mut child, hwnd, root) = launch_editor("toolmem");
    let cfg = root.join("rustshot").join("config.toml");
    let before = std::fs::read_to_string(&cfg).unwrap();
    key(hwnd, 'L' as u32); // Line
    // WM_MOUSEWHEEL: one notch up (the delta in the high word), past the
    // wheel's 160 ms throttle from the capture's start.
    std::thread::sleep(Duration::from_millis(300));
    post(hwnd, 0x020A, WPARAM((120usize) << 16), pack(300, 300));
    std::thread::sleep(Duration::from_millis(100));
    key(hwnd, 0x1B); // the tool off
    key(hwnd, 0x1B); // cancel
    assert_eq!(wait_exit(&mut child, 5000), Some(2));
    let after = std::fs::read_to_string(&cfg).unwrap();
    assert!(after.starts_with(&before), "the file is kept: {after:?}");
    assert!(after.contains("tool_line_size = 4"), "{after:?}");
}
