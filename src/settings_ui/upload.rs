//! Settings → **Upload** (the destination: Imgur or an S3-compatible
//! bucket, and what happens after an upload) and **S3 storage** (the
//! bucket's settings: Amazon S3, Cloudflare R2, MinIO, …, and a Test
//! connection).
//!
//! [`UpForm`] is the part of the settings form this tab edits (the
//! `upload_*`, `s3_*` keys and `copy_url_after_upload`). The secret access
//! key is typed here but never goes to `config.toml`: on Apply or OK it
//! is kept in the OS credential store ([`crate::keystore`]) under the
//! bucket's name, and the field empties again. [`State`] holds whether
//! one is stored (asked once per bucket name) and the Test connection's
//! worker and result.

use super::draw::{srow, srow_ex, FIELD_W, ROW_H};
use super::Click;
use crate::config::Config;
use crate::ui::compact::Tok;
use crate::ui::TextState;
use crate::ui::Ui;
use crate::chan::Receiver;

/// (label, config value) of the destinations.
pub const DESTS: [(crate::i18n::Key, &str); 2] = [(key!("settings.upload.imgur"), "imgur"), (key!("settings.upload.s3"), "s3")];
pub const S3: usize = 1;

/// The values this tab edits.
#[derive(Clone, PartialEq)]
pub struct UpForm {
    /// [`DESTS`] index.
    pub dest: usize,
    pub client_id: TextState,
    pub endpoint: TextState,
    pub region: TextState,
    pub bucket: TextState,
    pub access_key: TextState,
    /// Typed, not yet stored (empty: leave the stored one).
    pub secret: TextState,
    pub public_url: TextState,
    pub prefix: TextState,
    pub path_style: bool,
    pub copy_url: bool,
    pub auto_rec: bool,
}

impl std::fmt::Debug for UpForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret never reaches a log or a test failure.
        f.debug_struct("UpForm")
            .field("dest", &self.dest)
            .field("endpoint", &self.endpoint.text)
            .field("bucket", &self.bucket.text)
            .field("access_key", &self.access_key.text)
            .field("secret", &if self.secret.text.is_empty() { "none" } else { "typed (hidden)" })
            .finish_non_exhaustive()
    }
}

impl UpForm {
    pub fn new(c: &Config) -> UpForm {
        UpForm {
            dest: if crate::s3::Dest::of(c) == crate::s3::Dest::S3 { S3 } else { 0 },
            client_id: TextState::new(&c.upload_client_id),
            endpoint: TextState::new(&c.s3_endpoint),
            region: TextState::new(&c.s3_region),
            bucket: TextState::new(&c.s3_bucket),
            access_key: TextState::new(&c.s3_access_key_id),
            secret: TextState::default(),
            public_url: TextState::new(&c.s3_public_url),
            prefix: TextState::new(&c.s3_key_prefix),
            path_style: c.s3_path_style,
            copy_url: c.copy_url_after_upload,
            auto_rec: c.upload_recordings,
        }
    }

    /// Write what changed from `was` into `c` (text trimmed: a key or a
    /// URL never starts or ends with a space). The secret is not here.
    pub fn apply(&self, was: &UpForm, c: &mut Config) {
        if self.dest != was.dest {
            c.upload_destination = DESTS[self.dest.min(DESTS.len() - 1)].1.into();
        }
        let text = |now: &TextState, then: &TextState, to: &mut String| {
            if now.text.trim() != then.text.trim() {
                *to = now.text.trim().to_string();
            }
        };
        text(&self.client_id, &was.client_id, &mut c.upload_client_id);
        text(&self.endpoint, &was.endpoint, &mut c.s3_endpoint);
        text(&self.region, &was.region, &mut c.s3_region);
        text(&self.bucket, &was.bucket, &mut c.s3_bucket);
        text(&self.access_key, &was.access_key, &mut c.s3_access_key_id);
        text(&self.public_url, &was.public_url, &mut c.s3_public_url);
        text(&self.prefix, &was.prefix, &mut c.s3_key_prefix);
        if self.path_style != was.path_style {
            c.s3_path_style = self.path_style;
        }
        if self.copy_url != was.copy_url {
            c.copy_url_after_upload = self.copy_url;
        }
        if self.auto_rec != was.auto_rec {
            c.upload_recordings = self.auto_rec;
        }
    }

