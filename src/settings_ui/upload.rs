//! Settings → **Upload**: the destination (Imgur or an S3-compatible
//! bucket: Amazon S3, Cloudflare R2, MinIO, …), the bucket's settings, a
//! Test connection, and what happens after an upload.
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
use std::sync::mpsc::Receiver;

/// (label, config value) of the destinations.
pub const DESTS: [(&str, &str); 2] = [("Imgur", "imgur"), ("S3-compatible", "s3")];
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

/// Keep the typed secret for `bucket` in `store`. Nothing typed: nothing
/// to do.
pub fn store_secret(store: &dyn crate::keystore::Store, bucket: &str, secret: &str) -> Result<(), String> {
    if secret.is_empty() {
        return Ok(());
    }
    let bucket = bucket.trim();
    if bucket.is_empty() {
        return Err("Enter the bucket first: the secret access key is kept per bucket.".into());
    }
    store
        .set(&crate::keystore::s3_name(bucket), &crate::keystore::s3_label(bucket), secret)
        .map_err(|e| format!("Could not keep the secret access key in {}: {e}", crate::keystore::place()))
}

/// What Test connection said.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Running,
    Passed,
    Failed(String),
}

/// The tab's state besides the form.
#[derive(Default)]
pub struct State {
    /// (bucket, a secret is stored for it), asked once per bucket name.
    stored: Option<(String, bool)>,
    pub probe: Option<Probe>,
    rx: Option<Receiver<Result<(), String>>>,
}

impl State {
    /// Whether a secret is stored for `bucket` (the store asked once per
    /// name; never for a blank one).
    pub fn stored(&mut self, bucket: &str) -> bool {
        let bucket = bucket.trim();
        if bucket.is_empty() {
            return false;
        }
        match &self.stored {
            Some((b, on)) if b == bucket => *on,
            _ => {
                let on = crate::keystore::os().has(&crate::keystore::s3_name(bucket)).unwrap_or(false);
                self.stored = Some((bucket.to_string(), on));
                on
            }
        }
    }

    /// Ask the store again (a secret was just kept).
    pub fn forget_stored(&mut self) {
        self.stored = None;
    }

    /// Test the form's bucket on a worker (tests: at once, and never over
    /// the network).
    pub fn start_test(&mut self, f: &UpForm) {
        if self.rx.is_some() {
            return;
        }
        let (target, key, secret) = (f.target(), f.access_key.text.trim().to_string(), f.secret.text.clone());
        self.probe = Some(Probe::Running);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("s3-test".into()).spawn(move || {
            let _ = tx.send(crate::s3::test_with(&target, &key, &secret));
        });
        match spawned {
            Ok(_) => self.rx = Some(rx),
            Err(e) => self.probe = Some(Probe::Failed(e.to_string())),
        }
    }

    /// The worker answered: true once (repaint).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the test stopped".into()),
        };
        self.rx = None;
        self.probe = Some(match r {
            Ok(()) => Probe::Passed,
            Err(e) => Probe::Failed(e),
        });
        true
    }

    /// Wait for the worker (tests).
    #[cfg(test)]
    pub fn wait(&mut self) {
        if let Some(rx) = self.rx.take() {
            let r = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or_else(|_| Err("timed out".into()));
            self.probe = Some(match r {
                Ok(()) => Probe::Passed,
                Err(e) => Probe::Failed(e),
            });
        }
    }
}

/// The secret row's helper line.
pub fn secret_help(typed: bool, stored: bool) -> String {
    let place = crate::keystore::place();
    match (typed, stored) {
        (true, _) => format!("Saved to {place} on Apply"),
        (false, true) => format!("Stored in {place}"),
        (false, false) => format!("Not stored; goes to {place}"),
    }
}

/// The Test connection row's helper line (and whether it is an error).
pub fn probe_help(p: Option<&Probe>) -> (String, bool) {
    match p {
        None => ("Uploads and deletes rustshot-test-\u{2026}.txt".into(), false),
        Some(Probe::Running) => ("Testing\u{2026}".into(), false),
        Some(Probe::Passed) => ("Connected \u{b7} uploaded and deleted a test file".into(), false),
        Some(Probe::Failed(e)) => {
            let mut c = e.chars();
            let first = c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default();
            (format!("{first}{}", c.as_str()), true)
        }
    }
}

pub fn section(ui: &mut Ui, s: &mut super::Settings, tok: &Tok, click: &mut Option<Click>) {
    let bucket = s.form.up.bucket.text.clone();
    let stored = s.upload.stored(&bucket);
    let (probe, probe_err) = probe_help(s.upload.probe.as_ref());
    let f = &mut s.form.up;
    ui.sec("Destination", tok);
    srow(ui, tok, "Upload to", None, |ui| {
        let labels: Vec<&str> = DESTS.iter().map(|d| d.0).collect();
        ui.seg("upload_dest", &labels, &mut f.dest, tok);
    });
    if f.dest != S3 {
        ui.sec("Imgur", tok);
        srow(ui, tok, "Client ID", Some("Rustshot's own unless you register one"), |ui| {
            ui.width(FIELD_W).text_field("upload_client_id", &mut f.client_id);
        });
    } else {
        ui.sec("S3-compatible storage", tok);
        srow(ui, tok, "Endpoint", Some("The provider's S3 API address"), |ui| {
            ui.width(FIELD_W).text_field("s3_endpoint", &mut f.endpoint);
        });
        srow(ui, tok, "Region", Some("\"auto\" for Cloudflare R2"), |ui| {
            ui.width(FIELD_W).text_field("s3_region", &mut f.region);
        });
        srow(ui, tok, "Bucket", None, |ui| {
            ui.width(FIELD_W).text_field("s3_bucket", &mut f.bucket);
        });
        srow(ui, tok, "Access key ID", None, |ui| {
            ui.width(FIELD_W).text_field("s3_access_key", &mut f.access_key);
        });
        let help = secret_help(!f.secret.text.is_empty(), stored);
        srow_ex(ui, tok, "Secret access key", Some(&help), false, 44.0, |ui| {
            ui.masked = true;
            ui.width(FIELD_W).text_field("s3_secret", &mut f.secret);
        });
        srow(ui, tok, "Public URL base", Some("Shared links begin with it"), |ui| {
            ui.width(FIELD_W).text_field("s3_public_url", &mut f.public_url);
        });
        srow(ui, tok, "Key prefix", Some("%Y %m %d as in file names"), |ui| {
            ui.width(FIELD_W).text_field("s3_key_prefix", &mut f.prefix);
        });
        srow(ui, tok, "Path-style URLs", Some("On for R2 and most S3-compatible servers"), |ui| {
            ui.switch("s3_path_style", &mut f.path_style, tok);
        });
        let busy = matches!(s.upload.probe, Some(Probe::Running));
        srow_ex(ui, tok, "Test connection", Some(&probe), probe_err, if probe_err { 44.0 } else { ROW_H }, |ui| {
            let on = ui.enabled;
            ui.enabled = on && !busy;
            if ui.cbutton("s3_test", "Test connection", None, false, tok) {
                *click = Some(Click::TestUpload);
            }
            ui.enabled = on;
        });
    }
    ui.sec("After upload", tok);
    srow(ui, tok, "Copy link after upload", Some("Ctrl+U in the overlay uploads; the link replaces the image"), |ui| {
        ui.switch("copy_url", &mut f.copy_url, tok);
    });
    srow(ui, tok, "Upload recordings automatically", Some("When saved; Copy link then copies at once"), |ui| {
        ui.switch("upload_recordings", &mut f.auto_rec, tok);
    });
}
