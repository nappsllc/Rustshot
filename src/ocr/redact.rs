//! Quick Redact: sensitive text among recognized words, in pure Rust (no
//! regex crate). Each line's words are joined with single spaces and
//! scanned for e-mail addresses, phone numbers (E.164 and national
//! formats: 7 to 15 digits with separators), card numbers (13 to 19
//! digits passing Luhn), IBAN-like account numbers (mod 97), IPv4
//! addresses, URLs whose query names a token, key, secret or signature,
//! and values after "password:" / "api key:"-style labels. A match names
//! the words it covers; the overlay blurs their boxes.

use super::Line;
use std::ops::Range;

/// The kinds of sensitive text (the Redact menu's checkboxes,
/// `redact_kinds` in the config).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Kind {
    Email,
    Phone,
    Card,
    /// IBAN-like account numbers.
    Id,
    /// URLs with tokens, and labelled secrets ("password: …").
    Url,
    Ip,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Email, Kind::Phone, Kind::Card, Kind::Id, Kind::Url, Kind::Ip];

    /// Its name in `redact_kinds`.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Email => "emails",
            Kind::Phone => "phones",
            Kind::Card => "cards",
            Kind::Id => "ids",
            Kind::Url => "urls",
            Kind::Ip => "ips",
        }
    }

    /// Its checkbox in the Redact menu.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Email => "Emails",
            Kind::Phone => "Phone numbers",
            Kind::Card => "Card numbers",
            Kind::Id => "IDs (IBAN)",
            Kind::Url => "URLs, tokens & passwords",
            Kind::Ip => "IP addresses",
        }
    }

    pub fn from_id(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(s.trim()))
    }
}

/// The kinds named in `redact_kinds` ("emails, phones"); unknown names
/// are skipped. `"all"` (the default) is every kind, `""` none.
pub fn parse_kinds(s: &str) -> Vec<Kind> {
    if s.trim().eq_ignore_ascii_case("all") {
        return Kind::ALL.to_vec();
    }
    let mut v: Vec<Kind> = s.split(',').filter_map(Kind::from_id).collect();
    v.sort();
    v.dedup();
    v
}

/// `kinds` as `redact_kinds` writes them (every kind: `"all"`).
pub fn kinds_text(kinds: &[Kind]) -> String {
    if Kind::ALL.iter().all(|k| kinds.contains(k)) {
        return "all".into();
    }
    Kind::ALL.iter().filter(|k| kinds.contains(k)).map(|k| k.id()).collect::<Vec<_>>().join(",")
}

/// One match: its kind and the words it covers (line, word) in the
/// lines given to [`find`].
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub kind: Kind,
    pub line: usize,
    pub words: Range<usize>,
}

/// The sensitive text of `lines` of the `kinds` asked for.
pub fn find(lines: &[Line], kinds: &[Kind]) -> Vec<Hit> {
    let mut out = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let mut text = String::new();
        let mut spans = Vec::with_capacity(l.words.len());
        for (i, w) in l.words.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            let a = text.len();
            text.push_str(w.text.trim());
            spans.push(a..text.len());
        }
        for (kind, r) in find_in_text(&text, kinds) {
            let first = spans.iter().position(|s| s.end > r.start);
            let last = spans.iter().rposition(|s| s.start < r.end);
            if let (Some(a), Some(b)) = (first, last)
                && a <= b
            {
                out.push(Hit { kind, line: li, words: a..b + 1 });
            }
        }
    }
    out
}

