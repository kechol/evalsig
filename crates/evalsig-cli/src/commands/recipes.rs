//! `evalsig recipes [name]`: the steps of common tasks, and how to obtain what evalsig
//! needs from a source (agent-interface spec §2.6).

use clap::{Subcommand, ValueEnum};
use serde_json::json;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Source {
    Inspect,
    Promptfoo,
    Langfuse,
    Csv,
}

#[derive(Debug, Subcommand)]
pub enum RecipesCommand {
    /// Compare a new version with the current one.
    Compare,
    /// How many items a question needs.
    SampleSize,
    /// Decide adoption on a sealed holdout with preregistered conditions.
    Gate,
    /// Improve a system while looking at evaluations, without fitting the holdout.
    ImproveLoop,
    /// Use evalsig to judge a DSPy program's optimisation.
    Dspy,
    /// Steps to get judgments, replicates and labels out of a source.
    Acquire { source: Source },
}

/// Every task recipe by name, with its steps.
const TASKS: [(&str, &str, &[&str]); 5] = [
    (
        "compare",
        "Compare a new version with the current one",
        &[
            "Import both runs' records: evalsig import baseline.jsonl candidate.jsonl",
            "Compare them on their common items: evalsig compare <baseline-run> <candidate-run>",
            "Quote verdict.code and the interval of the difference; do not recompute numbers",
            "indistinguishable or negligible is not an improvement; for too few items run the power command in next",
        ],
    ),
    (
        "sample-size",
        "How many items a question needs",
        &[
            "To show a rate stays below u with zero events: evalsig power --zero-events-below <u>",
            "To estimate a rate to a half-width: evalsig power --rate <p> --half-width <w>",
            "To detect a paired difference: evalsig power --delta <d> --discordance <psi> --power 0.8",
            "For clustered items add --cluster-size <m> --icc <rho>; for a gate add --budget <B> --one-sided",
            "To see whether approved conditions can pass: evalsig power --gate gate.toml",
        ],
    ),
    (
        "gate",
        "Decide adoption on a sealed holdout with preregistered conditions",
        &[
            "As the referee: evalsig referee init <home>, then evalsig split --holdout --referee <home> --out remaining.json",
            "Give the agent remaining.json only; seal the holdout: evalsig seal --holdout <hash> --referee <home> --agent-project <dir>",
            "Draft the conditions: evalsig preregister --draft gate.toml (in agent mode add --yes)",
            "A person approves as the referee: evalsig preregister approve <draft> --holdout <hash> --from <dir> --referee <home>",
            "Submit: evalsig gate submit --approved <hash> --candidate <run> --records <file> --socket <home>/gate.sock",
            "Quote verdict.code and the budget left; people read the result in the referee ledger",
        ],
    ),
    (
        "improve-loop",
        "Improve a system while looking at evaluations, without fitting the holdout",
        &[
            "Split the remaining items: evalsig split --within remaining.json",
            "Look at errors on dev only; never ask for holdout labels or rewrite approved conditions",
            "Check a candidate on validation: evalsig compare <baseline> <candidate> --split validation",
            "Submit only when validation shows improved; each submission spends budget",
            "On fail go back to dev; on budget_exhausted stop and report to a person",
        ],
    ),
    (
        "dspy",
        "Use evalsig to judge a DSPy program's optimisation",
        &[
            "Run the program before and after optimisation on the same dev items, with replicates (several seeds)",
            "Write each run's judgments as records/1: evalsig schema records --emit python",
            "Compare on dev while optimising, then on validation: evalsig compare <before> <after> --split validation",
            "Keep the holdout for the gate recipe; the optimiser must not see it",
        ],
    ),
];

fn steps(source: Source) -> &'static [&'static str] {
    match source {
        Source::Inspect => &[
            "Run the evaluation with epochs of 2 or more so replicates exist: inspect eval task.py --epochs 3",
            "Keep the .eval log (the default); evalsig reads it directly: evalsig import logs/<file>.eval",
            "For a .json log, name the adapter: evalsig import --adapter inspect logs/<file>.json",
            "Pick the score with an [[import]] block (adapter = \"inspect\", score = \"<name>\") when samples carry several",
            "C/I grades are score semantics; to compare with human labels, import them from a CSV with --as label",
        ],
        Source::Promptfoo => &[
            "Run with repeats so replicates exist: promptfoo eval --repeat 3 -o out.json",
            "Import the JSON output: evalsig import --adapter promptfoo out.json",
            "Or read the local database without exporting: evalsig import ~/.promptfoo/promptfoo.db --arg eval=<evalId>",
            "Give tests a stable ID in metadata.testCaseId; otherwise evalsig keys items by description and vars",
        ],
        Source::Langfuse => &[
            "Export one JSON line per finished trace, each the public API's GET /api/public/traces/{traceId}, so a trace's judge and annotation scores sit together; keep one score name",
            "Look at the shape without printing contents: evalsig map probe traces.jsonl",
            "Draft a mapping and let a person decide its choices: evalsig map draft traces.jsonl --out mapping.toml",
            "Use the trace id as the item and sessionId as the cluster; take the judgment from $.scores[?(@.source == \"EVAL\")] and the label from $.scores[?(@.source == \"ANNOTATION\")] with rater_id from authorUserId",
            "examples/langfuse/mapping.toml is a worked mapping; scores edited later need a new label version",
        ],
        Source::Csv => &[
            "One judgment per row with columns named after record fields: evalsig import results.csv",
            "One label per row: evalsig import --as label labels.csv",
            "Other column names: write an [[import]] block (adapter = \"csv\", map.item_id = \"<column>\", ...) and run evalsig import --from <name>",
            "Values that cannot be mapped become missing (invalid_output); list them with value_map",
        ],
    }
}

fn task(name: &str) -> Output {
    let (_, about, steps) = TASKS
        .iter()
        .find(|(n, _, _)| *n == name)
        .expect("every task subcommand has a recipe");
    Output::success(Envelope::new(
        Schema::Recipes,
        json!({"recipe": name, "about": about, "steps": steps}),
    ))
}

pub fn run(cmd: Option<&RecipesCommand>) -> Result<Output, Error> {
    let source = match cmd {
        None => {
            let list: Vec<_> = TASKS
                .iter()
                .map(|(n, about, _)| json!({"recipe": n, "about": about}))
                .chain(std::iter::once(json!({"recipe": "acquire <source>", "about": "Steps to get judgments, replicates and labels out of a source"})))
                .collect();
            return Ok(Output::success(Envelope::new(
                Schema::Recipes,
                json!({"recipes": list}),
            )));
        }
        Some(RecipesCommand::Compare) => return Ok(task("compare")),
        Some(RecipesCommand::SampleSize) => return Ok(task("sample-size")),
        Some(RecipesCommand::Gate) => return Ok(task("gate")),
        Some(RecipesCommand::ImproveLoop) => return Ok(task("improve-loop")),
        Some(RecipesCommand::Dspy) => return Ok(task("dspy")),
        Some(RecipesCommand::Acquire { source }) => source,
    };
    let name = source
        .to_possible_value()
        .expect("no skipped variants")
        .get_name()
        .to_owned();
    Ok(Output::success(Envelope::new(
        Schema::Recipes,
        json!({"source": name, "steps": steps(*source)}),
    )))
}
