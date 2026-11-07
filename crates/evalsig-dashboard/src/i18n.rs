//! The dashboard's own text in each language it is shown in (D-033): screen names,
//! leads, headings, column names, buttons, notes and chart labels. Results, verdicts,
//! warnings, errors and glossary meanings come from the library in English (D-002) and
//! are shown as they are. The Japanese follows the terms of `README.ja.md`.

use evalsig_core::config::Language;

/// One piece of the pages' own text. Every variant has a text in every language, so a
/// missing translation does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T {
    // Screens and their leads.
    Overview,
    Comparison,
    Trend,
    Reproducibility,
    ReferenceQuality,
    Discrimination,
    Error,
    LeadOverview,
    LeadComparison,
    LeadTrend,
    LeadReproducibility,
    LeadReferenceQuality,
    LeadDiscrimination,
    // Panels.
    Runs,
    Differences,
    RocSpace,
    Transitions,
    ItemsChanged,
    ItemsUnchanged,
    EqualVarianceModel,
    EmpiricalRoc,
    Calibration,
    ReplicateAgreement,
    PerformanceByK,
    SameRule,
    RaterPairs,
    RaterVariability,
    Ceiling,
    ByPeriod,
    // Columns.
    Run,
    System,
    Version,
    OutputType,
    Items,
    Judgments,
    Missing,
    Item,
    Reference,
    Baseline,
    Candidate,
    Change,
    K,
    Rule,
    Metric,
    Value,
    Interval,
    CostPerItem,
    Who,
    Kind,
    MeanKappa,
    HumansCompared,
    Pair,
    PositiveAgreement,
    NegativeAgreement,
    Task,
    WithinRater,
    Pairs,
    BetweenRaters,
    Source,
    ItemsWithReplicates,
    PairwiseDisagreement,
    ItemsWithAnyDisagreement,
    UpperBound,
    Dprime,
    Criterion,
    Gained,
    Lost,
    Hits,
    FalseAlarms,
    // Changes of an item.
    Further,
    Closer,
    Changed,
    // Actions and labels.
    Show,
    Swap,
    CompareWith,
    RunField,
    Values,
    HowToRead,
    // Notes.
    NoRuns,
    NeedsTwoRuns,
    NoItemChanged,
    NoReplicates,
    NoEmpiricalRoc,
    CeilingNote,
    // Charts.
    DecisionAxis,
    Noise,
    Signal,
    HitsArea,
    FalseAlarmsArea,
    CriterionAt,
    FalseAlarmRate,
    HitRate,
    ZFalseAlarmRate,
    ZHitRate,
    MeanScore,
    SharePositive,
    ReplicatesPerItem,
    BaselineRun,
    CandidateRun,
}