/// The sensitive spans of one line of text (byte ranges), of `kinds`,
/// left to right; overlapping finds keep the first kind in the order
/// cards, IBANs, URLs, e-mails, IPs, labels, phones.
pub fn find_in_text(s: &str, kinds: &[Kind]) -> Vec<(Kind, Range<usize>)> {
    let on = |k: Kind| kinds.contains(&k);
    let mut found: Vec<(Kind, Range<usize>)> = Vec::new();
    let add = |k: Kind, r: Range<usize>, found: &mut Vec<(Kind, Range<usize>)>| {
        if r.is_empty() || found.iter().any(|(_, f)| f.start < r.end && r.start < f.end) {
            return;
        }
        found.push((k, r));
    };
    if on(Kind::Card) {
        for r in cards(s) {
            add(Kind::Card, r, &mut found);
        }
    }
    if on(Kind::Id) {
        for r in ibans(s) {
            add(Kind::Id, r, &mut found);
        }
    }
    if on(Kind::Url) {
        for r in token_urls(s) {
            add(Kind::Url, r, &mut found);
        }
    }
    if on(Kind::Email) {
        for r in emails(s) {
            add(Kind::Email, r, &mut found);
        }
    }
    if on(Kind::Ip) {
        for r in ipv4s(s) {
            add(Kind::Ip, r, &mut found);
        }
    }
    if on(Kind::Url) {
        for r in labelled(s) {
            add(Kind::Url, r, &mut found);
        }
    }
    if on(Kind::Phone) {
        // Not over what is a date, a card or an address already.
        for r in phones(s) {
            add(Kind::Phone, r, &mut found);
        }
    }
    found.sort_by_key(|(_, r)| r.start);
    found
}

/// A local part's character (Unicode letters and digits: IDN addresses).
fn is_local(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

fn is_domain(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-')
}

/// `name@domain.tld` (Unicode names and domains too: `jöhn@bücher.de`).
fn emails(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for (at, _) in s.match_indices('@') {
        // Every index below comes from `char_indices`: on char boundaries.
        let mut a = at;
        for (i, c) in s[..at].char_indices().rev() {
            if !is_local(c) {
                break;
            }
            a = i;
        }
        while a < at && s[a..].starts_with(['.', '-', '+']) {
            a += 1;
        }
        let mut e = at + 1;
        for (i, c) in s[at + 1..].char_indices() {
            if !is_domain(c) {
                break;
            }
            e = at + 1 + i + c.len_utf8();
        }
        while e > at + 1 && s[..e].ends_with(['.', '-']) {
            e -= 1;
        }
        let domain = &s[at + 1..e];
        let tld_ok = domain.rsplit_once('.').is_some_and(|(host, tld)| !host.is_empty() && tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic));
        if a < at && tld_ok && !domain.contains("..") {
            out.push(a..e);
        }
    }
    out
}

/// A run of digits with single separators from `seps` between them:
/// (range, digits).
fn digit_runs(s: &str, seps: &[u8]) -> Vec<(Range<usize>, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && (b[i - 1].is_ascii_alphanumeric())) {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits = String::new();
        let mut end = i;
        while i < b.len() {
            if b[i].is_ascii_digit() {
                digits.push(b[i] as char);
                i += 1;
                end = i;
            } else if seps.contains(&b[i]) && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }
        // A run that runs into letters is part of a word (an id, a hash).
        if end < b.len() && b[end].is_ascii_alphabetic() {
            i = end + 1;
            continue;
        }
        out.push((start..end, digits));
        i = end.max(start + 1);
    }
    out
}

/// The Luhn checksum of `digits` holds.
pub fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (i, c) in digits.bytes().rev().enumerate() {
        let mut d = (c - b'0') as u32;
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    !digits.is_empty() && sum.is_multiple_of(10)
}

/// 13 to 19 digits (spaces or dashes between groups) passing Luhn: the
/// longest run of whole groups from the first group that makes one (a
/// number next to it is not taken in).
fn cards(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for (r, _) in digit_runs(s, b" -") {
        // The run's groups of digits.
        let mut groups: Vec<Range<usize>> = Vec::new();
        let mut i = r.start;
        while i < r.end {
            let a = i;
            while i < r.end && b[i].is_ascii_digit() {
                i += 1;
            }
            groups.push(a..i);
            i += 1;
        }
        let mut g = 0;
        while g < groups.len() {
            let found = (g..groups.len()).rev().find(|&e| {
                let d: String = groups[g..=e].iter().map(|x| &s[x.clone()]).collect();
                (13..=19).contains(&d.len()) && luhn(&d) && !d.bytes().all(|c| c == b'0')
            });
            match found {
                Some(e) => {
                    out.push(groups[g].start..groups[e].end);
                    g = e + 1;
                }
                None => g += 1,
            }
        }
    }
    out
}

