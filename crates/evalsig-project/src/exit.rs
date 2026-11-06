//! Exit codes (D-008, SPEC §10.2).

/// The process exit code. The numbers are part of the public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Success, or a passed gate or check.
    Success,
    /// A gate or check failed.
    Failed,
    /// The holdout budget is exhausted.
    BudgetExhausted,
    /// Conditions are invalid, do not match the approval, or the candidate's records
    /// fail the referee's checks.
    Rejected,
    /// Too few items to decide.
    InsufficientData,
    /// Stopped by a validity check.
    Blocked,
    /// Wrong usage of the command line.
    Usage,
}

impl Exit {
    pub const ALL: [Exit; 7] = [
        Exit::Success,
        Exit::Failed,
        Exit::BudgetExhausted,
        Exit::Rejected,
        Exit::InsufficientData,
        Exit::Blocked,
        Exit::Usage,
    ];

    pub fn code(self) -> u8 {
        match self {
            Exit::Success => 0,
            Exit::Failed => 1,
            Exit::BudgetExhausted => 2,
            Exit::Rejected => 3,
            Exit::InsufficientData => 4,
            Exit::Blocked => 5,
            Exit::Usage => 64,
        }
    }

    pub fn meaning(self) -> &'static str {
        match self {
            Exit::Success => "success, or the gate or check passed",
            Exit::Failed => "the gate or check failed",
            Exit::BudgetExhausted => "the holdout budget is exhausted",
            Exit::Rejected => {
                "conditions are invalid or do not match the approval, or the candidate failed the referee's checks"
            }
            Exit::InsufficientData => "too few items to decide",
            Exit::Blocked => "stopped by a validity check",
            Exit::Usage => "usage error",
        }
    }
}
