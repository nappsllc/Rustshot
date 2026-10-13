//! The first-use dialog on Linux: "Recording MP4 needs an encoder", shown
//! when the user starts an MP4 recording and neither ffmpeg nor a
//! downloaded OpenH264 is there. **Download OpenH264 (Cisco, ~1 MB)**
//! downloads and checks it in the dialog (progress, Cancel, Retry) and
//! records with it; **Install ffmpeg** shows the distribution's command
//! (`apt`/`dnf`/`pacman`/`zypper`, from `/etc/os-release`) with a Copy
//! button; **Record GIF instead**; **Cancel**. The choice of encoder is
//! stored in `rec_linux_encoder` ("openh264" / "ffmpeg").
//!
//! As the update dialog: a pure [`Machine`] (unit-tested) drawn with the
//! `ui` kit in a `wind::run_window` window ([`ask`], Linux), the download
//! on a worker thread that reports through a channel.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::objects::FRect;
use crate::ui::Ui;
use std::path::PathBuf;

/// What the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// OpenH264 is downloaded and checked: record MP4 with it.
    DownloadOpenH264,
    /// They will install ffmpeg (nothing is recorded now).
    InstallFfmpeg,
    Gif,
    Cancel,
}

impl Choice {
    /// The `rec_linux_encoder` value the choice stores, if any.
    pub fn config_value(self) -> Option<&'static str> {
        match self {
            Choice::DownloadOpenH264 => Some("openh264"),
            Choice::InstallFfmpeg => Some("ffmpeg"),
            Choice::Gif | Choice::Cancel => None,
        }
    }
}

/// Window size, logical px.
pub const W: u32 = 520;
pub const H: u32 = 248;
const PAD: f32 = 20.0;
const ICON: f32 = 24.0;
const TITLE_H: f32 = 32.0;
const TEXT_X: f32 = ICON + 4.0 + crate::ui::layout::GAP;

pub const TITLE: &str = "Recording MP4 needs an encoder";
const ASK_TEXT: &str = "Rustshot records MP4 with ffmpeg when it is installed. Without it, Rustshot can download \
Cisco's OpenH264 and record with that (the sound is encoded with your system's Opus). Or record a GIF now.";
const FFMPEG_TEXT: &str = "Run this in a terminal, then start the recording again:";
const FFMPEG_UNKNOWN: &str = "Install ffmpeg with your distribution's package manager, then start the recording again.";
const FROM_CISCO: &str = "OpenH264 Video Codec provided by Cisco Systems, Inc.";

/// The `ID` and `ID_LIKE` words of an `/etc/os-release` file.
pub fn os_ids(os_release: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["ID", "ID_LIKE"] {
        for line in os_release.lines() {
            let Some(v) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix('=')) else { continue };
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            ids.extend(v.split_whitespace().map(|w| w.to_ascii_lowercase()));
        }
    }
    ids
}

