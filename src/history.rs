//! The migration history table: its schema, its queries, and row decoding.

use chrono::{DateTime, Utc};
use refinery_core::{Migration, SchemaVersion};
use surrealdb::types::SurrealValue;
use surrealdb_types::{Error as TypesError, Value};
use time::{Duration, OffsetDateTime};

use crate::Error;

/// The columns selected from the history table.
///
/// These must stay in step with [`HistoryRow`]. The projection is explicit
/// rather than `SELECT *` so that the record `id` column, which has no
/// corresponding field, is never handed to the deserializer.
const COLUMNS: &str = "version, name, applied_on, checksum";

/// One row of the migration history table.
#[derive(Debug, SurrealValue)]
pub(crate) struct HistoryRow {
    /// Read as `i64` because that is SurrealDB's integer width; narrowed to
    /// refinery's `SchemaVersion` when converting, with a range check.
    version: i64,
    name: String,
    applied_on: DateTime<Utc>,
    checksum: Checksum,
}

/// A refinery migration checksum.
///
/// refinery checksums are `u64`, but SurrealDB integers are `i64`, so a
/// checksum in the upper half of the range cannot be stored as a number. It is
/// stored as a string instead, which round-trips the full range.
#[derive(Debug)]
struct Checksum(u64);

impl SurrealValue for Checksum {
    fn kind_of() -> surrealdb_types::Kind {
        surrealdb_types::Kind::String
    }

    fn into_value(self) -> Value {
        Value::String(self.0.to_string())
    }

    fn from_value(value: Value) -> Result<Self, TypesError> {
        match value {
            Value::String(text) => text
                .parse::<u64>()
                .map(Checksum)
                .map_err(|error| TypesError::thrown(format!("invalid checksum {text:?}: {error}"))),
            other => Err(TypesError::thrown(format!(
                "expected checksum to be a string, found {other:?}"
            ))),
        }
    }
}

impl TryFrom<HistoryRow> for Migration {
    type Error = Error;

    fn try_from(row: HistoryRow) -> Result<Self, Error> {
        let version = SchemaVersion::try_from(row.version)
            .map_err(|_| Error::VersionOutOfRange(row.version))?;
        let applied_on = to_offset_datetime(&row.applied_on)
            .ok_or_else(|| Error::TimestampOutOfRange(row.applied_on.timestamp()))?;
        Ok(Migration::applied(
            version,
            row.name,
            applied_on,
            row.checksum.0,
        ))
    }
}

/// Convert the `chrono` value SurrealDB hands back into the `time` value
/// refinery expects, without going through a string.
fn to_offset_datetime(value: &DateTime<Utc>) -> Option<OffsetDateTime> {
    let seconds = OffsetDateTime::from_unix_timestamp(value.timestamp()).ok()?;
    Some(seconds + Duration::nanoseconds(i64::from(value.timestamp_subsec_nanos())))
}

/// Reject a table name that SurrealDB would need quoted.
///
/// refinery builds its history `INSERT` itself and interpolates the table name
/// unquoted, so a name this driver would have to quote could never work. It is
/// better to fail loudly than to create a table that cannot be written to.
pub(crate) fn validate_table_name(name: &str) -> Result<(), Error> {
    let valid = !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidTableName(name.to_owned()))
    }
}

/// DDL that brings the history table up to the shape this driver expects.
///
/// Every statement is `IF NOT EXISTS`, so this is a no-op after the first run.
/// That also means the shape is effectively frozen once a deployment has
/// migrated: changing it later needs an explicit upgrade, not an edit here.
///
/// There is no `BEGIN`/`COMMIT` here because [`crate::MigrationConnection`]
/// runs every batch inside a transaction of its own.
pub(crate) fn assert_table(table: &str) -> String {
    if validate_table_name(table).is_err() {
        return throw_invalid_table_name(table);
    }
    format!(
        "DEFINE TABLE IF NOT EXISTS {table} SCHEMAFULL;
         DEFINE FIELD IF NOT EXISTS version ON {table} TYPE int;
         DEFINE FIELD IF NOT EXISTS name ON {table} TYPE string;
         DEFINE FIELD IF NOT EXISTS applied_on ON {table} TYPE any VALUE <datetime> $value;
         DEFINE FIELD IF NOT EXISTS checksum ON {table} TYPE string;
         DEFINE INDEX IF NOT EXISTS {table}_version ON {table} FIELDS version UNIQUE;"
    )
}

