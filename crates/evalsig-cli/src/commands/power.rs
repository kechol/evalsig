//! `evalsig power`: sample sizes, the minimum detectable difference and a gate's pass
//! probability (metrics spec §12), and `evalsig threshold`: the cost-optimal decision
//! threshold and its sensitivity to the cost ratio (metrics spec §4).

use std::path::Path;

use evalsig_core::cost;
use evalsig_core::power;
use evalsig_core::records::Label;
use evalsig_core::report;
use serde_json::{Value, json};

use evalsig_project::analysis::{self, Analysis};

use crate::args::AnalysisArgs;
use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema, round_numbers};
use evalsig_project::exit::Exit;
use evalsig_project::project::Project;

#[derive(Debug, Clone, clap::Args)]
pub struct PowerArgs {
    /// Items to claim a disagreement rate below this bound when none is observed.
    #[arg(long, value_name = "U")]
    pub zero_events_below: Option<f64>,
    /// Items to measure a rate near P to within --half-width.
    #[arg(long, value_name = "P", requires = "half_width")]
    pub rate: Option<f64>,
    #[arg(long, value_name = "W")]
    pub half_width: Option<f64>,
    /// The accuracy difference to detect between two systems on the same items.
    #[arg(long, value_name = "DELTA", requires = "discordance")]
    pub delta: Option<f64>,
    /// The share of items only one of the two systems gets right (ψ).
    #[arg(long, value_name = "PSI")]
    pub discordance: Option<f64>,
    #[arg(long, default_value_t = 0.8)]
    pub power: f64,
    #[arg(long, default_value_t = 0.05)]
    pub alpha: f64,
    /// Test one-sided, as gates do.
    #[arg(long)]
    pub one_sided: bool,
    /// Submissions a gate allows: the level becomes alpha / budget.
    #[arg(long, default_value_t = 1)]
    pub budget: u32,
    /// With --discordance: the difference these many items detect, and with --delta
    /// the probability of passing.
    #[arg(long, value_name = "N")]
    pub n: Option<u64>,
    /// Take the level, sidedness and budget from these gate conditions (gate.toml or an
    /// approved copy): the primary condition's level and sidedness, and the holdout budget.
    #[arg(long, value_name = "FILE")]
    pub gate: Option<std::path::PathBuf>,
    /// Mean cluster size and intra-cluster correlation for the design effect.
    #[arg(long, value_name = "M", requires = "icc")]
    pub cluster_size: Option<f64>,
    #[arg(long, value_name = "RHO")]
    pub icc: Option<f64>,
}

pub fn power(args: &PowerArgs) -> Result<Output, Error> {
    let mut args = args.clone();
    if let Some(path) = &args.gate {
        let text = std::fs::read_to_string(path)
            .map_err(|e| evalsig_project::project::io_error(path, &e))?;
        let doc: toml::Table = toml::from_str(&text).map_err(|e| Error::invalid_input(path, e))?;
        let primary = doc
            .get("conditions")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first());
        let level = primary
            .and_then(|c| c.get("level"))
            .and_then(toml::Value::as_float)
            .unwrap_or(0.95);
        args.alpha = 1.0 - level;
        args.one_sided = primary
            .and_then(|c| c.get("sided"))
            .and_then(toml::Value::as_str)
            == Some("one");
        args.budget = doc
            .get("holdout")
            .and_then(|h| h.get("budget"))
            .and_then(toml::Value::as_integer)
            .map_or(5, |b| b as u32);
    }
    let args = &args;
    let effect = |n: u64| match (args.cluster_size, args.icc) {
        (Some(m), Some(rho)) => power::with_design_effect(n, m, rho),
        _ => n,
    };
    // Given items in clusters are worth fewer independent ones: n / (1 + (m̄ − 1)ρ).
    let effective = |n: u64| match (args.cluster_size, args.icc) {
        (Some(m), Some(rho)) => (n as f64 / (1.0 + (m - 1.0) * rho)).floor() as u64,
        _ => n,
    };
    let level = power::bonferroni(args.alpha, args.budget);
    let two_sided = !args.one_sided;
    let mut results = json!({ "alpha": args.alpha, "budget": args.budget, "level": level, "two_sided": two_sided });
    let mut any = false;
    if let Some(u) = args.zero_events_below {
        let (exact, wilson) = power::zero_event_n(u, level);
        results["zero_events"] =
            json!({"below": u, "exact": effect(exact), "wilson": effect(wilson)});
        any = true;
    }
    if let (Some(p), Some(w)) = (args.rate, args.half_width) {
        let n = power::n_for_halfwidth(p, w, 1.0 - level);
        results["rate_precision"] = json!({"rate": p, "half_width": w, "items": effect(n)});
        any = true;
    }
    if let Some(psi) = args.discordance {
        let mut paired = json!({"discordance": psi, "power": args.power});
        if let Some(delta) = args.delta {
            if !(delta > 0.0 && delta < psi && psi <= 1.0) {
                return Err(Error::usage("--delta must lie in (0, --discordance)"));
            }
            let (p10, p01) = ((psi + delta) / 2.0, (psi - delta) / 2.0);
            paired["delta"] = json!(delta);
            paired["exact"] =
                json!(power::mcnemar_n_exact(p10, p01, args.power, level, two_sided).map(effect));
            paired["approximation"] =
                json!(power::mcnemar_n_connor(p10, p01, args.power, level, two_sided).map(effect));
            if let Some(n) = args.n {
                paired["pass_probability"] = json!(power::mcnemar_power(
                    effective(n) as usize,
                    p10,
                    p01,
                    level,
                    two_sided
                ));
            }
        }
        if let Some(n) = args.n {
            paired["mde"] = json!(power::minimum_detectable_difference(
                effective(n),
                psi,
                args.power,
                level,
                two_sided
            ));
        }
        results["paired"] = paired;
        any = true;
    }
    if !any {
        return Err(Error::usage(
            "give --zero-events-below, --rate with --half-width, or --discordance (with --delta or --n)",
        ));
    }
    Ok(Output::success(Envelope::new(
        Schema::Power,
        round_numbers(results),
    )))
}

