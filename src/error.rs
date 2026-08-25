//! The error type returned by the driver.

use std::collections::HashMap;
use std::fmt;

/// An error produced while running migrations against SurrealDB.
///
/// Errors surface to callers wrapped in [`refinery_core::Error`], which adds the
/// migration that was being applied.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The SurrealDB client failed: connection, protocol, or authentication.
    Client(surrealdb::Error),
    /// One or more statements in a submitted query failed.
    ///
    /// SurrealDB reports per-statement errors rather than failing the whole
    /// query, so this carries every failure, ordered by statement index.
    Statements(Vec<StatementError>),
    /// A row of the migration history table could not be deserialized.
    Row(surrealdb::Error),
    /// A row of the migration history table held a value this driver cannot
    /// use: an unparseable checksum, or a version or timestamp out of range.
    InvalidHistoryRow(String),
}

/// A single failed statement within a submitted query.
#[derive(Debug)]
pub struct StatementError {
    /// Zero-based index of the statement within the submitted query.
    pub index: usize,
    /// The message SurrealDB reported for this statement.
    pub message: String,
}

impl fmt::Display for StatementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "statement {}: {}", self.index, self.message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(_) => write!(f, "SurrealDB client error"),
            Self::Statements(errors) => {
                write!(f, "{} statement(s) failed: ", errors.len())?;
                for (i, error) in errors.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
            Self::Row(_) => write!(f, "could not decode a migration history row"),
            Self::InvalidHistoryRow(reason) => {
                write!(f, "invalid migration history row: {reason}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(source) | Self::Row(source) => Some(source),
            Self::Statements(_) | Self::InvalidHistoryRow(_) => None,
        }
    }
}

impl Error {
    /// Build [`Error::Statements`] from the per-statement errors SurrealDB
    /// returns, ordered by statement index so the message is deterministic.
    pub(crate) fn from_statement_errors(errors: HashMap<usize, surrealdb::Error>) -> Self {
        let mut errors: Vec<StatementError> = errors
            .into_iter()
            .map(|(index, error)| StatementError {
                index,
                message: error.to_string(),
            })
            .collect();
        errors.sort_by_key(|error| error.index);
        Self::Statements(errors)
    }
}