/// An IBAN's mod-97 check (`s`: letters and digits, no spaces).
pub fn iban_ok(s: &str) -> bool {
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let (head, tail) = s.split_at(4);
    let mut rem = 0u32;
    for c in tail.chars().chain(head.chars()) {
        let v = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'A'..='Z' => c as u32 - 'A' as u32 + 10,
            _ => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

/// Two letters, two check digits and 11 to 30 letters or digits (spaces
/// between groups allowed) passing mod 97.
fn ibans(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        let starts = b[i].is_ascii_uppercase()
            && b[i + 1].is_ascii_uppercase()
            && b[i + 2].is_ascii_digit()
            && b[i + 3].is_ascii_digit()
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if !starts {
            i += 1;
            continue;
        }
        // Collect characters (skipping single spaces), remembering where
        // each ends, then take the longest prefix that checks out.
        let mut chars = String::new();
        let mut ends = Vec::new();
        let mut j = i;
        while j < b.len() && chars.len() < 34 {
            let c = b[j].to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                chars.push(c as char);
                ends.push(j + 1);
                j += 1;
            } else if b[j] == b' ' && j + 1 < b.len() && b[j + 1].is_ascii_alphanumeric() {
                j += 1;
            } else {
                break;
            }
        }
        let best = (15..=chars.len()).rev().find(|&n| {
            // Ends on a whole group: not halfway into a following word.
            let e = ends[n - 1];
            (e == b.len() || !b[e].is_ascii_alphanumeric()) && iban_ok(&chars[..n])
        });
        match best {
            Some(n) => {
                out.push(i..ends[n - 1]);
                i = ends[n - 1];
            }
            None => i += 1,
        }
    }
    out
}

/// A dotted quad of 0-255 (an optional `:port` after it).
fn ipv4s(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let start = i;
        let mut parts = 0;
        let mut ok = true;
        let mut j = i;
        loop {
            let a = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let n = &s[a..j];
            if n.is_empty() || n.len() > 3 || !n.parse::<u32>().is_ok_and(|v| v <= 255) {
                ok = false;
                break;
            }
            parts += 1;
            if parts == 4 {
                break;
            }
            if j < b.len() && b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                j += 1;
            } else {
                ok = false;
                break;
            }
        }
        let after = |k: usize| k < b.len() && (b[k].is_ascii_alphanumeric() || (b[k] == b'.' && k + 1 < b.len() && b[k + 1].is_ascii_digit()));
        if ok && parts == 4 && !after(j) {
            let mut e = j;
            if e + 1 < b.len() && b[e] == b':' && b[e + 1].is_ascii_digit() {
                e += 1;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
            }
            out.push(start..e);
            i = e;
        } else {
            i = j.max(i + 1);
        }
    }
    out
}

/// A query parameter's name that holds a secret.
fn secret_param(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let n = n.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    matches!(
        n,
        "token" | "key" | "secret" | "sig" | "signature" | "apikey" | "password" | "pass" | "pwd" | "auth" | "code" | "access_key" | "client_secret" | "session" | "sid" | "jwt" | "sessionid" | "session_id"
    ) || n.ends_with("token")
        || n.ends_with("_key")
        || n.ends_with("-key")
        || n.ends_with("secret")
        || n.ends_with("_sig")
        || n.ends_with("session")
}

/// Webhook and bot URLs whose path is the secret.
const SECRET_HOSTS: [&str; 4] = ["hooks.slack.com/services/", "discord.com/api/webhooks/", "discordapp.com/api/webhooks/", "api.telegram.org/bot"];

/// A path segment that looks like a token: 24 characters or more of
/// base64 / hex, with letters and digits mixed (not a slug of words).
fn token_segment(seg: &str) -> bool {
    let b = seg.as_bytes();
    let digits = b.iter().filter(|c| c.is_ascii_digit()).count();
    let letters = b.iter().filter(|c| c.is_ascii_alphabetic()).count();
    let dashes = b.iter().filter(|c| **c == b'-').count();
    b.len() >= 24 && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'=' | b'+')) && digits >= 3 && letters >= 3 && dashes <= 2
}

/// Characters that end a URL in running text (closing quotes and brackets).
const URL_TAIL: [char; 14] = ['.', ',', ')', ']', '}', '"', '\'', '>', ';', '!', '\u{201d}', '\u{2019}', '\u{bb}', '\u{203a}'];

