use super::*;
use crate::dbus::{Kind, Message, Value};
use std::collections::VecDeque;

/// An item: path, attributes, secret, locked.
type Item = (String, Vec<(String, String)>, Vec<u8>, bool);

/// A scripted Secret Service: items (path, attributes, secret, locked),
/// a default collection that may be locked or missing, prompts that are
/// answered (or dismissed) with a Completed signal.
struct Fake {
    items: Vec<Item>,
    collection_locked: bool,
    no_collection: bool,
    no_service: bool,
    dismiss: bool,
    signals: VecDeque<Message>,
    calls: Vec<String>,
    rules: Vec<String>,
    /// Items created so far (their paths' numbers).
    made: usize,
}

impl Fake {
    fn new() -> Fake {
        Fake { items: Vec::new(), collection_locked: false, no_collection: false, no_service: false, dismiss: false, signals: VecDeque::new(), calls: Vec::new(), rules: Vec::new(), made: 0 }
    }

    fn attrs_of(v: &Value) -> Vec<(String, String)> {
        match v.inner() {
            Value::Dict(_, _, e) => e.iter().map(|(k, v)| (k.as_str().unwrap().to_string(), v.as_str().unwrap().to_string())).collect(),
            _ => panic!("attributes: {v:?}"),
        }
    }
}

impl Bus for Fake {
    fn unique_name(&self) -> &str {
        ":1.42"
    }

    fn call(&mut self, m: Message, _t: Duration) -> Result<Message> {
        let member = m.member.clone().unwrap_or_default();
        let path = m.path.clone().unwrap_or_default();
        match member.as_str() {
            "AddMatch" => {
                self.rules.push(m.body[0].as_str().unwrap().into());
                return Ok(Message::reply(&m, vec![]));
            }
            "RemoveMatch" => {
                let r = m.body[0].as_str().unwrap().to_string();
                self.rules.retain(|x| *x != r);
                return Ok(Message::reply(&m, vec![]));
            }
            _ => {}
        }
        self.calls.push(format!("{member} {path}"));
        if self.no_service {
            bail!("{member}: org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.secrets was not provided");
        }
        let body = match member.as_str() {
            "OpenSession" => {
                assert_eq!(m.body[0].as_str(), Some("plain"));
                vec![Value::variant(Value::str("")), Value::path("/org/freedesktop/secrets/session/1")]
            }
            "SearchItems" => {
                let want = Fake::attrs_of(&m.body[0]);
                let hit = |locked: bool| {
                    Value::Array("o".into(), self.items.iter().filter(|i| i.3 == locked && want.iter().all(|w| i.1.contains(w))).map(|i| Value::path(&i.0)).collect())
                };
                vec![hit(false), hit(true)]
            }
            "Unlock" => {
                let objs: Vec<String> = m.body[0].items().iter().map(|p| p.as_str().unwrap().to_string()).collect();
                // Unlocking needs the person: a prompt.
                for i in self.items.iter_mut().filter(|i| objs.contains(&i.0)) {
                    i.3 = self.dismiss;
                }
                if objs.iter().any(|o| o == DEFAULT) && !self.dismiss {
                    self.collection_locked = false;
                }
                vec![Value::Array("o".into(), vec![]), Value::path("/org/freedesktop/secrets/prompt/u1")]
            }
            "Prompt" => {
                let mut s = Message::signal(&path, PROMPT, "Completed", vec![Value::Bool(self.dismiss), Value::variant(Value::str(""))]);
                s.sender = Some(":1.5".into());
                assert!(self.rules.iter().any(|r| r.contains(&path)), "subscribed before prompting");
                self.signals.push_back(s);
                vec![]
            }
            "GetSecret" => {
                let i = self.items.iter().find(|i| i.0 == path).expect("item");
                assert!(!i.3, "read only once unlocked");
                vec![Value::Struct(vec![m.body[0].clone(), Value::Array("y".into(), vec![]), Value::Array("y".into(), i.2.iter().map(|b| Value::Byte(*b)).collect()), Value::str("text/plain")])]
            }
            "CreateItem" => {
                assert_eq!(path, DEFAULT);
                if self.no_collection {
                    bail!("CreateItem: org.freedesktop.Secret.Error.NoSuchObject: no such collection");
                }
                if self.collection_locked {
                    bail!("CreateItem: org.freedesktop.Secret.Error.IsLocked: the collection is locked");
                }
                assert_eq!(m.signature(), "a{sv}(oayays)b");
                let attrs = Fake::attrs_of(m.body[0].get("org.freedesktop.Secret.Item.Attributes").unwrap());
                assert_eq!(m.body[0].get("org.freedesktop.Secret.Item.Label").and_then(Value::as_str), Some("Label"));
                let secret: Vec<u8> = m.body[1].items()[2].items().iter().map(|b| if let Value::Byte(b) = b { *b } else { 0 }).collect();
                assert_eq!(m.body[2], Value::Bool(true), "replace");
                self.items.retain(|i| i.1 != attrs);
                self.made += 1;
                let p = format!("/org/freedesktop/secrets/collection/login/{}", self.made);
                self.items.push((p.clone(), attrs, secret, false));
                vec![Value::path(&p), Value::path("/")]
            }
            "Delete" => {
                self.items.retain(|i| i.0 != path);
                vec![Value::path("/")]
            }
            other => panic!("unexpected call {other}"),
        };
        let r = Message::reply(&m, body);
        assert_eq!(r.kind, Kind::Return);
        Ok(r)
    }