impl T {
    /// The text in `lang`; `{}` marks where `fill` puts a value.
    pub fn text(self, lang: Language) -> &'static str {
        let (en, ja) = match self {
            T::Overview => ("Overview", "概要"),
            T::Comparison => ("Comparison", "比較"),
            T::Trend => ("Trend", "推移"),
            T::Reproducibility => ("Reproducibility", "再現性"),
            T::ReferenceQuality => ("Reference quality", "参照ラベルの質"),
            T::Discrimination => ("Discrimination", "弁別力"),
            T::Error => ("Error", "エラー"),
            T::LeadOverview => (
                "How each run performs: every metric with its interval, all runs on one axis.",
                "各 run の成績です。指標ごとの値と区間を、すべての run で同じ軸に並べます。",
            ),
            T::LeadComparison => (
                "Whether the candidate differs from the baseline, and on which items.",
                "候補がベースラインと違うか、どの item で違うかを示します。",
            ),
            T::LeadTrend => (
                "How a run's primary metric moves from one period to the next.",
                "run の主指標が期間ごとにどう動くかを示します。",
            ),
            T::LeadReproducibility => (
                "How often a run's replicates of one item disagree, and what more replicates buy.",
                "同じ item への反復がどれだけ食い違うか、反復を増やすと何が得られるかを示します。",
            ),
            T::LeadReferenceQuality => (
                "How well the human raters agree with one another, and the system with them.",
                "人の評価者どうしがどれだけ一致し、システムが人とどれだけ一致するかを示します。",
            ),
            T::LeadDiscrimination => (
                "How well a run's judgments separate positive items from negative ones.",
                "run の判定が、陽性の item と陰性の item をどれだけ見分けるかを示します。",
            ),
            T::Runs => ("Runs", "run の一覧"),
            T::Differences => (
                "Differences (candidate − baseline)",
                "差（候補 − ベースライン）",
            ),
            T::RocSpace => ("ROC space", "ROC 空間"),
            T::Transitions => ("Transitions", "判定の入れ替わり"),
            T::ItemsChanged => ("Items that changed ({})", "判定が変わった item（{} 件）"),
            T::ItemsUnchanged => ("Unchanged items ({})", "判定が変わらなかった item（{} 件）"),
            T::EqualVarianceModel => ("Equal-variance model", "等分散正規モデル"),
            T::EmpiricalRoc => ("Empirical ROC and z-ROC", "経験 ROC と z-ROC"),
            T::Calibration => ("Calibration", "較正"),
            T::ReplicateAgreement => ("Replicate agreement", "反復の一致"),
            T::PerformanceByK => ("Performance and cost by k", "k ごとの性能とコスト"),
            T::SameRule => (
                "Same rule: each rater, grader or run against the humans",
                "同じ規則での一致：各評価者・grader・run と人",
            ),
            T::RaterPairs => ("Rater pairs", "評価者の組"),
            T::RaterVariability => ("Rater variability", "評価者のばらつき"),
            T::Ceiling => ("Ceiling with a soft reference", "soft 参照での上限"),
            T::ByPeriod => ("{} by period", "期間ごとの {}"),
            T::Run => ("Run", "run"),
            T::System => ("System", "システム"),
            T::Version => ("Version", "版"),
            T::OutputType => ("Output type", "出力の型"),
            T::Items => ("Items", "item 数"),
            T::Judgments => ("Judgments", "判定数"),
            T::Missing => ("Missing", "欠損"),
            T::Item => ("Item", "item"),
            T::Reference => ("Reference", "参照"),
            T::Baseline => ("Baseline", "ベースライン"),
            T::Candidate => ("Candidate", "候補"),
            T::Change => ("Change", "変化"),
            T::K => ("k", "k"),
            T::Rule => ("Rule", "規則"),
            T::Metric => ("Metric", "指標"),
            T::Value => ("Value", "値"),
            T::Interval => ("Interval", "区間"),
            T::CostPerItem => ("Cost per item", "item あたりのコスト"),
            T::Who => ("Who", "誰"),
            T::Kind => ("Kind", "種類"),
            T::MeanKappa => ("Mean κ", "κ の平均"),
            T::HumansCompared => ("Humans compared", "比べた人数"),
            T::Pair => ("Pair", "組"),
            T::PositiveAgreement => ("Positive agreement", "陽性の一致"),
            T::NegativeAgreement => ("Negative agreement", "陰性の一致"),
            T::Task => ("Task", "タスク"),
            T::WithinRater => ("Within a rater", "同じ評価者の中"),
            T::Pairs => ("Pairs", "組数"),
            T::BetweenRaters => ("Between raters", "評価者の間"),
            T::Source => ("Source", "由来"),
            T::ItemsWithReplicates => ("Items with replicates", "反復のある item 数"),
            T::PairwiseDisagreement => ("Pairwise disagreement", "組ごとの不一致"),
            T::ItemsWithAnyDisagreement => {
                ("Items with any disagreement", "不一致のある item の割合")
            }
            T::UpperBound => ("Upper bound", "上限"),
            T::Dprime => ("d′ (sensitivity)", "d′（弁別力）"),
            T::Criterion => ("c (criterion)", "c（判定基準）"),
            T::Gained => ("Gained", "増えた"),
            T::Lost => ("Lost", "減った"),
            T::Hits => ("Hits", "ヒット"),
            T::FalseAlarms => ("False alarms", "誤警報"),
            T::Further => ("further from the reference", "参照から遠ざかった"),
            T::Closer => ("closer to the reference", "参照に近づいた"),
            T::Changed => ("changed", "変わった"),
            T::Show => ("Show", "表示"),
            T::Swap => (
                "Swap baseline and candidate",
                "ベースラインと候補を入れ替える",
            ),
            T::CompareWith => ("Compare with {}", "{} と比べる"),
            T::RunField => ("run", "run"),
            T::Values => ("Values", "値"),
            T::HowToRead => ("How to read", "読み方"),
            T::NoRuns => ("No runs.", "run がありません。"),
            T::NeedsTwoRuns => (
                "A comparison needs two runs.",
                "比較には 2 つの run が要ります。",
            ),
            T::NoItemChanged => (
                "No item's judgment changed.",
                "判定が変わった item はありません。",
            ),
            T::NoReplicates => (
                "Every item has one judgment in this run, so there are no replicates to compare. \
                 Judge each item several times (gen_replicate 0, 1, …) and import the run again.",
                "この run では各 item の判定が 1 回なので、比べる反復がありません。各 item を複数回判定し\
                 （gen_replicate 0, 1, …）、run を取り込み直してください。",
            ),
            T::NoEmpiricalRoc => (
                "This run's judgments are {}; an empirical curve needs ordered judgments (output \
                 type ordinal or score).",
                "この run の判定は {} です。経験 ROC には順序のある判定（出力の型 ordinal か score）が要ります。",
            ),
            T::CeilingNote => (
                "No system can exceed accuracy {} against this reference.",
                "この参照に対しては、どのシステムも正解率 {} を超えられません。",
            ),
            T::DecisionAxis => ("decision axis (standard deviations)", "決定軸（標準偏差）"),
            T::Noise => ("reference negative (noise)", "参照が陰性（ノイズ）"),
            T::Signal => ("reference positive (signal)", "参照が陽性（シグナル）"),
            T::HitsArea => ("hits", "ヒット"),
            T::FalseAlarmsArea => ("false alarms", "誤警報"),
            T::CriterionAt => ("criterion, c = {}", "判定基準 c = {}"),
            T::FalseAlarmRate => ("false alarm rate", "誤警報率"),
            T::HitRate => ("hit rate", "ヒット率"),
            T::ZFalseAlarmRate => ("z(false alarm rate)", "z(誤警報率)"),
            T::ZHitRate => ("z(hit rate)", "z(ヒット率)"),
            T::MeanScore => ("mean score", "スコアの平均"),
            T::SharePositive => ("share positive", "陽性の割合"),
            T::ReplicatesPerItem => ("replicates per item (k)", "item あたりの反復数（k）"),
            T::BaselineRun => ("baseline {}", "ベースライン {}"),
            T::CandidateRun => ("candidate {}", "候補 {}"),
        };
        match lang {
            Language::En => en,
            Language::Ja => ja,
        }
    }

    /// The text in `lang` with its `{}` replaced by `value`.
    pub fn fill(self, lang: Language, value: &str) -> String {
        self.text(lang).replacen("{}", value, 1)
    }

    /// The text in `lang`, escaped, with its `{}` replaced by `html`, markup the caller
    /// has escaped already.
    pub fn fill_html(self, lang: Language, html: &str) -> String {
        crate::html::esc(self.text(lang)).replacen("{}", html, 1)
    }
}

/// The value of the `lang` attribute of a page in `lang`.
pub fn tag(lang: Language) -> &'static str {
    match lang {
        Language::En => "en",
        Language::Ja => "ja",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_keep_their_place_for_a_value_in_every_language() {
        for t in [
            T::ItemsChanged,
            T::ItemsUnchanged,
            T::ByPeriod,
            T::CompareWith,
            T::NoEmpiricalRoc,
            T::CeilingNote,
            T::CriterionAt,
            T::BaselineRun,
            T::CandidateRun,
        ] {
            for lang in [Language::En, Language::Ja] {
                assert_eq!(t.text(lang).matches("{}").count(), 1, "{t:?} {lang:?}");
            }
        }
        assert_eq!(T::CompareWith.fill(Language::Ja, "r1"), "r1 と比べる");
    }
}