    /// The bucket as the form has it now.
    pub fn target(&self) -> crate::s3::Target {
        crate::s3::Target {
            endpoint: self.endpoint.text.trim().into(),
            region: self.region.text.trim().into(),
            bucket: self.bucket.text.trim().into(),
            path_style: self.path_style,
        }
    }
}

/// Where a secret is kept: (endpoint host, bucket, access key id); see
/// [`crate::keystore::s3_name`].
pub type Slot = (String, String, String);

impl UpForm {
    /// The slot the form's values name, when they name one.
    pub fn slot(&self) -> Option<Slot> {
        slot_of(&self.target(), &self.access_key.text)
    }
}

/// The slot of `target` and `key_id` (None while the bucket or the key
/// id is blank).
pub fn slot_of(target: &crate::s3::Target, key_id: &str) -> Option<Slot> {
    let (bucket, key_id) = (target.bucket.trim(), key_id.trim());
    (!bucket.is_empty() && !key_id.is_empty()).then(|| (target.host(), bucket.to_string(), key_id.to_string()))
}

/// Keep `secret` (trimmed: pasted ones often carry a space or a line
/// break, which no S3 secret has) in `store` under `slot`.
pub fn store_secret(store: &dyn crate::keystore::Store, slot: &Slot, secret: &str) -> Result<(), String> {
    let secret = secret.trim();
    if secret.is_empty() {
        return Ok(());
    }
    let (host, bucket, key_id) = slot;
    store
        .set(&crate::keystore::s3_name(host, bucket, key_id), &crate::keystore::s3_label(bucket), secret)
        .map_err(|e| tr!("settings.s3.keep_failed_in", place = crate::keystore::place(), e))
}

/// What Test connection said.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Running,
    Passed,
    Failed(String),
}

/// How long a slot must stay as typed before the store is asked about it.
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// A secret being saved: its slot, the secret (back in the field if it
/// fails), whether OK closes the window after, the worker.
struct Saving {
    slot: Slot,
    secret: String,
    close: bool,
    rx: Receiver<Result<(), String>>,
}

/// What [`State::poll`] found.
#[derive(Debug, Default, PartialEq)]
pub struct Polled {
    /// Something to repaint.
    pub changed: bool,
    /// A save ended: its result, the secret, whether to close now.
    pub saved: Option<(Result<(), String>, String, bool)>,
}

/// The tab's state besides the form. Nothing here touches the credential
/// store on the window's thread: the stored-secret check and the save
/// run on workers (a locked Linux keyring asks for its password, which
/// can take a person a while), answered through [`State::poll`].
#[derive(Default)]
pub struct State {
    /// What the store said per slot (`None`: asking).
    stored: Vec<(Slot, Option<bool>)>,
    asks: Vec<(Slot, Receiver<bool>)>,
    /// The slot the form showed last, and since when.
    seen: Option<(Slot, std::time::Instant)>,
    saving: Option<Saving>,
    pub probe: Option<Probe>,
    rx: Option<Receiver<Result<(), String>>>,
}

fn answer<T>(rx: &Receiver<T>, gone: impl FnOnce() -> T) -> Option<T> {
    match rx.try_recv() {
        Ok(v) => Some(v),
        Err(crate::chan::TryRecvError::Empty) => None,
        Err(crate::chan::TryRecvError::Disconnected) => Some(gone()),
    }
}

impl State {
    /// Whether a secret is stored for `slot`, as far as known (`None`:
    /// not known yet). Drawing only notes the slot; [`State::poll`] asks
    /// the store once it has stayed the same for [`SETTLE`].
    pub fn stored(&mut self, slot: Option<Slot>) -> Option<bool> {
        let slot = slot?;
        if self.seen.as_ref().is_none_or(|(s, _)| *s != slot) {
            self.seen = Some((slot.clone(), std::time::Instant::now()));
        }
        self.stored.iter().find(|(s, _)| *s == slot).and_then(|(_, v)| *v)
    }