/// Where a URL starts in `tok` (a word): its scheme, `www.`, or a bare
/// `host.tld/` (byte index, on a char boundary).
fn url_start(tok: &str) -> Option<usize> {
    if let Some(i) = tok.find("://") {
        let start = tok[..i].char_indices().rev().take_while(|(_, c)| c.is_ascii_alphanumeric()).last().map_or(i, |(j, _)| j);
        return (start < i).then_some(start);
    }
    if let Some(i) = tok.to_ascii_lowercase().find("www.") {
        return Some(i);
    }
    // example.com/x?key=…: a host with a dot and a letters-only TLD.
    let start = tok.char_indices().find(|(_, c)| c.is_ascii_alphanumeric())?.0;
    let slash = tok[start..].find('/')? + start;
    let host = &tok[start..slash];
    let ok = host.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
        && host.rsplit_once('.').is_some_and(|(h, tld)| !h.is_empty() && tld.len() >= 2 && tld.bytes().all(|c| c.is_ascii_alphabetic()));
    ok.then_some(start)
}

/// A URL whose query or fragment names a token, key, secret, signature or
/// session, a webhook URL, or one with a token-like path segment: the
/// whole URL.
fn token_urls(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for (start, tok) in words(s) {
        let Some(u) = url_start(tok) else { continue };
        let url = tok[u..].trim_end_matches(URL_TAIL);
        let lower = url.to_ascii_lowercase();
        let (head, query) = match url.find(['?', '#']) {
            Some(q) => (&url[..q], &url[q + 1..]),
            None => (url, ""),
        };
        let path = head.find("://").map_or(head, |i| &head[i + 3..]);
        let path = path.split_once('/').map_or("", |(_, p)| p);
        let by_query = query.split(['&', ';', '#', '?']).any(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            !v.is_empty() && secret_param(k)
        });
        let by_path = SECRET_HOSTS.iter().any(|h| lower.contains(h)) || path.split('/').any(token_segment);
        if by_query || by_path {
            out.push(start + u..start + u + url.len());
        }
    }
    out
}

/// The words of `s` (split at whitespace) with their byte offsets.
fn words(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.split_whitespace().map(move |w| (w.as_ptr() as usize - s.as_ptr() as usize, w))
}

/// Labels whose value is the rest of the line (passphrases, headers).
const LINE_LABELS: [&str; 6] = ["password", "passphrase", "passwd", "pwd", "passcode", "authorization"];
/// Labels whose value is the next word.
const WORD_LABELS: [&str; 8] = ["api key", "api_key", "apikey", "api-key", "secret", "token", "access token", "private key"];

/// Where a label's value starts: after the label at `i` (`len` bytes), a
/// few more letters ("API Key", "Secret Key") and `:` or `=`.
fn label_value(s: &str, i: usize, len: usize) -> Option<usize> {
    let b = s.as_bytes();
    // The label starts a word.
    if s[..i].chars().next_back().is_some_and(char::is_alphanumeric) {
        return None;
    }
    let mut j = i + len;
    while j < b.len() && j < i + len + 6 && (b[j] == b' ' || b[j].is_ascii_alphabetic()) {
        // One more word may follow ("key"), not a sentence.
        if b[j].is_ascii_alphabetic() && !s[i + len..j + 1].trim().bytes().all(|c| c.is_ascii_alphabetic()) {
            break;
        }
        j += 1;
    }
    if j >= b.len() || !(b[j] == b':' || b[j] == b'=') {
        return None;
    }
    j += 1;
    while j < b.len() && b[j] == b' ' {
        j += 1;
    }
    (j < b.len()).then_some(j)
}

/// The end of the word starting at `v0`.
fn word_end(s: &str, v0: usize) -> usize {
    s[v0..].find(char::is_whitespace).map_or(s.len(), |k| v0 + k)
}