/// Every applied migration, oldest first.
///
/// The ascending order is required, not cosmetic: refinery derives the current
/// schema version from the last element of this list.
pub(crate) fn applied_migrations(table: &str) -> String {
    if validate_table_name(table).is_err() {
        return throw_invalid_table_name(table);
    }
    format!("SELECT {COLUMNS} FROM {table} ORDER BY version ASC;")
}

/// The most recently applied migration.
pub(crate) fn last_applied_migration(table: &str) -> String {
    if validate_table_name(table).is_err() {
        return throw_invalid_table_name(table);
    }
    format!("SELECT {COLUMNS} FROM {table} ORDER BY version DESC LIMIT 1;")
}

/// A query that fails with a message naming the offending table.
///
/// The query-building methods of refinery's `AsyncMigrate` return a `String`
/// rather than a `Result`, so a bad table name cannot be reported directly.
/// `THROW` turns it into an ordinary statement error instead.
fn throw_invalid_table_name(table: &str) -> String {
    let escaped = table.replace('\\', "\\\\").replace('"', "\\\"");
    format!("THROW \"invalid migration table name: {escaped}\";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_refinery_default_table_name() {
        assert!(validate_table_name("refinery_schema_history").is_ok());
    }

    #[test]
    fn rejects_names_that_would_need_quoting() {
        for name in ["", "with-hyphen", "with space", "1leading_digit", "sémi"] {
            assert!(
                validate_table_name(name).is_err(),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_a_surrealql_injection_payload() {
        let payload = "t; DEFINE TABLE evil SCHEMALESS; --";
        assert!(validate_table_name(payload).is_err());
        for query in [
            assert_table(payload),
            applied_migrations(payload),
            last_applied_migration(payload),
        ] {
            // The payload does appear in the message, but only as the argument
            // of a single THROW: the whole query must be one quoted string, so
            // the payload is inert data rather than executable statements.
            let inner = query
                .strip_prefix("THROW \"")
                .and_then(|rest| rest.strip_suffix("\";"))
                .unwrap_or_else(|| panic!("not a lone THROW statement: {query}"));
            assert!(
                !closes_the_string_literal(inner),
                "payload can escape the string literal: {query}"
            );
        }
    }

    /// Whether `inner` contains an unescaped `"` that would end the literal
    /// early and let the rest of the payload be parsed as SurrealQL.
    fn closes_the_string_literal(inner: &str) -> bool {
        let mut escaped = false;
        for character in inner.chars() {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => return true,
                _ => {}
            }
        }
        false
    }

    #[test]
    fn escapes_quotes_in_the_thrown_message() {
        let query = super::throw_invalid_table_name("a\"b\\c");
        assert_eq!(
            query,
            "THROW \"invalid migration table name: a\\\"b\\\\c\";"
        );
    }

    #[test]
    fn applied_migrations_are_ordered_ascending() {
        // refinery reads the current version from the last row, so a descending
        // order here would silently defeat its out-of-order protection.
        assert!(applied_migrations("hist").contains("ORDER BY version ASC"));
    }

    #[test]
    fn last_applied_migration_orders_by_version() {
        let query = last_applied_migration("hist");
        assert!(query.contains("ORDER BY version DESC"), "{query}");
        assert!(query.contains("LIMIT 1"), "{query}");
    }

    #[test]
    fn converts_timestamps_without_losing_precision() {
        let value = DateTime::from_timestamp(1_700_000_000, 123_456_789).unwrap();
        let converted = to_offset_datetime(&value).unwrap();
        assert_eq!(converted.unix_timestamp(), 1_700_000_000);
        assert_eq!(converted.nanosecond(), 123_456_789);
    }

    #[test]
    fn checksum_round_trips_the_full_u64_range() {
        for original in [0, 1, i64::MAX as u64, u64::MAX] {
            let value = Checksum(original).into_value();
            assert_eq!(Checksum::from_value(value).unwrap().0, original);
        }
    }
}
