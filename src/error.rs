//! The error type returned by the driver.

use std::fmt;

/// An error produced while running migrations against SurrealDB.
///
/// Errors surface to callers wrapped in [`refinery_core::Error`], which adds the
/// migration that was being applied. Match on this type via
/// [`std::error::Error::source`] when you need to distinguish causes.
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
    /// A row in the migration history table could not be decoded.
    Row(surrealdb::Error),
    /// The configured migration table name is not a valid SurrealDB identifier.
    ///
    /// Only ASCII alphanumerics and underscores are accepted, and the name may
    /// not begin with a digit. refinery interpolates the table name into its
    /// history `INSERT` unquoted, so a name needing quoting cannot work.
    InvalidTableName(String),
    /// A history row recorded a version outside the range refinery supports.
    VersionOutOfRange(i64),
    /// A history row recorded a timestamp that cannot be represented.
    TimestampOutOfRange(i64),
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
            Self::InvalidTableName(name) => write!(
                f,
                "{name:?} is not a valid migration table name; use only ASCII \
                 letters, digits and underscores, and do not start with a digit"
            ),
            Self::VersionOutOfRange(version) => {
                write!(f, "migration version {version} is out of range")
            }
            Self::TimestampOutOfRange(seconds) => {
                write!(f, "applied_on timestamp {seconds} is out of range")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(source) | Self::Row(source) => Some(source),
            Self::Statements(_)
            | Self::InvalidTableName(_)
            | Self::VersionOutOfRange(_)
            | Self::TimestampOutOfRange(_) => None,
        }
    }
}

impl Error {
    /// Build a [`Error::Statements`] from the per-statement errors SurrealDB
    /// returns, ordered by statement index so the message is deterministic.
    pub(crate) fn from_statement_errors(
        errors: std::collections::HashMap<usize, surrealdb::Error>,
    ) -> Self {
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
