//! `evalsig export`: the data as Parquet, CSV or canonical JSONL for large analysis
//! elsewhere (D-025, SPEC §9 "Large analysis"). Exports are derived; verification
//! always uses the canonical snapshots.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use evalsig_core::records::{Label, Record};
use evalsig_core::report;
use evalsig_store::canonical::to_canonical_bytes;
use parquet::basic::{LogicalType, Repetition, Type as Physical};
use parquet::data_type::{BoolType, ByteArray, ByteArrayType, DoubleType, Int64Type};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::types::Type;
use serde_json::{Map, Value, json};

use evalsig_project::analysis::{self, Analysis};

use crate::args::AnalysisArgs;
use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, io_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum To {
    Parquet,
    Csv,
    Jsonl,
    /// The snapshot's records in one canonical file with a manifest of hashes (D-050).
    Bundle,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The file format.
    #[arg(long, value_enum)]
    pub to: To,
    /// The directory to write the files into (created if missing).
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Also this run's per-item report rows, as the table items_<run>.
    #[arg(long, value_name = "RUN")]
    pub items: Option<String>,
    /// With --to bundle: replace item IDs, clusters and strata values with a keyed hash
    /// (the key from EVALSIG_BUNDLE_KEY, never written) and drop free text.
    #[arg(long)]
    pub hash_ids: bool,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
}

/// Rows of one table as JSON objects.
struct Table {
    name: String,
    rows: Vec<Map<String, Value>>,
}

/// The table of a record type (D-025); content records describe stored bodies and
/// are not exported.
fn kind(r: &Record) -> Option<&'static str> {
    match r {
        Record::Run(_) => Some("runs"),
        Record::Item(_) => Some("items"),
        Record::Judgment(_) => Some("judgments"),
        Record::Label(_) => Some("labels"),
        Record::Preference(_) => Some("preferences"),
        Record::Attribution(_) => Some("attributions"),
        Record::Content(_) => None,
    }
}

/// A bundle's manifest (D-050), written by `export --to bundle` and checked by
/// `import --from-bundle`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Manifest {
    /// `evalsig.bundle/1`.
    pub bundle: String,
    /// The hash of `records.jsonl`, the snapshot's bytes.
    pub records: String,
    /// The hash of the configuration the records were analysed under.
    pub config: String,
    /// Whether item IDs, clusters and strata values are keyed hashes.
    pub hashed_ids: bool,
    pub tool: String,
    pub spec_version: String,
}

/// The format a bundle's manifest names.
pub(crate) const BUNDLE: &str = "evalsig.bundle/1";

/// The records in force, canonical and in primary-key order (the snapshot's bytes), and a
/// manifest naming their hash and the configuration's (D-050).
fn bundle(
    project: &Project,
    config: &evalsig_core::config::Config,
    d: &data::Data,
    args: &Args,
) -> Result<Output, Error> {
    let key = args
        .hash_ids
        .then(|| {
            std::env::var("EVALSIG_BUNDLE_KEY")
                .ok()
                // A short key could be found by trying it on guessable IDs.
                .filter(|k| k.chars().count() >= 16)
                .map(|k| blake3::derive_key("evalsig bundle ids", k.as_bytes()))
                .ok_or_else(|| {
                    Error::usage(
                        "--hash-ids needs a key of at least 16 characters in EVALSIG_BUNDLE_KEY",
                    )
                })
        })
        .transpose()?;
    let mut records: Vec<Record> = d
        .records
        .iter()
        .filter(|r| !matches!(r, Record::Content(_)))
        .cloned()
        .collect();
    if let Some(key) = &key {
        records.iter_mut().for_each(|r| pseudonymize(r, key));
    }
    let (lines, hash) = evalsig_store::rows::canonical_lines(&mut records).map_err(Error::store)?;
    evalsig_project::project::outside_store(&args.out)?;
    std::fs::create_dir_all(&args.out).map_err(|e| io_error(&args.out, &e))?;
    let records_path = args.out.join("records.jsonl");
    let write_lines = || -> std::io::Result<()> {
        let mut w = std::io::BufWriter::new(File::create(&records_path)?);
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                w.write_all(b"\n")?;
            }
            w.write_all(line)?;
        }
        w.flush()
    };
    write_lines().map_err(|e| io_error(&records_path, &e))?;
    let manifest = Manifest {
        bundle: BUNDLE.to_owned(),
        records: hash.to_string(),
        config: project.store_config(config)?.to_string(),
        hashed_ids: key.is_some(),
        tool: evalsig_project::tool_version(),
        spec_version: evalsig_core::SPEC_VERSION.to_owned(),
    };
    let manifest_path = args.out.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serializes"),
    )
    .map_err(|e| io_error(&manifest_path, &e))?;
    let mut envelope = Envelope::new(
        Schema::Export,
        json!({"format": "bundle", "records": records.len(), "manifest": manifest, "dir": args.out.display().to_string()}),
    );
    envelope.provenance.snapshots = d.snapshot_ids();
    Ok(Output::success(envelope))
}

