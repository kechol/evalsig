//! `--format html` (SPEC §11 "Static HTML", D-019): one page that loads nothing from
//! anywhere, with the same data as `--format json` embedded.

use serde_json::Value;

mod common;
use common::*;

fn embedded(page: &str) -> Value {
    let start = page
        .find("<script type=\"application/json\" id=\"evalsig-data\">")
        .unwrap();
    let rest = &page[start..];
    let body = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
    serde_json::from_str(body).unwrap()
}

#[test]
fn the_static_report_is_self_contained_and_carries_the_json() {
    // A stratum value that tries to end the script element.
    let mut text = std::fs::read_to_string(SYNTHETIC).unwrap();
    text = text.replacen(
        "\"channel\":\"chat\"",
        "\"channel\":\"</script><script>alert(1)</script><!--\"",
        1,
    );
    let p = project(&text);
    for args in [
        vec!["report", "triage-v1", "--by", "channel"],
        vec!["compare", "triage-v1", "triage-v2"],
    ] {
        let mut c = evalsig(p.path());
        let html = c.arg("--format").arg("html").args(&args).output().unwrap();
        assert!(
            html.status.success(),
            "{}",
            String::from_utf8_lossy(&html.stderr)
        );
        let page = String::from_utf8(html.stdout).unwrap();
        assert!(page.starts_with("<!DOCTYPE html>"));
        for external in ["http://", "https://", "src=", "<link", "@import", "url("] {
            assert!(!page.contains(external), "{args:?}: {external}");
        }
        assert!(!page.contains("<script>alert"), "{args:?}");
        assert_eq!(page.matches("</script>").count(), 1, "{args:?}");
        let mut c = evalsig(p.path());
        let json = c.arg("--format").arg("json").args(&args).output().unwrap();
        let json: Value = serde_json::from_slice(&json.stdout).unwrap();
        let mut data = embedded(&page);
        let mut json = json;
        // Each comparison counts itself among the uses of the same items.
        for v in [&mut data, &mut json] {
            v["results"]
                .as_object_mut()
                .unwrap()
                .remove("validation_uses");
        }
        assert_eq!(data["results"], json["results"], "{args:?}");
        assert_eq!(data["verdict"], json["verdict"]);
        assert!(page.contains("data-value="), "{args:?}");
    }
}