    fn send(&mut self, _m: Message) -> Result<u32> {
        Ok(0)
    }

    fn wait_signal(&mut self, want: &dyn Fn(&Message) -> bool, _t: Duration) -> Result<Option<Message>> {
        let i = self.signals.iter().position(want);
        Ok(i.and_then(|i| self.signals.remove(i)))
    }
}

#[test]
fn a_secret_is_kept_found_replaced_and_deleted() {
    let mut f = Fake::new();
    assert_eq!(get(&mut f, "rustshot:s3:shots").unwrap(), None);
    assert!(!has(&mut f, "rustshot:s3:shots").unwrap());
    set(&mut f, "rustshot:s3:shots", "Label", "s3cr3t/Key+").unwrap();
    assert!(has(&mut f, "rustshot:s3:shots").unwrap());
    set(&mut f, "rustshot:s3:other", "Label", "other").unwrap();
    assert_eq!(get(&mut f, "rustshot:s3:shots").unwrap().as_deref(), Some("s3cr3t/Key+"));
    set(&mut f, "rustshot:s3:shots", "Label", "new").unwrap();
    assert_eq!(f.items.len(), 2, "replaced, not added");
    assert_eq!(get(&mut f, "rustshot:s3:shots").unwrap().as_deref(), Some("new"));
    assert!(f.items.iter().all(|i| i.1.contains(&("application".into(), "rustshot".into()))));
    delete(&mut f, "rustshot:s3:shots").unwrap();
    assert_eq!(get(&mut f, "rustshot:s3:shots").unwrap(), None);
    assert_eq!(get(&mut f, "rustshot:s3:other").unwrap().as_deref(), Some("other"));
    delete(&mut f, "rustshot:s3:gone").unwrap();
}

/// A locked item is unlocked through the service's prompt before reading;
/// a locked collection before creating.
#[test]
fn locked_keyrings_are_unlocked_through_the_prompt() {
    let mut f = Fake::new();
    f.items.push(("/i/1".into(), vec![("application".into(), "rustshot".into()), ("rustshot-name".into(), "n".into())], b"x".to_vec(), true));
    assert!(has(&mut f, "n").unwrap(), "seen while locked");
    assert!(!f.calls.iter().any(|c| c.starts_with("Unlock")), "without unlocking");
    assert_eq!(get(&mut f, "n").unwrap().as_deref(), Some("x"));
    assert!(f.calls.iter().any(|c| c.starts_with("Unlock")) && f.calls.iter().any(|c| c.starts_with("Prompt")));
    assert!(f.rules.is_empty(), "the match rule is removed again");

    let mut f = Fake::new();
    f.collection_locked = true;
    set(&mut f, "n", "Label", "y").unwrap();
    assert_eq!(get(&mut f, "n").unwrap().as_deref(), Some("y"));
}

#[test]
fn a_dismissed_prompt_or_no_service_fails_and_says_why() {
    let mut f = Fake::new();
    f.collection_locked = true;
    f.dismiss = true;
    let e = set(&mut f, "n", "Label", "y").unwrap_err().to_string();
    assert!(e.contains("locked"), "{e}");
    assert!(f.items.is_empty());

    let mut f = Fake::new();
    f.no_service = true;
    let e = set(&mut f, "n", "Label", "y").unwrap_err().to_string();
    assert_eq!(e, MISSING);
    assert!(e.contains("does not keep the secret in a file"));

    let mut f = Fake::new();
    f.no_collection = true;
    assert!(set(&mut f, "n", "Label", "y").unwrap_err().to_string().contains("no default keyring"));
}