/// Item IDs, clusters, strata values and content references become a keyed hash, the
/// same for the same value across records; free text (attributes, reasons, missing
/// details) and source pointers are dropped. Attribution causes, the raters' vocabulary,
/// are kept.
fn pseudonymize(r: &mut Record, key: &[u8; 32]) {
    let h = |s: &str| {
        format!(
            "h:{}",
            &blake3::keyed_hash(key, s.as_bytes()).to_hex()[..32]
        )
    };
    let detail = |m: &mut Option<evalsig_core::records::Missing>| {
        if let Some(m) = m {
            m.detail = None;
        }
    };
    match r {
        Record::Item(i) => {
            i.item_id = h(&i.item_id);
            // A content hash can be confirmed by guessing the text, so it is keyed too;
            // equal bodies keep equal references.
            i.input_ref = i.input_ref.as_deref().map(h);
            i.reference_ref = i.reference_ref.as_deref().map(h);
            i.cluster = i.cluster.as_deref().map(h);
            if let Some(strata) = &mut i.strata {
                for v in strata.values_mut() {
                    // As JSON text, so the number 1 and the string "1" stay apart.
                    *v = Value::String(h(&v.to_string()));
                }
            }
            i.attrs = None;
        }
        Record::Label(l) => {
            l.item_id = h(&l.item_id);
            l.reason = None;
            l.source_ref = None;
            detail(&mut l.missing);
        }
        Record::Judgment(j) => {
            j.item_id = h(&j.item_id);
            j.output_ref = j.output_ref.as_deref().map(h);
            j.rationale_ref = j.rationale_ref.as_deref().map(h);
            j.source_ref = None;
            detail(&mut j.missing);
        }
        Record::Preference(p) => {
            p.item_id = h(&p.item_id);
            p.rationale_ref = p.rationale_ref.as_deref().map(h);
            p.source_ref = None;
            detail(&mut p.missing);
        }
        Record::Attribution(a) => a.item_id = h(&a.item_id),
        Record::Run(r) => r.attrs = None,
        Record::Content(_) => unreachable!("content records are not bundled"),
    }
}