/// Values after secret labels: a passphrase or a header to the end of the
/// line, an API key or a token one word; `Bearer <token>`; `.env` lines
/// (`AWS_SECRET_ACCESS_KEY=…`, a key naming SECRET, TOKEN, KEY, PASS or
/// PWD); JSON Web Tokens (`eyJ….….…`).
fn labelled(s: &str) -> Vec<Range<usize>> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    for (labels, line) in [(&LINE_LABELS[..], true), (&WORD_LABELS[..], false)] {
        for label in labels {
            for (i, _) in lower.match_indices(label) {
                let Some(v0) = label_value(s, i, label.len()) else { continue };
                let end = if line { s.trim_end().len().max(v0) } else { word_end(s, v0) };
                if end > v0 {
                    out.push(v0..end);
                }
            }
        }
    }
    // Bearer <token>.
    for (i, _) in lower.match_indices("bearer ") {
        if s[..i].chars().next_back().is_some_and(char::is_alphanumeric) {
            continue;
        }
        let v0 = i + "bearer ".len();
        let v0 = v0 + (s[v0..].len() - s[v0..].trim_start().len());
        let end = word_end(s, v0);
        if end >= v0 + 8 {
            out.push(v0..end);
        }
    }
    for (at, w) in words(s) {
        // KEY=value (and KEY: value with an env-style key).
        let (key, val, val_at) = match w.split_once('=') {
            Some((k, v)) => (k, v, at + k.len() + 1),
            None => ("", "", 0),
        };
        let env_key = |k: &str| {
            let u = k.to_ascii_uppercase();
            // An env-style name (SNAKE_CASE or UPPER), or the secret word itself.
            let named = k.contains('_') || k.bytes().all(|c| !c.is_ascii_lowercase()) || matches!(u.as_str(), "TOKEN" | "KEY" | "SECRET" | "PASSWORD" | "PASS" | "PASSWD" | "PWD" | "APIKEY");
            named && !k.is_empty() && k.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-')) && ["SECRET", "TOKEN", "KEY", "PASS", "PWD"].iter().any(|n| u.contains(n))
        };
        if env_key(key) && !val.is_empty() {
            out.push(val_at..at + w.len());
        }
        if let Some(k) = w.strip_suffix(':')
            && env_key(k)
            && (k.contains('_') || k.bytes().all(|c| !c.is_ascii_lowercase()))
        {
            let v0 = at + w.len();
            let v0 = v0 + (s[v0..].len() - s[v0..].trim_start().len());
            let end = word_end(s, v0);
            if end > v0 {
                out.push(v0..end);
            }
        }
        // A JSON Web Token: three base64url parts, the first `eyJ`.
        if let Some(p) = w.find("eyJ") {
            let t = w[p..].trim_end_matches(URL_TAIL);
            let parts: Vec<&str> = t.split('.').collect();
            let b64 = |x: &str| x.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'));
            if parts.len() == 3 && parts[0].len() >= 10 && parts[1].len() >= 4 && parts.iter().all(|x| b64(x)) {
                out.push(at + p..at + p + t.len());
            }
        }
    }
    out.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    out
}

/// A date (2024-10-12, 12.10.2024, 10/12/2024): never a phone number.
fn date_like(t: &str) -> bool {
    let groups: Vec<&str> = t.split(['-', '.', '/']).collect();
    groups.len() == 3
        && groups.iter().all(|g| g.bytes().all(|c| c.is_ascii_digit()))
        && matches!(groups.iter().map(|g| g.len()).collect::<Vec<_>>()[..], [4, 1..=2, 1..=2] | [1..=2, 1..=2, 4] | [1..=2, 1..=2, 2])
}

