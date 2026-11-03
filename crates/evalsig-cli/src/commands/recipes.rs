//! `evalsig recipes acquire <source>`: how to obtain what evalsig needs from a source.

use clap::{Subcommand, ValueEnum};
use serde_json::json;

use crate::error::CliError;
use crate::output::{Envelope, Output};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Source {
    Inspect,
    Promptfoo,
    Langfuse,
    Csv,
}

#[derive(Debug, Subcommand)]
pub enum RecipesCommand {
    /// Steps to get judgments, replicates and labels out of a source.
    Acquire { source: Source },
}

fn steps(source: Source) -> Vec<&'static str> {
    match source {
        Source::Inspect => vec![
            "Run the evaluation with epochs of 2 or more so replicates exist: inspect eval task.py --epochs 3",
            "Keep the .eval log (the default); evalsig reads it directly: evalsig import logs/<file>.eval",
            "For a .json log, name the adapter: evalsig import --adapter inspect logs/<file>.json",
            "Pick the score with an [[import]] block (adapter = \"inspect\", score = \"<name>\") when samples carry several",
            "C/I grades are score semantics; to compare with human labels, import them from a CSV with --as label",
        ],
        Source::Promptfoo => vec![
            "Run with repeats so replicates exist: promptfoo eval --repeat 3 -o out.json",
            "Import the JSON output: evalsig import --adapter promptfoo out.json",
            "Or read the local database without exporting: evalsig import ~/.promptfoo/promptfoo.db --arg eval=<evalId>",
            "Give tests a stable ID in metadata.testCaseId; otherwise evalsig keys items by description and vars",
        ],
        Source::Langfuse => vec![
            "Export the scores as JSON, for example from the public API (GET /api/public/scores)",
            "Look at the shape without printing contents: evalsig map probe scores.json",
            "Draft a mapping and let a person decide its choices: evalsig map draft scores.json --out mapping.toml",
            "Use traceId (or observationId) as the item, the score name as the task, and value or stringValue as the value",
            "Map annotation scores (source ANNOTATION) as labels with rater_id from authorUserId; scores edited later need a new label version",
        ],
        Source::Csv => vec![
            "One judgment per row with columns named after record fields: evalsig import results.csv",
            "One label per row: evalsig import --as label labels.csv",
            "Other column names: write an [[import]] block (adapter = \"csv\", map.item_id = \"<column>\", ...) and run evalsig import --from <name>",
            "Values that cannot be mapped become missing (invalid_output); list them with value_map",
        ],
    }
}

pub fn run(cmd: &RecipesCommand) -> Result<Output, CliError> {
    let RecipesCommand::Acquire { source } = cmd;
    let name = source
        .to_possible_value()
        .expect("no skipped variants")
        .get_name()
        .to_owned();
    Ok(Output::success(Envelope::new(
        "evalsig.recipes/1",
        json!({"source": name, "steps": steps(*source)}),
    )))
}