/// A run ID as a file-name part: letters, digits, `-`, `_` and `.` kept, anything
/// else (path separators included) replaced by `_`, and no leading dot.
fn file_part(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    match s.strip_prefix('.') {
        Some(rest) => format!("_{rest}"),
        None => s,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Column {
    Bool,
    Int,
    Float,
    Text,
}

fn column_type(rows: &[Map<String, Value>], name: &str) -> Column {
    let values: Vec<&Value> = rows
        .iter()
        .filter_map(|r| r.get(name))
        .filter(|v| !v.is_null())
        .collect();
    if !values.is_empty() && values.iter().all(|v| v.is_boolean()) {
        Column::Bool
    } else if !values.is_empty() && values.iter().all(|v| v.is_i64()) {
        Column::Int
    } else if !values.is_empty() && values.iter().all(|v| v.is_number()) {
        Column::Float
    } else {
        Column::Text
    }
}

/// A cell as text: strings as they are, other values as canonical JSON.
fn text(v: &Value) -> Result<String, Error> {
    Ok(match v {
        Value::String(s) => s.clone(),
        other => String::from_utf8(to_canonical_bytes(other).map_err(Error::store)?)
            .expect("canonical JSON is UTF-8"),
    })
}

fn columns(t: &Table) -> Vec<String> {
    let set: BTreeSet<&String> = t.rows.iter().flat_map(|r| r.keys()).collect();
    set.into_iter().cloned().collect()
}

fn write_parquet(t: &Table, path: &Path) -> Result<(), Error> {
    let fail =
        |e: parquet::errors::ParquetError| Error::new("export", e.to_string(), Exit::Rejected);
    let names = columns(t);
    let types: Vec<Column> = names.iter().map(|n| column_type(&t.rows, n)).collect();
    let fields = names
        .iter()
        .zip(&types)
        .map(|(n, c)| {
            let (physical, logical) = match c {
                Column::Bool => (Physical::BOOLEAN, None),
                Column::Int => (Physical::INT64, None),
                Column::Float => (Physical::DOUBLE, None),
                Column::Text => (Physical::BYTE_ARRAY, Some(LogicalType::String)),
            };
            Type::primitive_type_builder(n, physical)
                .with_repetition(Repetition::OPTIONAL)
                .with_logical_type(logical)
                .build()
                .map(Arc::new)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    let schema = Arc::new(
        Type::group_type_builder("schema")
            .with_fields(fields)
            .build()
            .map_err(fail)?,
    );
    let file = File::create(path).map_err(|e| io_error(path, &e))?;
    let props = Arc::new(WriterProperties::builder().build());
    let mut writer = SerializedFileWriter::new(file, schema, props).map_err(fail)?;
    let mut group = writer.next_row_group().map_err(fail)?;
    let mut k = 0;
    while let Some(mut col) = group.next_column().map_err(fail)? {
        let (name, ty) = (&names[k], types[k]);
        k += 1;
        let cells: Vec<Option<&Value>> = t
            .rows
            .iter()
            .map(|r| r.get(name).filter(|v| !v.is_null()))
            .collect();
        let defs: Vec<i16> = cells.iter().map(|c| i16::from(c.is_some())).collect();
        let present = cells.iter().flatten();
        match ty {
            Column::Bool => {
                let v: Vec<bool> = present.map(|c| c.as_bool().unwrap_or(false)).collect();
                col.typed::<BoolType>()
                    .write_batch(&v, Some(&defs), None)
                    .map_err(fail)?;
            }
            Column::Int => {
                let v: Vec<i64> = present.map(|c| c.as_i64().unwrap_or(0)).collect();
                col.typed::<Int64Type>()
                    .write_batch(&v, Some(&defs), None)
                    .map_err(fail)?;
            }
            Column::Float => {
                let v: Vec<f64> = present.map(|c| c.as_f64().unwrap_or(0.0)).collect();
                col.typed::<DoubleType>()
                    .write_batch(&v, Some(&defs), None)
                    .map_err(fail)?;
            }
            Column::Text => {
                let v: Vec<ByteArray> = present
                    .map(|c| text(c).map(|s| ByteArray::from(s.into_bytes())))
                    .collect::<Result<_, _>>()?;
                col.typed::<ByteArrayType>()
                    .write_batch(&v, Some(&defs), None)
                    .map_err(fail)?;
            }
        }
        col.close().map_err(fail)?;
    }
    group.close().map_err(fail)?;
    writer.close().map_err(fail)?;
    Ok(())
}

/// RFC 4180: quote a field holding a comma, quote, CR or LF, doubling quotes.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn write_csv(t: &Table, path: &Path) -> Result<(), Error> {
    let names = columns(t);
    let mut out = names
        .iter()
        .map(|n| csv_field(n))
        .collect::<Vec<_>>()
        .join(",");
    out.push_str("\r\n");
    for r in &t.rows {
        let cells: Vec<String> = names
            .iter()
            .map(|n| match r.get(n) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(v) => text(v).map(|s| csv_field(&s)),
            })
            .collect::<Result<_, _>>()?;
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    std::fs::write(path, out).map_err(|e| io_error(path, &e))
}

fn write_jsonl(t: &Table, path: &Path) -> Result<(), Error> {
    let mut f = File::create(path).map_err(|e| io_error(path, &e))?;
    for r in &t.rows {
        let mut line = to_canonical_bytes(&Value::Object(r.clone())).map_err(Error::store)?;
        line.push(b'\n');
        f.write_all(&line).map_err(|e| io_error(path, &e))?;
    }
    Ok(())
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    Analysis::from(args.analysis.clone()).without_proxy("export")?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    if args.to == To::Bundle {
        if args.items.is_some() {
            return Err(Error::usage_code(
                "conflicting_arguments",
                "--items applies to the table formats, not --to bundle",
            ));
        }
        return bundle(&project, &config, &d, args);
    }
    if args.hash_ids {
        return Err(Error::usage_code(
            "conflicting_arguments",
            "--hash-ids applies to --to bundle",
        ));
    }
    let mut tables: BTreeMap<&str, Table> = BTreeMap::new();
    for r in &d.records {
        let Some(k) = kind(r) else { continue };
        let Value::Object(mut row) = serde_json::to_value(r).expect("records serialize") else {
            continue;
        };
        // JSONL keeps the canonical records whole; the tables drop the type column.
        if args.to != To::Jsonl {
            row.remove("record");
        }
        tables
            .entry(k)
            .or_insert_with(|| Table {
                name: k.to_owned(),
                rows: Vec::new(),
            })
            .rows
            .push(row);
    }
    let mut tables: Vec<Table> = tables.into_values().collect();
    if let Some(run_id) = &args.items {
        let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
        let labels: Vec<&Label> = all.iter().collect();
        let mut settings = analysis::settings(&config, &args.analysis.clone().into());
        settings.sections.items = true;
        let r = report::build(
            d.run(run_id)?,
            &d.judgments(run_id),
            &d.items(),
            &labels,
            &settings,
        )
        .map_err(Error::report)?;
        let rows = r
            .items
            .iter()
            .filter_map(|i| match serde_json::to_value(i).expect("rows serialize") {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect();
        tables.push(Table {
            name: format!("items_{}", file_part(run_id)),
            rows,
        });
    }
    evalsig_project::project::outside_store(&args.out)?;
    std::fs::create_dir_all(&args.out).map_err(|e| io_error(&args.out, &e))?;
    let ext = match args.to {
        To::Parquet => "parquet",
        To::Csv => "csv",
        To::Jsonl | To::Bundle => "jsonl",
    };
    let mut written = Vec::new();
    for t in tables.iter().filter(|t| !t.rows.is_empty()) {
        let path = args.out.join(format!("{}.{ext}", t.name));
        match args.to {
            To::Parquet => write_parquet(t, &path)?,
            To::Csv => write_csv(t, &path)?,
            To::Jsonl | To::Bundle => write_jsonl(t, &path)?,
        }
        written.push(
            json!({"table": t.name, "rows": t.rows.len(), "file": path.display().to_string()}),
        );
    }
    let mut envelope = Envelope::new(Schema::Export, json!({ "format": ext, "tables": written }));
    envelope.provenance.snapshots = d.snapshot_ids();
    Ok(Output::success(envelope))
}

#[cfg(test)]
mod tests {
    use super::file_part;

    #[test]
    fn run_ids_become_safe_file_names() {
        assert_eq!(file_part("triage-v1"), "triage-v1");
        assert_eq!(file_part("../etc/x"), "_._etc_x");
        assert_eq!(file_part("a b/c"), "a_b_c");
    }
}
