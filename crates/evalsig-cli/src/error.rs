//! Errors as returned to the user: a stable code, a message, a hint and an exit code.
//! With `--format json` they are written to standard error as
//! `{"error": {"code", "message", "hint"}}`.

use serde::Serialize;

use crate::exit::Exit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip)]
    pub exit: Exit,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>, exit: Exit) -> Self {
        CliError {
            code,
            message: message.into(),
            hint: None,
            exit,
        }
    }

    /// A report that cannot be computed from these records and settings.
    pub fn report(e: evalsig_core::report::ReportError) -> Self {
        CliError::new("report", e.to_string(), Exit::Rejected)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        CliError::new("usage", message, Exit::Usage)
    }

    /// Input that cannot be read as the requested format.
    pub fn invalid_input(path: &std::path::Path, e: impl std::fmt::Display) -> Self {
        CliError::new(
            "invalid_input",
            format!("{}: {e}", path.display()),
            Exit::Blocked,
        )
    }

    /// The object store, snapshots or ledger failed.
    pub fn store(e: impl std::fmt::Display) -> Self {
        CliError::new("store", e.to_string(), Exit::Blocked)
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(&serde_json::json!({ "error": self }))
            .expect("error serializes");
        s.push('\n');
        s
    }

    pub fn to_text(&self) -> String {
        match &self.hint {
            Some(h) => format!("error[{}]: {}\nhint: {h}\n", self.code, self.message),
            None => format!("error[{}]: {}\n", self.code, self.message),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}
