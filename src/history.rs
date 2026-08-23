//! The migration history table: its schema, its queries, and row decoding.

use refinery_core::{Migration, SchemaVersion};
use surrealdb::types::SurrealValue;
use surrealdb_types::Datetime;
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
    applied_on: Datetime,
    /// refinery checksums are `u64`, but SurrealDB integers are `i64`, so a
    /// checksum in the upper half of the range cannot be stored as a number.
    /// It is stored as a string, which round-trips the full range.
    checksum: String,
}

impl TryFrom<HistoryRow> for Migration {
    type Error = Error;

    fn try_from(row: HistoryRow) -> Result<Self, Error> {
        let version = SchemaVersion::try_from(row.version).map_err(|_| {
            Error::InvalidHistoryRow(format!("version {} out of range", row.version))
        })?;
        let checksum = row.checksum.parse::<u64>().map_err(|error| {
            Error::InvalidHistoryRow(format!("checksum {:?}: {error}", row.checksum))
        })?;
        let applied_on = to_offset_datetime(&row.applied_on).ok_or_else(|| {
            Error::InvalidHistoryRow(format!("applied_on {} out of range", *row.applied_on))
        })?;

        Ok(Migration::applied(version, row.name, applied_on, checksum))
    }
}

/// Convert the value SurrealDB hands back into the `time` value refinery
/// expects, without going through a string.
fn to_offset_datetime(value: &Datetime) -> Option<OffsetDateTime> {
    let seconds = OffsetDateTime::from_unix_timestamp(value.timestamp()).ok()?;
    Some(seconds + Duration::nanoseconds(i64::from(value.timestamp_subsec_nanos())))
}

/// Whether `name` is a bare SurrealDB identifier.
///
/// refinery builds its history `INSERT` itself and interpolates the table name
/// unquoted, so a name this driver would have to quote could never work. It is
/// better to fail loudly than to create a table that cannot be written to.
fn is_bare_identifier(name: &str) -> bool {
    !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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
    if !is_bare_identifier(table) {
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
    if !is_bare_identifier(table) {
        return throw_invalid_table_name(table);
    }
    format!("SELECT {COLUMNS} FROM {table} ORDER BY version ASC;")
}

/// The most recently applied migration.
pub(crate) fn last_applied_migration(table: &str) -> String {
    if !is_bare_identifier(table) {
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
        assert!(is_bare_identifier("refinery_schema_history"));
    }

    #[test]
    fn rejects_names_that_would_need_quoting() {
        for name in ["", "with-hyphen", "with space", "1leading_digit", "sémi"] {
            assert!(!is_bare_identifier(name), "{name:?} should be rejected");
        }
    }

    #[test]
    fn rejects_a_surrealql_injection_payload() {
        let payload = "t; DEFINE TABLE evil SCHEMALESS; --";
        assert!(!is_bare_identifier(payload));
        for query in [
            assert_table(payload),
            applied_migrations(payload),
            last_applied_migration(payload),
        ] {
            // The payload does appear in the message, but only as the argument
            // of a lone THROW, so it is inert data rather than statements.
            assert_eq!(
                query,
                "THROW \"invalid migration table name: t; DEFINE TABLE evil SCHEMALESS; --\";"
            );
        }
    }

    #[test]
    fn escapes_quotes_in_the_thrown_message() {
        // Without escaping, the quote would close the string literal and the
        // rest of the name would be parsed as SurrealQL.
        let query = throw_invalid_table_name("a\"b\\c");
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
}