/// The command that installs ffmpeg on the distribution `/etc/os-release`
/// describes (its own `ID` first, then the ones it is like); `None` when
/// it is none of the apt, dnf, pacman or zypper families.
pub fn install_command(os_release: &str) -> Option<&'static str> {
    os_ids(os_release).iter().find_map(|id| {
        Some(match id.as_str() {
            "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "raspbian" | "zorin" | "neon" | "kali" => "sudo apt install ffmpeg",
            // Fedora's own repositories have ffmpeg as "ffmpeg-free".
            "fedora" => "sudo dnf install ffmpeg-free",
            "rhel" | "centos" | "rocky" | "almalinux" | "nobara" => "sudo dnf install ffmpeg",
            "arch" | "manjaro" | "endeavouros" | "garuda" | "artix" | "cachyos" => "sudo pacman -S ffmpeg",
            "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "suse" | "sles" => "sudo zypper install ffmpeg",
            _ => return None,
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Download,
    Ffmpeg,
    Gif,
    Cancel,
    Copy,
    Back,
    Done,
    Retry,
}

impl Btn {
    fn label(self, copied: bool) -> &'static str {
        match self {
            Btn::Download => "Download OpenH264 (Cisco, ~1 MB)",
            Btn::Ffmpeg => "Install ffmpeg",
            Btn::Gif => "Record GIF instead",
            Btn::Cancel => "Cancel",
            Btn::Copy if copied => "Copied",
            Btn::Copy => "Copy",
            Btn::Back => "Back",
            Btn::Done => "Done",
            Btn::Retry => "Retry",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Btn::Download => "download",
            Btn::Ffmpeg => "ffmpeg",
            Btn::Gif => "gif",
            Btn::Cancel => "cancel",
            Btn::Copy => "copy",
            Btn::Back => "back",
            Btn::Done => "done",
            Btn::Retry => "retry",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Screen {
    Ask,
    Ffmpeg,
    Downloading { got: u64, total: Option<u64> },
    Failed(String),
}

/// Input to the [`Machine`].
#[derive(Debug)]
pub enum In {
    Click(Btn),
    /// Esc or the close button.
    Dismiss,
    /// Download attempt `.0` got `.1` bytes of `.2`.
    Progress(u64, u64, Option<u64>),
    /// Download attempt `.0` ended (the library's path, or why not).
    /// Reports of any attempt but the current one are ignored.
    Downloaded(u64, Result<PathBuf, String>),
}

/// What the driver must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Start download attempt `.0` (numbered from 1).
    Download(u64),
    CancelDownload,
    Close(Choice),
}

/// The dialog's state machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    screen: Screen,
    /// The ffmpeg command for this distribution.
    cmd: Option<&'static str>,
    copied: bool,
    /// The newest download attempt (0: none yet).
    attempt: u64,
}

/// Download errors that mean the user cancelled.
const CANCELLED: &str = crate::update::CANCELLED;

impl Machine {
    pub fn new(cmd: Option<&'static str>) -> Machine {
        Machine { screen: Screen::Ask, cmd, copied: false, attempt: 0 }
    }

    /// The buttons: (the row under the text, the bottom row), each left
    /// to right, and the primary one.
    fn buttons(&self) -> (&'static [Btn], &'static [Btn], Option<Btn>) {
        use Btn::*;
        match (&self.screen, self.cmd.is_some()) {
            (Screen::Ask, _) => (&[Download, Ffmpeg], &[Gif, Cancel], Some(Download)),
            (Screen::Ffmpeg, true) => (&[Copy], &[Back, Done], Some(Done)),
            (Screen::Ffmpeg, false) => (&[], &[Back, Done], Some(Done)),
            (Screen::Downloading { .. }, _) => (&[], &[Cancel], None),
            (Screen::Failed(_), _) => (&[], &[Gif, Cancel, Retry], Some(Retry)),
        }
    }

    pub fn on(&mut self, ev: In) -> Vec<Act> {
        use Screen as S;
        // A cancelled attempt reporting late.
        if let In::Progress(n, ..) | In::Downloaded(n, _) = ev
            && n != self.attempt
        {
            return vec![];
        }
        let screen = std::mem::replace(&mut self.screen, S::Ask);
        let (next, acts) = match (screen, ev) {
            (S::Ask | S::Failed(_), In::Click(Btn::Download | Btn::Retry)) => {
                self.attempt += 1;
                (S::Downloading { got: 0, total: None }, vec![Act::Download(self.attempt)])
            }
            (S::Ask, In::Click(Btn::Ffmpeg)) => {
                self.copied = false;
                (S::Ffmpeg, vec![])
            }
            (S::Ask | S::Failed(_), In::Click(Btn::Gif)) => (S::Ask, vec![Act::Close(Choice::Gif)]),
            (S::Ask | S::Failed(_), In::Click(Btn::Cancel) | In::Dismiss) => (S::Ask, vec![Act::Close(Choice::Cancel)]),
            (S::Ffmpeg, In::Click(Btn::Copy)) => {
                self.copied = true;
                (S::Ffmpeg, vec![])
            }
            (S::Ffmpeg, In::Click(Btn::Back)) => (S::Ask, vec![]),
            (S::Ffmpeg, In::Click(Btn::Done)) => (S::Ffmpeg, vec![Act::Close(Choice::InstallFfmpeg)]),
            (S::Ffmpeg, In::Dismiss) => (S::Ffmpeg, vec![Act::Close(Choice::Cancel)]),
            (S::Downloading { .. }, In::Progress(_, got, total)) => (S::Downloading { got, total }, vec![]),
            (S::Downloading { .. }, In::Click(Btn::Cancel) | In::Dismiss) => (S::Ask, vec![Act::CancelDownload]),
            (S::Downloading { .. }, In::Downloaded(_, Ok(_))) => (S::Ask, vec![Act::Close(Choice::DownloadOpenH264)]),
            (S::Downloading { .. }, In::Downloaded(_, Err(e))) if e == CANCELLED => (S::Ask, vec![]),
            (S::Downloading { .. }, In::Downloaded(_, Err(e))) => (S::Failed(e), vec![]),
            // The current attempt's report after it was cancelled.
            (s, In::Downloaded(..) | In::Progress(..)) => (s, vec![]),
            (s, _) => (s, vec![]),
        };
        self.screen = next;
        acts
    }

    /// A download is running.
    pub fn busy(&self) -> bool {
        matches!(self.screen, Screen::Downloading { .. })
    }
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_000_000.0)
}

/// Lay out a right-aligned row of `btns` in `r`; the clicked one.
fn button_row(ui: &mut Ui, r: FRect, btns: &[Btn], primary: Option<Btn>, copied: bool, left: bool) -> Option<Btn> {
    let gap = crate::ui::layout::GAP;
    let total: f32 = btns.iter().map(|b| ui.button_width(b.label(copied))).sum::<f32>() + gap * (btns.len().max(1) as f32 - 1.0);
    let mut clicked = None;
    ui.area(r, |ui| {
        ui.row(|ui| {
            if !left {
                ui.space((r.w - total).max(0.0));
            }
            for &b in btns {
                if ui.button(b.id(), b.label(copied), Some(b) == primary) {
                    clicked = Some(b);
                }
            }
        })
    });
    clicked
}

/// One frame of the dialog; the clicked button (Copy has copied already).
pub fn paint(ui: &mut Ui, m: &Machine) -> Option<Btn> {
    let th = ui.theme;
    let b = ui.bounds();
    let inner = b.w - 2.0 * PAD;
    let text_w = inner - TEXT_X;
    let h = crate::ui::layout::H;
    let (icon, col, title, text): (&str, _, String, Option<String>) = match &m.screen {
        Screen::Ask => ("video", th.accent, TITLE.into(), Some(ASK_TEXT.into())),
        Screen::Ffmpeg => (
            "video",
            th.accent,
            "Install ffmpeg".into(),
            Some(if m.cmd.is_some() { FFMPEG_TEXT } else { FFMPEG_UNKNOWN }.into()),
        ),
        Screen::Downloading { .. } => ("video", th.accent, "Downloading OpenH264…".into(), Some(FROM_CISCO.into())),
        Screen::Failed(e) => ("alert", th.error, "Could not get OpenH264".into(), Some(e.clone())),
    };
    let title_r = FRect { x: PAD, y: PAD, w: inner, h: TITLE_H };
    ui.area(title_r, |ui| {
        ui.row(|ui| {
            ui.icon(icon, ICON, col);
            ui.space(4.0);
            ui.heading(&title);
        })
    });
    let mut y = title_r.y1() + 4.0;
    if let Some(t) = &text {
        let th_ = ui.paragraph_height(t, text_w).min(72.0);
        let r = FRect { x: PAD + TEXT_X, y, w: text_w, h: th_ };
        ui.area(r, |ui| ui.height(r.h).paragraph(t, true));
        y = r.y1() + 14.0;
    }
    let (row, bottom, primary) = m.buttons();
    let mut clicked = None;
    match &m.screen {
        Screen::Ffmpeg => {
            if let Some(cmd) = m.cmd {
                let copy_w = ui.button_width(Btn::Copy.label(true));
                // One whole line: no scroll bar.
                let box_h = crate::ui::controls::text_view_fit(h, 1);
                let r = FRect { x: PAD + TEXT_X, y, w: text_w - copy_w - crate::ui::layout::GAP, h: box_h };
                let mut scroll = 0.0;
                ui.place(r).text_view_styled("cmd", &[(cmd, false)], &mut scroll);
                let br = FRect { x: r.x1() + crate::ui::layout::GAP, y: y + ((box_h - h) / 2.0).round(), w: copy_w, h };
                if let Some(c) = button_row(ui, br, row, None, m.copied, true) {
                    ui.clip.set(cmd);
                    clicked = Some(c);
                }
            }
        }
        Screen::Downloading { got, total } => {
            let s = match total {
                Some(t) => format!("Downloading… {} of {}", mb(*got), mb(*t)),
                None => "Downloading…".into(),
            };
            let f = total.map_or(0.0, |t| (*got as f32 / t.max(1) as f32).min(1.0));
            let r = FRect { x: PAD + TEXT_X, y, w: text_w, h: 32.0 };
            ui.area(r, |ui| {
                ui.lay.gap = 6.0;
                ui.note(&s);
                ui.progress(f);
            });
        }
        _ => {
            if !row.is_empty() {
                let r = FRect { x: PAD + TEXT_X, y, w: text_w, h };
                clicked = button_row(ui, r, row, primary, m.copied, true);
            }
        }
    }
    let br = FRect { x: PAD, y: b.h - PAD - h, w: inner, h };
    let bottom_primary = primary.filter(|p| bottom.contains(p));
    clicked.or(button_row(ui, br, bottom, bottom_primary, m.copied, false))
}

#[cfg(target_os = "linux")]
pub use imp::ask;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::pixbuf::PixBuf;
    use crate::rec::linux::openh264;
    use crate::theme::Theme;
    use crate::ui::{FocusState, Input};
    use crate::wind::{self, Cursor, Driver, Ev, Hwnd, WindowSpec};
    use std::sync::atomic::{AtomicBool, Ordering};
    use crate::chan::{Receiver, Sender};
    use std::sync::Arc;

    struct Dialog {
        hwnd: Hwnd,
        m: Machine,
        th: Theme,
        input: Input,
        focus: FocusState,
        canvas: PixBuf,
        out: PixBuf,
        tx: Sender<In>,
        rx: Receiver<In>,
        cancel: Arc<AtomicBool>,
        choice: Option<Choice>,
    }

    impl Dialog {
        fn new(cmd: Option<&'static str>, th: Theme) -> Dialog {
            let (tx, rx) = crate::chan::channel();
            Dialog {
                hwnd: Hwnd::default(),
                m: Machine::new(cmd),
                th,
                input: Input::default(),
                focus: FocusState::default(),
                canvas: PixBuf::default(),
                out: PixBuf::default(),
                tx,
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                choice: None,
            }
        }

        fn perform(&mut self, acts: Vec<Act>) {
            for a in acts {
                match a {
                    Act::Download(n) => {
                        // Each attempt has its own cancel flag and file.
                        self.cancel = Arc::new(AtomicBool::new(false));
                        let (tx, cancel) = (self.tx.clone(), self.cancel.clone());
                        std::thread::spawn(move || {
                            let last = std::cell::Cell::new(0u64);
                            let r = openh264::download(n, &|got, total| {
                                if got < last.get() || got - last.get() >= 32 * 1024 || total == Some(got) {
                                    last.set(got);
                                    let _ = tx.send(In::Progress(n, got, total));
                                }
                                !cancel.load(Ordering::SeqCst)
                            });
                            let _ = tx.send(In::Downloaded(n, r));
                        });
                    }
                    Act::CancelDownload => self.cancel.store(true, Ordering::SeqCst),
                    Act::Close(c) => self.choice = Some(c),
                }
            }
        }

        fn feed(&mut self, ev: In) {
            let acts = self.m.on(ev);
            self.perform(acts);
        }

        fn render(&mut self) {
            let k = wind::scale(self.hwnd);
            for _ in 0..3 {
                if self.choice.is_some() || self.canvas.width() == 0 {
                    return;
                }
                let (w, h) = self.canvas.dimensions();
                let bg = self.th.surface.with_alpha(255);
                for p in self.canvas.as_raw_mut().as_chunks_mut::<4>().0 {
                    *p = [bg.r, bg.g, bg.b, 255];
                }
                let area = FRect { x: 0.0, y: 0.0, w: w as f32 / k, h: h as f32 / k };
                let mut clip = crate::ui::SystemClipboard;
                let fb = crate::uifb::Fb::new(self.canvas.as_raw_mut(), w as usize);
                let mut ui = Ui::new(fb, &self.th, &self.input, &mut self.focus, &mut clip, k, area);
                let clicked = paint(&mut ui, &self.m);
                let out = ui.finish();
                self.input.end_frame();
                let ev = match clicked {
                    Some(b) => Some(In::Click(b)),
                    None if out.escape => Some(In::Dismiss),
                    _ => None,
                };
                match ev {
                    Some(ev) => self.feed(ev),
                    None if !out.redraw => return,
                    None => {}
                }
            }
        }

        fn settle(&mut self) {
            if self.choice.is_some() {
                wind::close(self.hwnd);
            }
        }
    }

    impl Driver for Dialog {
        fn on_create(&mut self, hwnd: Hwnd) {
            self.hwnd = hwnd;
        }

        fn on_event(&mut self, ev: Ev) -> bool {
            match ev {
                Ev::Resize(w, h) => {
                    self.canvas = PixBuf::new(w.max(1), h.max(1));
                    self.render();
                }
                Ev::Close => self.feed(In::Dismiss),
                Ev::Timer => {
                    let evs: Vec<In> = self.rx.try_iter().collect();
                    let any = !evs.is_empty();
                    for e in evs {
                        self.feed(e);
                    }
                    if any {
                        self.render();
                    }
                    self.settle();
                    return any && self.choice.is_none();
                }
                Ev::Move { .. } => {
                    self.input.feed(&ev);
                    self.render();
                    if self.choice.is_none() {
                        wind::invalidate(self.hwnd);
                    }
                }
                _ => {
                    self.input.feed(&ev);
                    self.render();
                }
            }
            self.settle();
            self.choice.is_none()
        }

        fn frame(&mut self) -> Option<&mut PixBuf> {
            if self.canvas.width() == 0 || self.choice.is_some() {
                return None;
            }
            if self.out.dimensions() != self.canvas.dimensions() {
                self.out = PixBuf::new(self.canvas.width(), self.canvas.height());
            }
            self.out.as_raw_mut().copy_from_slice(self.canvas.as_raw());
            Some(&mut self.out)
        }

        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }

        fn frame_colors(&self) -> Option<wind::FrameColors> {
            Some(crate::ui::paint::WTok::of(&self.th).frame())
        }

        fn on_quit(&mut self) {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Show the dialog (blocks until it closes) and store the chosen
    /// encoder in the config.
    pub fn ask() -> Choice {
        let cfg = crate::config::load();
        let th = crate::theme::resolve(&cfg);
        let os = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).unwrap_or_default();
        let mut d = Dialog::new(install_command(&os), th);
        let spec = WindowSpec { title: "Rustshot".into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::Close };
        if let Err(e) = wind::run_window(spec, &mut d) {
            eprintln!("rustshot: encoder dialog: {e:#}");
        }
        d.cancel.store(true, Ordering::SeqCst);
        let choice = d.choice.unwrap_or(Choice::Cancel);
        if let Some(v) = choice.config_value() {
            let path = crate::config::config_path();
            if let Err(e) = crate::config::save_top_key_at(&path, "rec_linux_encoder", v) {
                eprintln!("rustshot: cannot save rec_linux_encoder to {}: {e}", path.display());
            }
        }
        choice
    }

    #[cfg(test)]
    mod window {
        use super::*;

        /// Smoke (CI runs it under Xvfb): the real dialog opens, draws and
        /// closes after about a second.
        #[test]
        #[ignore = "opens a window"]
        fn encoder_dialog_window_smoke() {
            if !wind::has_display() {
                eprintln!("no display; skipped");
                return;
            }
            let _guard = wind::test_window_lock();
            let mut dlg = Dialog::new(Some("sudo apt install ffmpeg"), crate::theme::resolve(&crate::config::Config::default()));
            let mut d = wind::AutoClose::new(&mut dlg, 1000);
            let spec = WindowSpec { title: "Rustshot".into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::Close };
            wind::run_window(spec, &mut d).expect("encoder dialog");
            assert!(d.frames > 0, "drew a frame");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{preview, FocusState, Input};

    #[test]
    fn distro_commands_from_os_release() {
        let ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
        assert_eq!(install_command(ubuntu), Some("sudo apt install ffmpeg"));
        assert_eq!(install_command("ID=debian\n"), Some("sudo apt install ffmpeg"));
        assert_eq!(install_command("ID=fedora\n"), Some("sudo dnf install ffmpeg-free"));
        assert_eq!(install_command("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"), Some("sudo dnf install ffmpeg"));
        assert_eq!(install_command("ID=arch\n"), Some("sudo pacman -S ffmpeg"));
        assert_eq!(install_command("ID=endeavouros\nID_LIKE=arch\n"), Some("sudo pacman -S ffmpeg"));
        assert_eq!(install_command("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), Some("sudo zypper install ffmpeg"));
        // Unknown, but like a known one.
        assert_eq!(install_command("ID=mydistro\nID_LIKE=\"ubuntu debian\"\n"), Some("sudo apt install ffmpeg"));
        assert_eq!(install_command("ID=nixos\n"), None);
        assert_eq!(install_command(""), None);
        // VERSION_ID / ID-like keys are not ID.
        assert_eq!(install_command("VERSION_ID=arch\nIDX=arch\n"), None);
    }

    #[test]
    fn choices_store_the_encoder() {
        assert_eq!(Choice::DownloadOpenH264.config_value(), Some("openh264"));
        assert_eq!(Choice::InstallFfmpeg.config_value(), Some("ffmpeg"));
        assert_eq!(Choice::Gif.config_value(), None);
        assert_eq!(Choice::Cancel.config_value(), None);
    }

    #[test]
    fn download_then_record_with_openh264() {
        let mut m = Machine::new(Some("sudo apt install ffmpeg"));
        assert_eq!(m.on(In::Click(Btn::Download)), vec![Act::Download(1)]);
        assert!(m.busy());
        assert_eq!(m.on(In::Progress(1, 1000, Some(4000))), vec![]);
        assert_eq!(m.screen, Screen::Downloading { got: 1000, total: Some(4000) });
        assert_eq!(m.on(In::Downloaded(1, Ok("/x".into()))), vec![Act::Close(Choice::DownloadOpenH264)]);
    }

    #[test]
    fn a_failed_download_can_be_retried_or_given_up() {
        let mut m = Machine::new(None);
        m.on(In::Click(Btn::Download));
        assert_eq!(m.on(In::Downloaded(1, Err("bzip2 is needed to unpack OpenH264".into()))), vec![]);
        assert!(matches!(&m.screen, Screen::Failed(e) if e.contains("bzip2")));
        assert_eq!(m.on(In::Click(Btn::Retry)), vec![Act::Download(2)]);
        m.on(In::Downloaded(2, Err("curl: 404".into())));
        assert_eq!(m.on(In::Click(Btn::Gif)), vec![Act::Close(Choice::Gif)]);
        let mut m = Machine::new(None);
        m.on(In::Click(Btn::Download));
        m.on(In::Downloaded(1, Err("x".into())));
        assert_eq!(m.on(In::Dismiss), vec![Act::Close(Choice::Cancel)]);
    }

    #[test]
    fn cancelling_a_download_goes_back_to_the_question() {
        let mut m = Machine::new(None);
        m.on(In::Click(Btn::Download));
        assert_eq!(m.on(In::Click(Btn::Cancel)), vec![Act::CancelDownload]);
        assert_eq!(m.screen, Screen::Ask);
        // The worker's late report changes nothing.
        assert_eq!(m.on(In::Downloaded(1, Err(CANCELLED.into()))), vec![]);
        assert_eq!(m.on(In::Downloaded(1, Ok("/x".into()))), vec![]);
        assert_eq!(m.screen, Screen::Ask);
        let mut m = Machine::new(None);
        m.on(In::Click(Btn::Download));
        assert_eq!(m.on(In::Dismiss), vec![Act::CancelDownload], "Esc cancels the download, not the dialog");
    }

    #[test]
    fn a_cancelled_attempt_reporting_late_does_not_touch_the_next_one() {
        let mut m = Machine::new(None);
        assert_eq!(m.on(In::Click(Btn::Download)), vec![Act::Download(1)]);
        m.on(In::Click(Btn::Cancel));
        assert_eq!(m.on(In::Click(Btn::Download)), vec![Act::Download(2)]);
        // Attempt 1's worker winds down while attempt 2 runs.
        assert_eq!(m.on(In::Progress(1, 600_000, Some(634_264))), vec![]);
        assert_eq!(m.screen, Screen::Downloading { got: 0, total: None }, "not attempt 1's progress");
        assert_eq!(m.on(In::Downloaded(1, Err(CANCELLED.into()))), vec![]);
        assert!(m.busy(), "attempt 1's cancel does not end attempt 2");
        assert_eq!(m.on(In::Downloaded(1, Err("curl: (6)".into()))), vec![]);
        assert!(m.busy(), "nor does its failure");
        assert_eq!(m.on(In::Downloaded(1, Ok("/x".into()))), vec![], "nor its success");
        assert_eq!(m.on(In::Progress(2, 1000, Some(4000))), vec![]);
        assert_eq!(m.screen, Screen::Downloading { got: 1000, total: Some(4000) });
        assert_eq!(m.on(In::Downloaded(2, Ok("/x".into()))), vec![Act::Close(Choice::DownloadOpenH264)]);
    }

    #[test]
    fn install_ffmpeg_shows_the_command_and_copies_it() {
        let mut m = Machine::new(Some("sudo pacman -S ffmpeg"));
        assert_eq!(m.on(In::Click(Btn::Ffmpeg)), vec![]);
        assert_eq!(m.screen, Screen::Ffmpeg);
        assert_eq!(m.buttons().0, &[Btn::Copy]);
        m.on(In::Click(Btn::Copy));
        assert!(m.copied);
        assert_eq!(m.on(In::Click(Btn::Back)), vec![]);
        assert_eq!(m.screen, Screen::Ask);
        m.on(In::Click(Btn::Ffmpeg));
        assert!(!m.copied, "a fresh Copy");
        assert_eq!(m.on(In::Click(Btn::Done)), vec![Act::Close(Choice::InstallFfmpeg)]);
        let mut m = Machine::new(None);
        m.on(In::Click(Btn::Ffmpeg));
        assert!(m.buttons().0.is_empty(), "no command to copy");
        assert_eq!(m.on(In::Dismiss), vec![Act::Close(Choice::Cancel)]);
    }

    #[test]
    fn gif_and_cancel_from_the_question() {
        let mut m = Machine::new(None);
        assert_eq!(m.on(In::Click(Btn::Gif)), vec![Act::Close(Choice::Gif)]);
        let mut m = Machine::new(None);
        assert_eq!(m.on(In::Click(Btn::Cancel)), vec![Act::Close(Choice::Cancel)]);
        let mut m = Machine::new(None);
        assert_eq!(m.on(In::Dismiss), vec![Act::Close(Choice::Cancel)]);
    }

    fn screens() -> Vec<(&'static str, Machine)> {
        let at = |screen, cmd, copied| Machine { screen, cmd, copied, attempt: 0 };
        vec![
            ("ask", at(Screen::Ask, Some("sudo apt install ffmpeg"), false)),
            ("ffmpeg", at(Screen::Ffmpeg, Some("sudo apt install ffmpeg"), false)),
            ("ffmpeg-copied", at(Screen::Ffmpeg, Some("sudo dnf install ffmpeg-free"), true)),
            ("ffmpeg-unknown", at(Screen::Ffmpeg, None, false)),
            ("downloading", at(Screen::Downloading { got: 312_000, total: Some(634_264) }, None, false)),
            ("failed", at(Screen::Failed(crate::rec::linux::openh264::NO_BZIP2.into()), None, false)),
            (
                "failed-long",
                at(
                    Screen::Failed("curl: (6) Could not resolve host: ciscobinary.openh264.org; check the network connection and try again".into()),
                    None,
                    false,
                ),
            ),
        ]
    }

    fn render(m: &Machine, th: &crate::theme::Theme, k: f32, input: &Input) -> (crate::pixbuf::PixBuf, Option<Btn>) {
        let mut focus = FocusState::default();
        preview::render(W, H, k, th, &Input::default(), &mut focus, |ui| {
            paint(ui, m);
        });
        let mut clicked = None;
        let img = preview::render(W, H, k, th, input, &mut focus, |ui| clicked = paint(ui, m));
        (img, clicked)
    }

    #[test]
    fn preview_encoder_dialog_pngs() {
        for (tname, th) in [("dark", &crate::theme::DARK), ("light", &crate::theme::LIGHT)] {
            for (name, m) in screens() {
                let (img, _) = render(&m, th, 1.0, &Input::default());
                let bg = th.surface;
                assert!(img.as_raw().as_chunks::<4>().0.iter().any(|p| p[..3] != [bg.r, bg.g, bg.b]), "{name}");
                preview::save(&format!("encoder-{name}-{tname}.png"), &img);
            }
        }
        for (name, m) in screens().into_iter().filter(|(n, _)| ["ask", "ffmpeg"].contains(n)) {
            let (img, _) = render(&m, &crate::theme::DARK, 1.5, &Input::default());
            preview::save(&format!("encoder-{name}-dark-150.png"), &img);
        }
    }
}
