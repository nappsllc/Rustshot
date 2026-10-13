//! The freedesktop Secret Service (`org.freedesktop.secrets`) over our
//! D-Bus client: a `plain` session (the bus is local and per user), items
//! in the default collection found by their attributes, a locked
//! collection unlocked through the service's own prompt.
//!
//! The protocol is written against [`Bus`], so the tests run it against a
//! scripted fake; [`Session`] is the real one (Linux).

use crate::dbus::{Bus, Message, Value};
use anyhow::{Result, anyhow, bail};
use std::time::Duration;

pub const DEST: &str = "org.freedesktop.secrets";
pub const PATH: &str = "/org/freedesktop/secrets";
pub const SERVICE: &str = "org.freedesktop.Secret.Service";
pub const COLLECTION: &str = "org.freedesktop.Secret.Collection";
pub const ITEM: &str = "org.freedesktop.Secret.Item";
pub const PROMPT: &str = "org.freedesktop.Secret.Prompt";
pub const DEFAULT: &str = "/org/freedesktop/secrets/aliases/default";
/// How long a person has to answer an unlock prompt.
pub const PROMPT_WAIT: Duration = Duration::from_secs(120);
const CALL: Duration = Duration::from_secs(10);

/// Why nothing is kept when there is no Secret Service.
pub const MISSING: &str = "no Secret Service is running (install or start GNOME Keyring or KWallet); Rustshot does not keep the secret in a file";

fn call(b: &mut dyn Bus, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Result<Message> {
    b.call(Message::call(DEST, path, iface, member, body), CALL).map_err(|e| {
        let s = format!("{e:#}");
        if s.contains("ServiceUnknown") || s.contains("NameHasNoOwner") { anyhow!(MISSING) } else { e }
    })
}

/// Our items' attributes.
pub fn attributes(name: &str) -> Value {
    Value::Dict("s".into(), "s".into(), vec![(Value::str("application"), Value::str("rustshot")), (Value::str("rustshot-name"), Value::str(name))])
}

/// Open a `plain` session: its path.
pub fn open_session(b: &mut dyn Bus) -> Result<String> {
    let r = call(b, PATH, SERVICE, "OpenSession", vec![Value::str("plain"), Value::variant(Value::str(""))])?;
    r.body.get(1).and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("OpenSession: no session"))
}

