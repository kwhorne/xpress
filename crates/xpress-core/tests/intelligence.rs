//! Apple Intelligence through the bundled helper — against the real on-device
//! model when it's there (build it with scripts/build-xpress-ai.sh). Skipped
//! elsewhere: CI runners and Macs without Apple Intelligence.

use xpress_core::intelligence::{self, Status, Task};

fn ready() -> bool {
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xpress-ai");
    if helper.join("xpress-ai").is_file() {
        xpress_core::tools::set_bin_dir_override(helper);
    }
    let status = intelligence::status();
    if status != Status::Available {
        eprintln!("skipping: Apple Intelligence is {status:?}");
    }
    status == Status::Available
}

#[test]
fn proofreads_with_the_on_device_model() {
    if !ready() {
        return;
    }
    let out = intelligence::run(Task::Proofread, "Ths is a smal tset of the speling.").unwrap();
    let lower = out.to_lowercase();
    assert!(
        lower.contains("test") && lower.contains("spelling"),
        "{out:?}"
    );
    assert!(!lower.contains("tset"), "{out:?}");
}

#[test]
fn summarises_and_keeps_the_language() {
    if !ready() {
        return;
    }
    let text = "Møtet på fredag er flyttet fra klokken ti til klokken tolv fordi \
                styreleder kommer sent fra Bergen. Vi bruker samme rom som sist, \
                og lunsj blir servert etter møtet. Ta med Q3-rapporten.";
    let out = intelligence::run(Task::Summarize, text).unwrap();
    assert!(out.len() < text.len() + 40, "{out:?}");
    assert!(
        out.contains("12") || out.to_lowercase().contains("tolv"),
        "{out:?}"
    );
}

#[test]
fn empty_text_is_refused_before_asking() {
    if !ready() {
        return;
    }
    assert!(intelligence::run(Task::Summarize, "   ").is_err());
}
