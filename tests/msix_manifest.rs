//! The Store package's manifest (packaging/windows/AppxManifest.xml.in):
//! what Partner Center's package validation accepts. 0.6.5's package was
//! refused for a hidden application (AppListEntry="none", a "headless"
//! app, which needs the HeadlessAppBypass waiver).

const MANIFEST: &str = include_str!("../packaging/windows/AppxManifest.xml.in");

/// The manifest without its `<!-- -->` comments.
fn body() -> String {
    let mut s = MANIFEST;
    let mut out = String::new();
    while let Some(i) = s.find("<!--") {
        out.push_str(&s[..i]);
        let j = s[i..].find("-->").expect("an unclosed comment") + i + 3;
        s = &s[j..];
    }
    out.push_str(s);
    out
}

/// Each `<Application ...>...</Application>` element.
fn applications(body: &str) -> Vec<&str> {
    let mut apps = Vec::new();
    let mut s = body;
    while let Some(i) = s.find("<Application ") {
        let j = s[i..].find("</Application>").expect("an unclosed Application") + i;
        apps.push(&s[i..j]);
        s = &s[j..];
    }
    apps
}

fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let i = element.find(&key)? + key.len();
    Some(&element[i..i + element[i..].find('"')?])
}

#[test]
fn no_application_is_hidden_from_start() {
    assert!(!body().contains("AppListEntry"), "Partner Center refuses AppListEntry=\"none\" without the HeadlessAppBypass waiver");
}

#[test]
fn rustshot_and_rustshot_settings_are_the_applications() {
    let b = body();
    let apps = applications(&b);
    let ids: Vec<_> = apps.iter().map(|a| attr(a, "Id").unwrap()).collect();
    assert_eq!(ids, ["Rustshot", "RustshotSettings"]);
    // A plain launch (no arguments) is the daemon; Settings passes `settings`.
    assert_eq!(attr(apps[0].split('>').next().unwrap(), "uap10:Parameters"), None);
    assert_eq!(attr(apps[1], "uap10:Parameters"), Some("settings"));
}

#[test]
fn the_startup_task_is_on_rustshot_with_0_6_5s_task_id() {
    let b = body();
    let apps = applications(&b);
    assert_eq!(b.matches("windows.startupTask").count(), 1, "one startup task");
    let main = apps[0];
    let i = main.find("<desktop:StartupTask ").expect("the startup task on the Rustshot application");
    let task = &main[i..i + main[i..].find("/>").unwrap()];
    // The TaskId keeps the user's on/off choice across the update from 0.6.5.
    assert_eq!(attr(task, "TaskId"), Some("RustshotDaemon"));
    assert_eq!(attr(task, "Enabled"), Some("true"));
    assert_eq!(attr(task, "DisplayName"), Some("Rustshot hotkeys"));
    // The listing's certification notes name it.
    let listing = include_str!("../docs/store/msstore-listing.md");
    assert!(listing.contains("The startup task \"Rustshot hotkeys\" starts the background process at sign-in"));
}