    /// Keep `secret` under `slot` on a worker; `close`: OK asked, close
    /// once it is kept.
    pub fn save(&mut self, slot: Slot, secret: String, close: bool) {
        let (tx, rx) = crate::chan::channel();
        let (sl, se) = (slot.clone(), secret.clone());
        let tx2 = tx.clone();
        if let Err(e) = std::thread::Builder::new().name("s3-secret".into()).spawn(move || {
            let _ = tx.send(store_secret(crate::keystore::os(), &sl, &se));
        }) {
            let _ = tx2.send(Err(tr!("settings.s3.keep_failed", e)));
        }
        self.saving = Some(Saving { slot, secret, close, rx });
    }

    /// A secret is being saved.
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// Test the form's bucket on a worker (never over the network in tests).
    pub fn start_test(&mut self, f: &UpForm) {
        if self.rx.is_some() {
            return;
        }
        let (target, key, secret) = (f.target(), f.access_key.text.trim().to_string(), f.secret.text.trim().to_string());
        self.probe = Some(Probe::Running);
        let (tx, rx) = crate::chan::channel();
        let spawned = std::thread::Builder::new().name("s3-test".into()).spawn(move || {
            let _ = tx.send(crate::s3::test_with(&target, &key, &secret, &crate::s3::PLATFORM));
        });
        match spawned {
            Ok(_) => self.rx = Some(rx),
            Err(e) => self.probe = Some(Probe::Failed(e.to_string())),
        }
    }

    /// Start a settled slot's check; collect what the workers said.
    pub fn poll(&mut self) -> Polled {
        let mut out = Polled::default();
        if let Some((slot, since)) = &self.seen
            && since.elapsed() >= SETTLE
            && !self.stored.iter().any(|(s, _)| s == slot)
        {
            let slot = slot.clone();
            let (tx, rx) = crate::chan::channel();
            let name = crate::keystore::s3_name(&slot.0, &slot.1, &slot.2);
            let spawned = std::thread::Builder::new().name("s3-secret-check".into()).spawn(move || {
                let _ = tx.send(crate::keystore::os().has(&name).unwrap_or(false));
            });
            if spawned.is_ok() {
                self.stored.push((slot.clone(), None));
                self.asks.push((slot, rx));
            }
        }
        let mut answered = Vec::new();
        self.asks.retain(|(slot, rx)| match answer(rx, || false) {
            Some(v) => {
                answered.push((slot.clone(), v));
                false
            }
            None => true,
        });
        for (slot, v) in answered {
            if let Some(e) = self.stored.iter_mut().find(|(s, _)| *s == slot) {
                e.1 = Some(v);
            }
            out.changed = true;
        }
        if let Some(r) = self.saving.as_ref().and_then(|sv| answer(&sv.rx, || Err(tr!("settings.s3.save_stopped").into()))) {
            let sv = self.saving.take().expect("saving");
            if r.is_ok() {
                self.stored.retain(|(s, _)| *s != sv.slot);
                self.stored.push((sv.slot.clone(), Some(true)));
            }
            out.saved = Some((r, sv.secret, sv.close));
            out.changed = true;
        }
        if let Some(r) = self.rx.as_ref().and_then(|rx| answer(rx, || Err(tr!("settings.s3.test_stopped").into()))) {
            self.rx = None;
            self.probe = Some(match r {
                Ok(()) => Probe::Passed,
                Err(e) => Probe::Failed(e),
            });
            out.changed = true;
        }
        out
    }

