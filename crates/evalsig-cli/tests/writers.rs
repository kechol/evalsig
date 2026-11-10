//! The writers printed by `schema records --emit` produce records that validate.

use std::path::Path;
use std::process::Command as Process;

use assert_cmd::Command;

fn emit(lang: &str) -> Vec<u8> {
    Command::cargo_bin("evalsig")
        .unwrap()
        .args(["schema", "records", "--emit", lang])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

fn validates(records: &Path) {
    Command::cargo_bin("evalsig")
        .unwrap()
        .arg("validate")
        .arg("--records")
        .arg(records)
        .assert()
        .success();
}

#[test]
fn the_python_writer_produces_valid_records() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("evalsig_records.py"), emit("python")).unwrap();
    let script = dir.path().join("write.py");
    std::fs::write(
        &script,
        r#"import sys
sys.path.insert(0, sys.argv[1])
from evalsig_records import Writer, missing
with open(sys.argv[2], "w", encoding="utf-8") as f:
    w = Writer(f)
    w.run("r1", system_id="triage", system_version="v1", output_type="ordinal", levels=4,
          features={"R10-1": {"expected": "positive", "lineage": "R10"}})
    w.run("r2", system_id="grader", system_version="v1", output_type="score", value_semantics="score", grader={"id": "rubric"})
    w.item("t-1", cluster="c-1", strata={"lang": "en"}, occurred_at="2026-09-01T00:00:00Z")
    w.label("t-1", rater_id="rater:a", source="human", value=True)
    w.judgment("t-1", "r1", gen_replicate=0, output_type="ordinal", levels=4, value=3)
    w.judgment("t-1", "r1", gen_replicate=1, output_type="ordinal", levels=4, missing=missing("timeout", "60 s"))
    w.judgment("t-1", "r2", gen_replicate=0, output_type="score", value=0.75)
    w.attribution("t-1", "r1", rater_id="agent:review", causes=["needs_external_info", "no_rule"])
    try:
        w.judgment("t-1", "r1", gen_replicate=2, output_type="ordinal", value=1, missing=missing("error"))
    except ValueError:
        pass
    else:
        raise SystemExit("value and missing together were accepted")
"#,
    )
    .unwrap();
    let out = dir.path().join("records.jsonl");
    let status = Process::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(dir.path())
        .arg(&out)
        .status()
        .expect("python3 runs");
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&out).unwrap().lines().count(), 8);
    validates(&out);
}

/// Runs where Node strips TypeScript types (22.6 and later); elsewhere it reports and
/// returns.
#[test]
fn the_typescript_writer_produces_valid_records() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("evalsig_records.ts"), emit("typescript")).unwrap();
    let main = dir.path().join("main.ts");
    std::fs::write(
        &main,
        r#"import { RecordsWriter, missing } from "./evalsig_records.ts";
import { writeFileSync } from "node:fs";
const lines: string[] = [];
const w = new RecordsWriter((line) => lines.push(line));
w.run({ runId: "r1", systemId: "triage", systemVersion: "v1", outputType: "binary" });
w.item({ itemId: "t-1", cluster: "c-1" });
w.label({ itemId: "t-1", raterId: "rater:a", source: "human", value: true });
w.judgment({ itemId: "t-1", runId: "r1", genReplicate: 0, outputType: "binary", value: false });
w.judgment({ itemId: "t-1", runId: "r1", genReplicate: 1, outputType: "binary", missing: missing("error") });
w.attribution({ itemId: "t-1", runId: "r1", raterId: "agent:review", causes: ["fixable"] });
writeFileSync(process.argv[2], lines.join(""));
"#,
    )
    .unwrap();
    let node = |script: &Path| {
        Process::new("node")
            .args(["--experimental-strip-types", "--no-warnings"])
            .arg(script)
            .arg(dir.path().join("records.jsonl"))
            .status()
    };
    let probe = dir.path().join("probe.ts");
    std::fs::write(&probe, "const n: number = 1;\n").unwrap();
    if !node(&probe).is_ok_and(|s| s.success()) {
        eprintln!("node with type stripping is not available; skipped");
        return;
    }
    assert!(node(&main).unwrap().success());
    let out = dir.path().join("records.jsonl");
    assert_eq!(std::fs::read_to_string(&out).unwrap().lines().count(), 6);
    validates(&out);
}
