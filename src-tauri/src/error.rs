use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("database busy; retry shortly")]
    Busy,
    /// Structured error. Displays (and serializes to the frontend) as
    /// `code:category`, so `operation_in_progress:runner` stays byte-identical
    /// to the legacy string. An empty category displays as the bare code (for
    /// example `run_not_found`). `message` is human-readable detail for CLI
    /// JSON output and logs; it is not part of the wire string.
    #[error("{}", coded_display(code, category))]
    Coded {
        code: &'static str,
        category: String,
        message: String,
    },
}

/// Machine-readable parts of an [`AppError`] (CLI `--json` error payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorParts {
    pub code: String,
    pub category: String,
    pub message: String,
}

impl AppError {
    pub fn coded(
        code: &'static str,
        category: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Coded {
            code,
            category: category.into(),
            message: message.into(),
        }
    }

    /// Split into `code` / `category` / `message`.
    ///
    /// - `Coded` returns its fields.
    /// - `Message` holding a legacy coded string (`code:category[:detail]`,
    ///   for example `operation_in_progress:runner`) is parsed: `code` is the
    ///   text before the first colon, `category` the text up to the second
    ///   colon, and `message` the full string.
    /// - Everything else gets a fixed code/category by variant, with the
    ///   display text as `message`.
    pub fn code_parts(&self) -> ErrorParts {
        let parts = |code: &str, category: &str| ErrorParts {
            code: code.to_string(),
            category: category.to_string(),
            message: self.to_string(),
        };
        match self {
            Self::Coded {
                code,
                category,
                message,
            } => ErrorParts {
                code: (*code).to_string(),
                category: category.clone(),
                message: if message.trim().is_empty() {
                    self.to_string()
                } else {
                    message.clone()
                },
            },
            Self::Message(text) => match parse_legacy_coded(text) {
                Some((code, category)) => parts(code, category),
                None => parts("error", "message"),
            },
            Self::Anyhow(_) => parts("error", "internal"),
            Self::Sqlite(_) => parts("database", "sqlite"),
            Self::Io(_) => parts("io", "filesystem"),
            Self::Busy => parts("database", "busy"),
        }
    }
}

fn coded_display(code: &str, category: &str) -> String {
    if category.is_empty() {
        code.to_string()
    } else {
        format!("{code}:{category}")
    }
}

/// Parse `code:category[:detail]`. `code` must be a snake_case identifier
/// (`[a-z][a-z0-9_]*`); `category` must be non-empty, whitespace-free, and not
/// start with `/` (so URLs such as `https://…` are not mistaken for codes).
fn parse_legacy_coded(text: &str) -> Option<(&str, &str)> {
    let (code, rest) = text.split_once(':')?;
    let mut chars = code.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    if !first_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return None;
    }
    let category = rest.split_once(':').map_or(rest, |(c, _)| c);
    if category.is_empty() || category.starts_with('/') || category.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((code, category))
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<String> for AppError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for AppError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

pub fn map_sqlite(err: rusqlite::Error) -> AppError {
    match &err {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::DatabaseBusy
                || code.code == rusqlite::ErrorCode::DatabaseLocked =>
        {
            AppError::Busy
        }
        _ => AppError::Sqlite(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_displays_and_serializes_as_code_category() {
        let err = AppError::coded(
            "operation_in_progress",
            "runner",
            "Another run is in progress",
        );
        assert_eq!(err.to_string(), "operation_in_progress:runner");
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            "\"operation_in_progress:runner\""
        );
        // Byte-identical to the legacy string error.
        let legacy = AppError::from("operation_in_progress:runner");
        assert_eq!(err.to_string(), legacy.to_string());
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            serde_json::to_string(&legacy).unwrap()
        );
    }

    #[test]
    fn code_parts_for_coded_error() {
        let parts =
            AppError::coded("retry_ineligible", "empty_selection", "Nothing to retry").code_parts();
        assert_eq!(parts.code, "retry_ineligible");
        assert_eq!(parts.category, "empty_selection");
        assert_eq!(parts.message, "Nothing to retry");

        let blank = AppError::coded("run_not_found", "run", "").code_parts();
        assert_eq!(blank.message, "run_not_found:run");
    }

    #[test]
    fn code_parts_parses_legacy_strings() {
        let parts = AppError::from("operation_in_progress:runner").code_parts();
        assert_eq!(parts.code, "operation_in_progress");
        assert_eq!(parts.category, "runner");
        assert_eq!(parts.message, "operation_in_progress:runner");

        let parts = AppError::from("run_failed:persistence_unavailable:extra detail").code_parts();
        assert_eq!(parts.code, "run_failed");
        assert_eq!(parts.category, "persistence_unavailable");
        assert_eq!(
            parts.message,
            "run_failed:persistence_unavailable:extra detail"
        );

        let parts = AppError::from("run_canceled:3f2b-uuid").code_parts();
        assert_eq!(
            (parts.code.as_str(), parts.category.as_str()),
            ("run_canceled", "3f2b-uuid")
        );
    }

    #[test]
    fn code_parts_does_not_misparse_plain_messages() {
        for text in [
            "invalid timestamp \"x\": bad",
            "corrupt run data: status",
            "https://example.com/jobs",
            "Job not found",
            "code:",
            "Code:thing",
        ] {
            let parts = AppError::from(text).code_parts();
            assert_eq!(
                (parts.code.as_str(), parts.category.as_str()),
                ("error", "message"),
                "{text}"
            );
            assert_eq!(parts.message, text);
        }
        assert_eq!(AppError::Busy.code_parts().category, "busy");
    }
}
