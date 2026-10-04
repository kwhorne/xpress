//! `xpress history` against a history in a temp folder (never the user's,
//! and `copy` is left out so the clipboard is never touched).

use std::path::Path;
use std::process::{Command, Output};

use xpress_core::history::{History, NewClip, Rule};

fn xpress(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xpress"))
        .args(args)
        .env("HOME", home)
        .env("XPRESS_HISTORY_DIR", home.join("history"))
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn json(out: &Output) -> Vec<serde_json::Value> {
    serde_json::from_str(&stdout(out)).unwrap()
}

#[test]
fn search_show_pin_and_delete() {
    let home = tempfile::tempdir().unwrap();
    let (invoice, link) = {
        let h = History::open(&home.path().join("history")).unwrap();
        let invoice = h
            .add(NewClip::text("Invoice 4711 is paid").from_app(Some("Mail".into()), None))
            .unwrap()
            .id;
        let link = h
            .add(NewClip::text("https://kwhorne.com").from_app(Some("Safari".into()), None))
            .unwrap()
            .id;
        let work = h
            .save_category(None, "Work", [1, 2, 3], &Rule::default())
            .unwrap();
        h.set_category(invoice, work, true).unwrap();
        (invoice, link)
    };

    // Newest first; words; kinds; apps; categories.
    let all = json(&xpress(home.path(), &["history", "--json"]));
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["title"], "https://kwhorne.com");
    let found = json(&xpress(home.path(), &["history", "inv", "47", "--json"]));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["id"], invoice);
    assert_eq!(found[0]["categories"][0], "Work");
    let links = json(&xpress(
        home.path(),
        &["history", "search", "--kind", "links", "--json"],
    ));
    assert_eq!(links[0]["id"], link);
    let mail = json(&xpress(
        home.path(),
        &["history", "--app", "Mail", "--json"],
    ));
    assert_eq!(mail.len(), 1);
    let work = json(&xpress(
        home.path(),
        &["history", "--category", "work", "--json"],
    ));
    assert_eq!(work[0]["id"], invoice);

    let human = stdout(&xpress(home.path(), &["history"]));
    assert!(
        human.contains("Invoice 4711 is paid") && human.contains("Safari"),
        "{human}"
    );

    assert_eq!(
        stdout(&xpress(
            home.path(),
            &["history", "show", &invoice.to_string()]
        )),
        "Invoice 4711 is paid\n"
    );

    stdout(&xpress(home.path(), &["history", "pin", &link.to_string()]));
    let pinned = json(&xpress(home.path(), &["history", "--pinned", "--json"]));
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0]["pinned"], true);

    let cats = json(&xpress(home.path(), &["history", "categories", "--json"]));
    assert_eq!(cats[0]["name"], "Work");
    assert_eq!(cats[0]["count"], 1);

    stdout(&xpress(
        home.path(),
        &["history", "delete", &invoice.to_string()],
    ));
    assert_eq!(json(&xpress(home.path(), &["history", "--json"])).len(), 1);

    // Unknown clips and categories are clear errors.
    let missing = xpress(home.path(), &["history", "show", "999"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no clip 999"));
    let nocat = xpress(home.path(), &["history", "--category", "Nope"]);
    assert!(String::from_utf8_lossy(&nocat.stderr).contains("no category"));
}

#[test]
fn without_a_history_it_says_how_to_start_one() {
    let home = tempfile::tempdir().unwrap();
    let out = xpress(home.path(), &["history"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("turn it on in the xpress app"));
    assert!(!home.path().join("history").exists(), "nothing is created");
}