/// `+` and 7 to 15 digits (E.164, separators allowed), or 7 to 15 digits
/// in groups (spaces, dashes, dots, a bracketed area code): never a bare
/// number, a date, or a decimal.
fn phones(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let lead = c == b'+' || c == b'(' || c.is_ascii_digit();
        if !lead || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.' || b[i - 1] == b'+')) {
            i += 1;
            continue;
        }
        let start = i;
        let plus = c == b'+';
        let mut j = if plus { i + 1 } else { i };
        let mut digits = 0;
        let mut seps = 0;
        let mut end = j;
        let mut last_sep = true;
        let mut opens = 0;
        while j < b.len() {
            let ch = b[j];
            if ch.is_ascii_digit() {
                digits += 1;
                j += 1;
                end = j;
                last_sep = false;
            } else if ch == b'(' && opens == 0 {
                opens += 1;
                seps += 1;
                j += 1;
                last_sep = true;
            } else if ch == b')' && opens == 1 && !last_sep {
                opens += 1;
                seps += 1;
                j += 1;
                end = j;
                // "(415) 555-2671": a space or a dash may follow.
                last_sep = false;
            } else if matches!(ch, b' ' | b'-' | b'.') && !last_sep && j + 1 < b.len() && (b[j + 1].is_ascii_digit() || b[j + 1] == b'(') {
                seps += 1;
                j += 1;
                last_sep = true;
            } else {
                break;
            }
        }
        let tail_word = end < b.len() && b[end].is_ascii_alphanumeric();
        let text = &s[start..end];
        let groups: Vec<&str> = text.split(|c: char| !c.is_ascii_digit()).filter(|g| !g.is_empty()).collect();
        // Dots alone separate a decimal or a version unless they group
        // like a number (555.123.4567).
        let dots_only = text.contains('.') && !text.contains([' ', '-', '(']);
        let dotted_ok = !dots_only || (groups.len() >= 3 && groups.iter().all(|g| (2..=4).contains(&g.len())));
        let ok = (7..=15).contains(&digits) && (plus || seps > 0) && !tail_word && !date_like(text) && dotted_ok && (plus || groups.len() >= 2);
        if ok {
            out.push(start..end);
        }
        // Past the whole run: a part of a longer number is no phone.
        i = end.max(start + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{Line, Word};

    fn kinds(s: &str) -> Vec<(Kind, &str)> {
        find_in_text(s, &Kind::ALL).into_iter().map(|(k, r)| (k, &s[r])).collect()
    }

    fn one(s: &str, k: Kind) -> String {
        let f = kinds(s);
        assert_eq!(f.len(), 1, "{s:?}: {f:?}");
        assert_eq!(f[0].0, k, "{s:?}: {f:?}");
        f[0].1.to_string()
    }

    fn none(s: &str) {
        assert_eq!(kinds(s), Vec::<(Kind, &str)>::new(), "{s:?}");
    }

    #[test]
    fn emails() {
        assert_eq!(one("Mail jane.doe+news@example.co.uk today", Kind::Email), "jane.doe+news@example.co.uk");
        assert_eq!(one("<a.b@c-d.io>.", Kind::Email), "a.b@c-d.io");
        assert_eq!(one("ops@rustshot.dev.", Kind::Email), "ops@rustshot.dev");
        for s in ["@handle on social", "me@localhost", "a@b.c", "x@.com", "x@y..com", "just text"] {
            none(s);
        }
    }

    #[test]
    fn phones() {
        assert_eq!(one("Call +1 415 555 2671 now", Kind::Phone), "+1 415 555 2671");
        assert_eq!(one("+442071838750", Kind::Phone), "+442071838750");
        assert_eq!(one("tel (415) 555-2671", Kind::Phone), "(415) 555-2671");
        assert_eq!(one("555-123-4567", Kind::Phone), "555-123-4567");
        assert_eq!(one("030 1234567", Kind::Phone), "030 1234567");
        assert_eq!(one("555.123.4567", Kind::Phone), "555.123.4567");
        assert_eq!(one("+49-30-901820", Kind::Phone), "+49-30-901820");
        for s in [
            "2024-10-12",
            "12.10.2024",
            "10/12/2024",
            "total 1234567",
            "v1.2.3",
            "3.14159",
            "1,234,567",
            "+12345",
            "123-45",
            "page 12 of 30",
            "ID 12345678",
            "order 12-34x",
            "1.234.567,89",
        ] {
            none(s);
        }
    }

    #[test]
    fn cards() {
        assert_eq!(one("Visa 4111 1111 1111 1111 exp", Kind::Card), "4111 1111 1111 1111");
        assert_eq!(one("5500-0000-0000-0004", Kind::Card), "5500-0000-0000-0004");
        assert_eq!(one("378282246310005", Kind::Card), "378282246310005");
        assert_eq!(one("6011111111111117", Kind::Card), "6011111111111117");
        assert!(luhn("79927398713"));
        assert!(!luhn("79927398710"));
        // Fails Luhn: a phone at most, not a card (16 digits is too many).
        none("4111 1111 1111 1112");
        none("0000 0000 0000 0000");
        none("12345678901234567890123");
    }

    #[test]
    fn ibans() {
        assert_eq!(one("IBAN: DE89 3704 0044 0532 0130 00", Kind::Id), "DE89 3704 0044 0532 0130 00");
        assert_eq!(one("GB82WEST12345698765432", Kind::Id), "GB82WEST12345698765432");
        assert_eq!(one("to FR14 2004 1010 0505 0001 3M02 606 thanks", Kind::Id), "FR14 2004 1010 0505 0001 3M02 606");
        assert!(iban_ok("NL91ABNA0417164300"));
        assert!(!iban_ok("NL91ABNA0417164301"));
        none("DE00 1234 5678 9012 3456 78");
        none("AB12 short");
    }

    #[test]
    fn ips() {
        assert_eq!(one("host 192.168.1.20 up", Kind::Ip), "192.168.1.20");
        assert_eq!(one("10.0.0.1:8080", Kind::Ip), "10.0.0.1:8080");
        assert_eq!(one("(8.8.8.8)", Kind::Ip), "8.8.8.8");
        for s in ["256.1.1.1", "1.2.3", "1.2.3.4.5", "v10.0.0.1", "version 1.2.3.4a"] {
            none(s);
        }
    }

    #[test]
    fn token_urls_and_labels() {
        assert_eq!(one("see https://api.example.com/v1?user=me&token=abc123 ok", Kind::Url), "https://api.example.com/v1?user=me&token=abc123");
        assert_eq!(one("www.site.org/x?api_key=XYZ.", Kind::Url), "www.site.org/x?api_key=XYZ");
        assert_eq!(one("s3.amazonaws.com https://b.s3.aws.com/f?X-Amz-Signature=ff&sig=1", Kind::Url), "https://b.s3.aws.com/f?X-Amz-Signature=ff&sig=1");
        assert_eq!(one("https://x.io/cb#access_token=eyJ", Kind::Url), "https://x.io/cb#access_token=eyJ");
        none("https://example.com/search?q=rust&page=2");
        none("https://example.com/?token=");
        assert_eq!(one("Password: hunter2", Kind::Url), "hunter2");
        assert_eq!(one("api key = sk_live_123", Kind::Url), "sk_live_123");
        assert_eq!(one("API_KEY:abc", Kind::Url), "abc");
        assert_eq!(one("token=deadbeef", Kind::Url), "deadbeef");
        none("Forgot your password?");
        none("passwords must be long");
    }

    #[test]
    fn kinds_can_be_left_out_and_overlaps_go_to_the_first() {
        let s = "a@b.com 4111111111111111 10.1.2.3";
        assert_eq!(find_in_text(s, &[Kind::Ip]).len(), 1);
        assert!(find_in_text(s, &[]).is_empty());
        let all = kinds(s);
        assert_eq!(all.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [Kind::Email, Kind::Card, Kind::Ip]);
        // A card is no phone too.
        assert_eq!(kinds("4111 1111 1111 1111").len(), 1);
    }

    #[test]
    fn config_names_round_trip() {
        assert_eq!(parse_kinds("all"), Kind::ALL.to_vec());
        assert_eq!(parse_kinds("ips, Emails,bogus,ips"), vec![Kind::Email, Kind::Ip]);
        assert!(parse_kinds("").is_empty());
        assert_eq!(kinds_text(&Kind::ALL), "all");
        assert_eq!(kinds_text(&[Kind::Ip, Kind::Email]), "emails,ips");
        assert_eq!(kinds_text(&[]), "");
    }

    /// URLs in typographic quotes and after non-ASCII letters used to
    /// slice mid-character (a panic: the process aborts in release).
    #[test]
    fn unicode_around_urls_never_panics() {
        assert_eq!(one("\u{201c}https://x.io/?token=abc\u{201d}", Kind::Url), "https://x.io/?token=abc");
        assert_eq!(one("\u{ab}https://x.io/a?token=abc\u{bb}", Kind::Url), "https://x.io/a?token=abc");
        // No ASCII scheme before "://": no URL, the token label still holds.
        assert_eq!(one("\u{fc}://x?token=a", Kind::Url), "a");
    }

    /// Random text of mixed scripts, emoji, combining marks, quotes and
    /// the matchers' own punctuation: no panic, every range on char
    /// boundaries, and `find` maps them to words.
    #[test]
    fn fuzzed_unicode_never_panics() {
        const POOL: &[&str] = &[
            "a", "Z", "0", "7", "4111", " ", "  ", ".", ",", ":", "=", "/", "?", "&", "#", "@", "+", "-", "_", "(", ")", "\u{201c}", "\u{201d}", "\u{ab}", "\u{bb}", "\u{fc}",
            "\u{e9}", "\u{0438}", "\u{4f60}", "\u{597d}", "\u{1f600}", "\u{1f469}\u{200d}\u{1f4bb}", "\u{301}", "\u{5d0}", "\u{627}", "\u{e01}", "\u{fe0f}", "\t", "https://", "www.", "token=", "password:",
            "Bearer ", "eyJ", "api key", "SECRET_KEY=", "DE89", "192.168.", "+1 ", "@b\u{fc}cher.de", "hooks.slack.com/services/",
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let n = (next() % 40) as usize;
            let s: String = (0..n).map(|_| POOL[(next() % POOL.len() as u64) as usize]).collect();
            for (_, r) in find_in_text(&s, &Kind::ALL) {
                assert!(s.is_char_boundary(r.start) && s.is_char_boundary(r.end) && r.start < r.end, "{s:?}: {r:?}");
            }
            let words: Vec<&str> = s.split(' ').filter(|w| !w.is_empty()).collect();
            let _ = find(&[line(&words)], &Kind::ALL);
        }
    }

    #[test]
    fn more_secrets() {
        assert_eq!(one("Authorization: Bearer eyJhbGciOi.eyJzdWIi.c2ln", Kind::Url), "Bearer eyJhbGciOi.eyJzdWIi.c2ln");
        assert_eq!(one("curl -H bearer abcdef1234567890", Kind::Url), "abcdef1234567890");
        assert_eq!(one("jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sflKxwRJSMeKKF2QT4fw", Kind::Url), "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sflKxwRJSMeKKF2QT4fw");
        assert_eq!(one("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG", Kind::Url), "wJalrXUtnFEMI/K7MDENG");
        assert_eq!(one("export OPENAI_KEY=sk-abc123", Kind::Url), "sk-abc123");
        assert_eq!(one("db_pass=hunter2", Kind::Url), "hunter2");
        assert_eq!(one("GITHUB_TOKEN: ghp_abcdef", Kind::Url), "ghp_abcdef");
        assert_eq!(one("Password: correct horse battery staple", Kind::Url), "correct horse battery staple");
        assert_eq!(one("Passphrase: open sesame now", Kind::Url), "open sesame now");
        assert_eq!(one("https://hooks.slack.com/services/T000/B000/XXXX", Kind::Url), "https://hooks.slack.com/services/T000/B000/XXXX");
        assert_eq!(one("https://discord.com/api/webhooks/123/abc", Kind::Url), "https://discord.com/api/webhooks/123/abc");
        assert_eq!(one("see https://files.example.com/d/a8f3k29dk3l2m4n5b6v7c8x9z0q1w2", Kind::Url), "https://files.example.com/d/a8f3k29dk3l2m4n5b6v7c8x9z0q1w2");
        assert_eq!(one("https://app.example.com/home?session=abc123", Kind::Url), "https://app.example.com/home?session=abc123");
        assert_eq!(one("https://a.io/?sid=9f8e7d", Kind::Url), "https://a.io/?sid=9f8e7d");
        assert_eq!(one("https://a.io/x?auth=zzz", Kind::Url), "https://a.io/x?auth=zzz");
        assert_eq!(one("example.com/x?key=abc", Kind::Url), "example.com/x?key=abc");
        assert_eq!(one("write to j\u{f6}hn@b\u{fc}cher.de", Kind::Email), "j\u{f6}hn@b\u{fc}cher.de");
        // Not secrets: a slug, a plain path, words.
        none("https://blog.example.com/how-to-set-up-your-account-today-please");
        none("https://example.com/docs/getting-started");
        none("monkey=banana");
        none("Keyboard: US");
    }

    fn line(words: &[&str]) -> Line {
        Line { words: words.iter().enumerate().map(|(i, t)| Word { text: (*t).into(), x: i as f32 * 50.0, y: 0.0, w: 40.0, h: 12.0 }).collect() }
    }

    #[test]
    fn hits_name_the_words() {
        let lines = [line(&["Contact:", "jane@example.com"]), line(&["Card", "4111", "1111", "1111", "1111", "thanks"]), line(&["nothing", "here"])];
        let h = find(&lines, &Kind::ALL);
        assert_eq!(h, vec![Hit { kind: Kind::Email, line: 0, words: 1..2 }, Hit { kind: Kind::Card, line: 1, words: 1..5 }]);
        assert!(find(&lines, &[Kind::Phone]).is_empty());
    }
}