    /// Poll until every worker answered (tests).
    #[cfg(test)]
    pub fn wait(&mut self) -> Polled {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut all = Polled::default();
        loop {
            let p = self.poll();
            all.changed |= p.changed;
            if p.saved.is_some() {
                all.saved = p.saved;
            }
            if (self.rx.is_none() && self.saving.is_none() && self.asks.is_empty()) || std::time::Instant::now() > until {
                return all;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Count the slot last seen as settled (tests).
    #[cfg(test)]
    pub fn settle_now(&mut self) {
        if let Some((_, t)) = &mut self.seen
            && let Some(back) = t.checked_sub(SETTLE)
        {
            *t = back;
        }
    }
}

/// The secret row's helper line (`stored`: as far as known).
pub fn secret_help(typed: bool, stored: Option<bool>, saving: bool) -> String {
    let place = crate::keystore::place();
    match (saving, typed, stored) {
        (true, _, _) => tr!("settings.s3.secret.saving", place),
        (false, true, _) => tr!("settings.s3.secret.saved_on_apply", place),
        (false, false, Some(true)) => tr!("settings.s3.secret.stored", place),
        (false, false, Some(false)) => tr!("settings.s3.secret.not_stored", place),
        (false, false, None) => tr!("settings.s3.secret.kept", place),
    }
}

/// Shown under an empty Public URL base: the copied link is then the
/// bucket's own URL, which only opens for others when the bucket is public.
pub const PUBLIC_WARNING: crate::i18n::Key = key!("settings.s3.public_warning");

/// The Public URL row's helper line, and whether it warns.
pub fn public_help(base: &str) -> (&'static str, bool) {
    if base.trim().is_empty() { (PUBLIC_WARNING.get(), true) } else { (tr!("settings.s3.public_url.help"), false) }
}

/// The Test connection row's helper line (and whether it is an error).
pub fn probe_help(p: Option<&Probe>) -> (String, bool) {
    match p {
        None => (tr!("settings.s3.probe.idle").into(), false),
        Some(Probe::Running) => (tr!("settings.s3.probe.running").into(), false),
        Some(Probe::Passed) => (tr!("settings.s3.probe.passed").into(), false),
        Some(Probe::Failed(e)) => {
            let mut c = e.chars();
            let first = c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default();
            (format!("{first}{}", c.as_str()), true)
        }
    }
}

/// The Upload row that stands for the S3 settings: which bucket, at
/// which host (or that it is not set up yet).
pub fn s3_summary(f: &UpForm) -> String {
    let bucket = f.bucket.text.trim();
    let host = f.endpoint.text.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
    match (bucket.is_empty(), host.is_empty()) {
        (true, _) | (_, true) => tr!("settings.s3.not_set_up").into(),
        (false, false) => tr!("settings.s3.summary", bucket, host),
    }
}

/// The Imgur client ID field holds the shared built-in one: Upload says
/// so under it, with "Get a key". (An empty field turns Imgur uploads
/// off rather than using it.)
pub fn builtin_key(client_id: &str) -> bool {
    client_id.trim() == crate::config::BUILTIN_IMGUR_CLIENT_ID
}

/// Settings › Upload: where uploads go and what follows them; an S3
/// bucket's own settings are in [`s3_section`].
pub fn section(ui: &mut Ui, s: &mut super::Settings, tok: &Tok, click: &mut Option<Click>) {
    let mut open_s3 = false;
    let f = &mut s.form.up;
    ui.sec(tr!("settings.upload.destination"), tok);
    srow(ui, tok, tr!("settings.upload.upload_to"), None, |ui| {
        let labels: Vec<&str> = DESTS.iter().map(|d| d.0.get()).collect();
        ui.seg("upload_dest", &labels, &mut f.dest, tok);
    });
    if f.dest != S3 {
        srow(ui, tok, tr!("settings.upload.imgur_client_id"), Some(tr!("settings.upload.imgur_client_id.help")), |ui| {
            ui.width(FIELD_W).text_field("upload_client_id", &mut f.client_id);
        });
        if builtin_key(&f.client_id.text) && super::draw::note_link_line(ui, tok, "imgur_get_key", tr!("settings.upload.builtin_key"), tr!("settings.upload.get_key")) {
            *click = Some(Click::Link(crate::actions::IMGUR_ADD_CLIENT));
        }
    } else {
        let summary = s3_summary(f);
        srow(ui, tok, tr!("settings.upload.s3_storage"), Some(&summary), |ui| {
            open_s3 = ui.cbutton("s3_open", tr!("settings.upload.set_up"), None, false, tok);
        });
    }
    ui.sec(tr!("settings.upload.after_upload"), tok);
    srow(ui, tok, tr!("settings.upload.copy_link_after_upload"), Some(tr!("settings.upload.copy_link_after_upload.help")), |ui| {
        ui.switch("copy_url", &mut f.copy_url, tok);
    });
    srow(ui, tok, tr!("settings.upload.upload_recordings_automatically"), Some(tr!("settings.upload.upload_recordings_automatically.help")), |ui| {
        ui.switch("upload_recordings", &mut f.auto_rec, tok);
    });
    if open_s3 {
        s.section = super::S3_STORAGE;
    }
}

/// Settings › S3 storage: the bucket (Amazon S3, Cloudflare R2, MinIO, …)
/// and a Test connection; used when Upload › Upload to is S3-compatible.
pub fn s3_section(ui: &mut Ui, s: &mut super::Settings, tok: &Tok, click: &mut Option<Click>) {
    let stored = s.upload.stored(s.form.up.slot());
    let saving = s.upload.saving();
    let (probe, probe_err) = probe_help(s.upload.probe.as_ref());
    let busy = matches!(s.upload.probe, Some(Probe::Running));
    let f = &mut s.form.up;
    if f.dest != S3 {
        super::draw::note_line(ui, tok, tr!("settings.s3.used_when"), false, Some(2.0));
    }
    srow(ui, tok, tr!("settings.s3.endpoint"), Some(tr!("settings.s3.endpoint.help")), |ui| {
        ui.width(FIELD_W).text_field("s3_endpoint", &mut f.endpoint);
    });
    // Bucket and region share a row (the region is short: "auto", "us-east-1").
    srow(ui, tok, tr!("settings.s3.bucket_and_region"), Some(tr!("settings.s3.bucket_and_region.help")), |ui| {
        ui.width(96.0).text_field("s3_region", &mut f.region);
        ui.width(FIELD_W - 96.0 - 12.0).text_field("s3_bucket", &mut f.bucket);
    });
    srow(ui, tok, tr!("settings.s3.access_key_id"), None, |ui| {
        ui.width(FIELD_W).text_field("s3_access_key", &mut f.access_key);
    });
    let help = secret_help(!f.secret.text.trim().is_empty(), stored, saving);
    srow_ex(ui, tok, tr!("settings.s3.secret_access_key"), Some(&help), false, 44.0, |ui| {
        ui.masked = true;
        ui.width(FIELD_W).text_field("s3_secret", &mut f.secret);
    });
    let (help, warn) = public_help(&f.public_url.text);
    srow_ex(ui, tok, tr!("settings.s3.public_url_base"), Some(help), warn, if warn { 58.0 } else { ROW_H }, |ui| {
        ui.width(FIELD_W).text_field("s3_public_url", &mut f.public_url);
    });
    srow(ui, tok, tr!("settings.s3.key_prefix"), Some(tr!("settings.s3.key_prefix.help")), |ui| {
        ui.width(FIELD_W).text_field("s3_key_prefix", &mut f.prefix);
    });
    srow(ui, tok, tr!("settings.s3.path_style_urls"), Some(tr!("settings.s3.path_style_urls.help")), |ui| {
        ui.switch("s3_path_style", &mut f.path_style, tok);
    });
    srow_ex(ui, tok, tr!("settings.s3.test_connection"), Some(&probe), probe_err, if probe_err { 44.0 } else { ROW_H }, |ui| {
        let on = ui.enabled;
        ui.enabled = on && !busy;
        if ui.cbutton("s3_test", tr!("settings.s3.test_connection"), None, false, tok) {
            *click = Some(Click::TestUpload);
        }
        ui.enabled = on;
    });
}
