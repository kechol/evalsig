//! Errors as returned to the user: a stable code, a message, a hint, the commands to
//! run next and an exit code (D-040). With `--format json` they are written to standard
//! error as `{"error": {"code", "message", "hint", "next"}}`.

use std::borrow::Cow;

use serde::Serialize;

use crate::envelope::Next;
use crate::exit::Exit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    /// A literal in this program, or a code relayed from a referee (`gate submit`).
    pub code: Cow<'static, str>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Commands built from evalsig's own vocabulary, never from the input, so agent
    /// mode prints them in full.
    pub next: Vec<Next>,
    #[serde(skip)]
    pub exit: Exit,
}

impl Error {
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<String>, exit: Exit) -> Self {
        Error {
            code: code.into(),
            message: message.into(),
            hint: None,
            next: Vec::new(),
            exit,
        }
    }

    /// A report that cannot be computed from these records and settings.
    pub fn report(e: evalsig_core::report::ReportError) -> Self {
        Error::new("report", e.to_string(), Exit::Rejected)
    }

    /// Input this command refuses: exit 3 with the given code.
    pub fn rejected(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Error::new(code, message, Exit::Rejected)
    }

    /// Gate or check conditions that are malformed or cannot be judged.
    pub fn invalid_conditions(message: impl Into<String>) -> Self {
        Error::rejected("invalid_conditions", message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error::new("usage", message, Exit::Usage)
    }

    /// A usage error (exit 64) common enough to have a code of its own, such as
    /// `unknown_run` (D-040).
    pub fn usage_code(code: &'static str, message: impl Into<String>) -> Self {
        Error::new(code, message, Exit::Usage)
    }

    /// Input that cannot be read as the requested format.
    pub fn invalid_input(path: &std::path::Path, e: impl std::fmt::Display) -> Self {
        Error::new(
            "invalid_input",
            format!("{}: {e}", path.display()),
            Exit::Blocked,
        )
    }

    /// The object store, snapshots or ledger failed.
    pub fn store(e: impl std::fmt::Display) -> Self {
        Error::new("store", e.to_string(), Exit::Blocked)
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// A command to run next; `command` names no value from the input.
    pub fn with_next(mut self, command: impl Into<String>, why: impl Into<String>) -> Self {
        self.next.push(Next {
            command: command.into(),
            why: why.into(),
        });
        self
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(&serde_json::json!({ "error": self }))
            .expect("error serializes");
        s.push('\n');
        s
    }

    pub fn to_text(&self) -> String {
        let mut s = format!("error[{}]: {}\n", self.code, self.message);
        if let Some(h) = &self.hint {
            s.push_str(&format!("hint: {h}\n"));
        }
        for n in &self.next {
            s.push_str(&format!("next: {}  ({})\n", n.command, n.why));
        }
        s
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