#[derive(Debug, Clone, clap::Args)]
pub struct ThresholdArgs {
    /// A run whose judgments are levels.
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// The cost ratio r = C_FN / C_FP for the threshold (default: the matrix's corners,
    /// C[positive][lowest level] / C[negative][highest level]).
    #[arg(long, value_name = "R")]
    pub ratio: Option<f64>,
    /// Also the expected-cost difference with this run (at its decision rule) at both
    /// ends of the configured ratio range.
    #[arg(long, value_name = "RUN")]
    pub against: Option<String>,
}

pub fn threshold(root: &Path, args: &ThresholdArgs) -> Result<Output, Error> {
    Analysis::from(args.analysis.clone()).without_proxy("threshold")?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let c = config.cost.clone().ok_or_else(|| {
        Error::usage("threshold needs a [cost] matrix and prevalence in the configuration")
    })?;
    let d = data::load(&project)?;
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().collect();
    let settings = analysis::settings(&config, &args.analysis.clone().into());
    let items = d.items();
    let prepare = |id: &str| -> Result<report::Prepared<'_>, Error> {
        report::prepare(d.run(id)?, &d.judgments(id), &items, &labels, &settings)
            .map_err(Error::report)
    };
    let p = prepare(&args.run)?;
    let levels = c.matrix.first().map_or(0, Vec::len);
    let (pos, neg) = p.level_counts(levels).ok_or_else(|| {
        Error::new(
            "insufficient_data",
            "the run's judgments are not levels 0 … k−1 with a reference",
            Exit::InsufficientData,
        )
    })?;
    let ratio = args.ratio.or_else(|| c.corner_ratio()).ok_or_else(|| {
        Error::usage("the [cost] matrix's corners give no cost ratio; pass --ratio")
    })?;
    let binary = cost::ratio_matrix(ratio);
    let mut results = json!({
        "run": args.run,
        "prevalence": c.prevalence,
        "positives_by_level": pos,
        "negatives_by_level": neg,
        "expected_cost": cost::expected_cost(&pos, &neg, &c.matrix, c.prevalence),
        "ratio": ratio,
        "threshold_costs": cost::threshold_costs(&pos, &neg, &binary, c.prevalence)
            .map(|v| v.into_iter().map(|(t, ec)| json!({"threshold": t, "expected_cost": ec})).collect::<Vec<_>>()),
        "optimal_threshold": cost::optimal_threshold(&pos, &neg, &binary, c.prevalence).map(|(t, _)| t),
        "posterior_threshold": cost::posterior_threshold(1.0, ratio),
        "lr_threshold": cost::lr_threshold(1.0, ratio, c.prevalence),
    });
    if let Some(range) = c.ratio_range {
        results["sensitivity"] =
            json!(cost::threshold_sensitivity(&pos, &neg, c.prevalence, range));
        if let Some(other) = &args.against {
            let q = prepare(other)?;
            let (a, b) = (p.rates(), q.rates());
            results["difference"] = match (a, b) {
                (Some(a), Some(b)) => json!({
                    "against": other,
                    "sensitivity": cost::difference_sensitivity(a, b, c.prevalence, range),
                }),
                _ => Value::Null,
            };
        }
    }
    Ok(Output::success(Envelope::new(
        Schema::Threshold,
        round_numbers(results),
    )))
}
