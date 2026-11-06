use assert_cmd::Command;
use serde_json::Value;

fn init(dir: &std::path::Path, extra: &[&str]) -> Value {
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["init", "--format", "json", "--project"])
        .arg(dir)
        .args(extra)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn init_creates_the_layout_and_a_valid_template() {
    let dir = tempfile::tempdir().unwrap();
    let v = init(dir.path(), &[]);
    assert_eq!(v["schema"], "evalsig.init/1");
    assert_eq!(v["results"]["config"]["state"], "written");
    for sub in ["objects", "reports"] {
        assert!(dir.path().join(".evalsig").join(sub).is_dir(), "{sub}");
    }
    let text = std::fs::read_to_string(dir.path().join(".evalsig/config.toml")).unwrap();
    evalsig_core::config::Config::from_toml(&text).unwrap();
    assert!(text.contains("# "), "template is commented");
}

#[test]
fn init_is_idempotent_and_keeps_an_existing_config() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &[]);
    let config = dir.path().join(".evalsig/config.toml");
    std::fs::write(&config, "[project]\nname = \"mine\"\n").unwrap();
    let v = init(dir.path(), &[]);
    assert_eq!(v["results"]["config"]["state"], "kept");
    assert_eq!(v["results"]["created"], serde_json::json!([]));
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "[project]\nname = \"mine\"\n"
    );
    let v = init(dir.path(), &["--force"]);
    assert_eq!(v["results"]["config"]["state"], "overwritten");
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("my-evaluation")
    );
}
