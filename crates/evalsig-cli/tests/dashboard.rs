//! `evalsig dashboard` (SPEC §11, D-019): loopback only, a per-launch token, the
//! Host header checked, and pages from the CLI's own results.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Stdio};

use serde_json::Value;

mod common;
use common::*;

pub struct Dashboard {
    child: Child,
    pub url: String,
    pub port: u16,
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn start(project: &std::path::Path) -> Dashboard {
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("evalsig"))
        .env_remove("AI_AGENT")
        .arg("--project")
        .arg(project)
        .args(["dashboard", "--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    let url = v["url"].as_str().unwrap().to_owned();
    let port = url
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    Dashboard { child, url, port }
}

pub fn get(port: u16, path: &str, host: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn the_dashboard_serves_its_token_and_host_only() {
    let p = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let d = start(p.path());
    let path = d
        .url
        .trim_start_matches(&format!("http://127.0.0.1:{}", d.port));
    let host = format!("127.0.0.1:{}", d.port);
    let page = get(d.port, path, &host);
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("triage-v1") && page.contains("triage-v2"));
    assert!(page.contains("Content-Security-Policy: default-src 'none'"));
    let foreign = get(d.port, path, &format!("rebind.example.com:{}", d.port));
    assert!(foreign.starts_with("HTTP/1.1 421"), "{foreign}");
    assert!(!foreign.contains("triage"));
    let tokenless = get(d.port, "/", &host);
    assert!(tokenless.starts_with("HTTP/1.1 403"), "{tokenless}");
    assert!(!tokenless.contains("triage"));
    // The dashboard writes nothing to the project.
    let ledger = std::fs::read_to_string(p.path().join(".evalsig/ledger.jsonl")).unwrap();
    let _ = get(d.port, path, &host);
    assert_eq!(
        std::fs::read_to_string(p.path().join(".evalsig/ledger.jsonl")).unwrap(),
        ledger
    );
}

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");

#[test]
fn every_screen_renders_for_the_examples() {
    // The synthetic triage example and the ordinal example with times.
    let synthetic = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let ordinal = tempfile::tempdir().unwrap();
    evalsig(ordinal.path()).arg("init").assert().success();
    std::fs::write(
        ordinal.path().join(".evalsig/config.toml"),
        "[project]\nname = \"example\"\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n[stats]\nseed = 20261029\n",
    )
    .unwrap();
    evalsig(ordinal.path())
        .arg("import")
        .arg(EXAMPLE)
        .assert()
        .success();
    for p in [synthetic.path(), ordinal.path()] {
        let d = start(p);
        let base = d
            .url
            .trim_start_matches(&format!("http://127.0.0.1:{}", d.port))
            .to_owned();
        let host = format!("127.0.0.1:{}", d.port);
        let before = std::fs::read_to_string(p.join(".evalsig/ledger.jsonl")).unwrap();
        for (screen, expect) in [
            ("", "<h1>Overview</h1>"),
            ("compare", "<h1>Comparison</h1>"),
            ("trend", "<h1>Trend</h1>"),
            ("reproducibility", "<h1>Reproducibility</h1>"),
            ("reference", "<h1>Reference quality</h1>"),
            ("discrimination", "<h1>Discrimination</h1>"),
        ] {
            let page = get(d.port, &format!("{base}{screen}"), &host);
            assert!(page.starts_with("HTTP/1.1 200"), "{screen}: {page}");
            assert!(
                page.contains(expect),
                "{screen}: {}",
                &page[..page.len().min(2000)]
            );
            assert!(!page.contains("<h1>Error</h1>"), "{screen}: {page}");
            assert!(
                !page.contains("http://") || page.contains("xmlns=\"http://www.w3.org/2000/svg\"")
            );
        }
        // The comparison shows the CLI's verdict and the overview its intervals.
        let page = get(d.port, &format!("{base}compare"), &host);
        assert!(page.contains("class=\"verdict\""), "{page}");
        assert!(get(d.port, &base, &host).contains("data-value="));
        assert_eq!(
            std::fs::read_to_string(p.join(".evalsig/ledger.jsonl")).unwrap(),
            before,
            "the dashboard wrote to the ledger"
        );
    }
}