fn paths(v: Option<&Value>) -> Vec<String> {
    v.map(|v| v.items().iter().filter_map(|p| p.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The items named `name`: (unlocked, locked).
pub fn search(b: &mut dyn Bus, name: &str) -> Result<(Vec<String>, Vec<String>)> {
    let r = call(b, PATH, SERVICE, "SearchItems", vec![attributes(name)])?;
    Ok((paths(r.body.first()), paths(r.body.get(1))))
}

/// Run prompt `prompt` ("/" = none) and wait for its answer; Err when
/// dismissed.
pub fn run_prompt(b: &mut dyn Bus, prompt: &str) -> Result<()> {
    if prompt.is_empty() || prompt == "/" {
        return Ok(());
    }
    let rule = crate::dbus::signal_rule("", prompt, PROMPT, "Completed");
    crate::dbus::add_match(b, &rule)?;
    let r = call(b, prompt, PROMPT, "Prompt", vec![Value::str("")]).and_then(|_| {
        let p = prompt.to_string();
        let m = b.wait_signal(&|m| m.path.as_deref() == Some(p.as_str()) && m.is(PROMPT, "Completed"), PROMPT_WAIT)?;
        let m = m.ok_or_else(|| anyhow!("the keyring prompt was not answered"))?;
        if m.body.first().and_then(Value::as_bool).unwrap_or(true) {
            bail!("the keyring stayed locked");
        }
        Ok(())
    });
    let _ = crate::dbus::remove_match(b, &rule);
    r
}

/// Unlock `objects` (items or a collection), prompting when needed.
pub fn unlock(b: &mut dyn Bus, objects: &[String]) -> Result<()> {
    let list = Value::Array("o".into(), objects.iter().map(|p| Value::path(p)).collect());
    let r = call(b, PATH, SERVICE, "Unlock", vec![list])?;
    let prompt = r.body.get(1).and_then(Value::as_str).unwrap_or("/").to_string();
    run_prompt(b, &prompt)
}

/// The secret of item `item` in `session`.
fn item_secret(b: &mut dyn Bus, item: &str, session: &str) -> Result<String> {
    let r = call(b, item, ITEM, "GetSecret", vec![Value::path(session)])?;
    let s = r.body.first().ok_or_else(|| anyhow!("GetSecret: empty reply"))?;
    let bytes: Vec<u8> = s.items().get(2).map(|v| v.items().iter().filter_map(|x| if let Value::Byte(b) = x { Some(*b) } else { None }).collect()).unwrap_or_default();
    String::from_utf8(bytes).map_err(|_| anyhow!("the stored secret is not text"))
}

/// The secret named `name`.
pub fn get(b: &mut dyn Bus, name: &str) -> Result<Option<String>> {
    let session = open_session(b)?;
    let (unlocked, locked) = search(b, name)?;
    let item = match (unlocked.first(), locked.first()) {
        (Some(i), _) => i.clone(),
        (None, Some(i)) => {
            unlock(b, std::slice::from_ref(i))?;
            i.clone()
        }
        (None, None) => return Ok(None),
    };
    item_secret(b, &item, &session).map(Some)
}

/// The `CreateItem` arguments for `name`.
pub fn create_args(name: &str, label: &str, secret: &str, session: &str) -> Vec<Value> {
    let props = Value::Dict(
        "s".into(),
        "v".into(),
        vec![
            (Value::str("org.freedesktop.Secret.Item.Label"), Value::variant(Value::str(label))),
            (Value::str("org.freedesktop.Secret.Item.Attributes"), Value::variant(attributes(name))),
        ],
    );
    let secret = Value::Struct(vec![
        Value::path(session),
        Value::Array("y".into(), Vec::new()),
        Value::Array("y".into(), secret.bytes().map(Value::Byte).collect()),
        Value::str("text/plain; charset=utf8"),
    ]);
    vec![props, secret, Value::Bool(true)]
}

/// Keep `secret` as `name` in the default collection (replacing it).
pub fn set(b: &mut dyn Bus, name: &str, label: &str, secret: &str) -> Result<()> {
    let session = open_session(b)?;
    let create = |b: &mut dyn Bus| call(b, DEFAULT, COLLECTION, "CreateItem", create_args(name, label, secret, &session));
    let r = match create(b) {
        Ok(r) => r,
        Err(e) if format!("{e:#}").contains("IsLocked") => {
            unlock(b, &[DEFAULT.to_string()])?;
            create(b)?
        }
        Err(e) if format!("{e:#}").contains("NoSuchObject") => bail!("the Secret Service has no default keyring"),
        Err(e) => return Err(e),
    };
    let item = r.body.first().and_then(Value::as_str).unwrap_or("/").to_string();
    let prompt = r.body.get(1).and_then(Value::as_str).unwrap_or("/").to_string();
    run_prompt(b, &prompt)?;
    if item == "/" && prompt == "/" {
        bail!("the Secret Service did not keep the secret");
    }
    Ok(())
}

/// Forget every item named `name`.
pub fn delete(b: &mut dyn Bus, name: &str) -> Result<()> {
    let (unlocked, locked) = search(b, name)?;
    if !locked.is_empty() {
        unlock(b, &locked)?;
    }
    for item in unlocked.iter().chain(&locked) {
        let r = call(b, item, ITEM, "Delete", Vec::new())?;
        let prompt = r.body.first().and_then(Value::as_str).unwrap_or("/").to_string();
        run_prompt(b, &prompt)?;
    }
    Ok(())
}

/// The session bus's Secret Service.
#[cfg(target_os = "linux")]
pub struct Session;

#[cfg(target_os = "linux")]
impl Session {
    fn with<R>(f: impl FnOnce(&mut dyn Bus) -> Result<R>) -> Result<R, String> {
        let mut c = crate::dbus::Conn::session().map_err(|e| format!("{MISSING} ({e:#})"))?;
        f(&mut c).map_err(|e| format!("{e:#}"))
    }
}

#[cfg(target_os = "linux")]
impl super::Store for Session {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Session::with(|b| get(b, name))
    }

    fn set(&self, name: &str, label: &str, secret: &str) -> Result<(), String> {
        Session::with(|b| set(b, name, label, secret))
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        Session::with(|b| delete(b, name))
    }
}

#[cfg(test)]
mod tests;
