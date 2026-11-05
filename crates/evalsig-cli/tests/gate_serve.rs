//! `gate serve` and `gate submit` (SPEC §8.5, D-014) on one host: the Unix socket and
//! token HTTP answer what `gate` answers, the full result stays with the referee, and
//! a wrong token gets nothing. `scripts/gate_serve_docker.sh` runs the same across
//! two OS users.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};

use serde_json::Value;

mod common;
use common::*;

const GATE: &str = r#"name = "triage-v2 is no worse"
hypothesis = "The new prompt keeps accuracy"
split = "holdout"
min_items = 10

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "not_worse_than"
value = -0.3
sided = "one"

[holdout]
budget = 3
"#;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `gate serve --http 0` and read where it listens.
fn serve(w: &World) -> (Server, Value) {
    let bin = assert_cmd::cargo::cargo_bin("evalsig");
    let mut child = std::process::Command::new(bin)
        .env("EVALSIG_ACTOR", "user:referee")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(w.referee.path())
        .args(["gate", "serve", "--http", "0", "--referee"])
        .arg(w.home.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    (Server(child), serde_json::from_str(&line).unwrap())
}

fn submit_cmd(w: &World, records: &Path, how: &[&str]) -> std::process::Output {
    evalsig(w.agent.path())
        .args([
            "--format",
            "json",
            "gate",
            "submit",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(records)
        .arg("--agent-ledger")
        .arg(w.agent.path().join(".evalsig/ledger.jsonl"))
        .args(how)
        .output()
        .unwrap()
}

// The Unix socket half exists only where Unix sockets do.
#[cfg(unix)]
#[test]
fn submissions_over_the_socket_and_http_get_the_gate_answer() {
    let w = world(GATE, None);
    let (_server, at) = serve(&w);
    let socket = at["socket"].as_str().unwrap().to_owned();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(Path::new(&socket)), 0o600);
        // Only the referee's user reads its ledger and enters its directories.
        assert_eq!(mode(&w.home.path().join("ledger.jsonl")), 0o600);
        assert_eq!(mode(&w.home.path().join("identity.txt")), 0o600);
        assert_eq!(mode(&w.home.path().join("token")), 0o600);
        assert_eq!(mode(w.home.path()), 0o700);
        for sub in ["holdouts", "sealed", "approved", "results", "state"] {
            assert_eq!(mode(&w.home.path().join(sub)), 0o700, "{sub}");
        }
    }

    let out = submit_cmd(&w, &variant(w.agent.path(), 1), &["--socket", &socket]);
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["verdict"]["code"], "pass");
    assert_eq!(v["results"]["budget"]["used"], 1);
    assert_eq!(v["provenance"]["ledger"], "referee");

    let url = at["http"].as_str().unwrap().to_owned();
    let token = at["token_file"].as_str().unwrap().to_owned();
    let out = submit_cmd(
        &w,
        &variant(w.agent.path(), 2),
        &["--url", &url, "--token-file", &token],
    );
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json(&out)["results"]["budget"]["used"], 2);

    // A wrong token is turned away before anything is judged.
    let wrong = w.agent.path().join("wrong-token");
    std::fs::write(&wrong, "0".repeat(64)).unwrap();
    let out = submit_cmd(
        &w,
        &variant(w.agent.path(), 3),
        &["--url", &url, "--token-file", wrong.to_str().unwrap()],
    );
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "gate_transport");

    // An answer the referee refused comes back with its exit code.
    let out = submit_cmd(
        &w,
        &candidate(w.agent.path(), "grader.jsonl", |rows| {
            rows[0]["grader"]["version"] = "9".into()
        }),
        &["--socket", &socket],
    );
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");

    // The referee keeps the results of judged submissions and copies the agent's ledger
    // head into each gate line.
    let gates: Vec<Value> = referee_ledger(&w)
        .into_iter()
        .filter(|e| e["kind"] == "gate")
        .collect();
    // A refusal after the submission counts leaves a line too, with its code.
    assert_eq!(gates.len(), 3);
    assert!(gates[0]["summary"]["agent_ledger_head"].is_string());
    assert_eq!(gates[2]["summary"]["error"], "does_not_match_approval");
    assert_eq!(accesses(&w), 3);
    assert_eq!(
        std::fs::read_dir(w.home.path().join("results"))
            .unwrap()
            .count(),
        2
    );
}

/// Token HTTP alone, as on platforms without Unix sockets.
#[test]
fn a_submission_over_token_http_gets_the_gate_answer() {
    let w = world(GATE, None);
    let (_server, at) = serve(&w);
    let url = at["http"].as_str().unwrap().to_owned();
    let token = at["token_file"].as_str().unwrap().to_owned();
    let out = submit_cmd(
        &w,
        &variant(w.agent.path(), 1),
        &["--url", &url, "--token-file", &token],
    );
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json(&out)["verdict"]["code"], "pass");
    assert_eq!(accesses(&w), 1);
}
