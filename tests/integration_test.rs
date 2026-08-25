//! End-to-end tests against an in-memory SurrealDB.
//!
//! Several of these are regression tests for defects that the previous
//! implementation had while the whole suite still passed, so each one states
//! what would break if it were removed.

use std::collections::HashMap;

use refinery_core::error::Kind;
use surrealdb::Surreal;
use surrealdb::engine::any::{self, Any};
use surrealdb_refinery::{MigrationConnection, Runner, load_migrations};

async fn fresh_db() -> Surreal<Any> {
    let db = any::connect("mem://").await.expect("connect to mem://");
    db.use_ns("test").await.expect("use_ns");
    db.use_db("test").await.expect("use_db");
    db
}

async fn run(db: &Surreal<Any>, dir: &str) -> Result<Vec<String>, refinery_core::Error> {
    let migrations = load_migrations(dir).expect("discovery");
    let mut connection = MigrationConnection(db);
    let report = Runner::new(&migrations).run_async(&mut connection).await?;
    Ok(report
        .applied_migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect())
}

async fn tables(db: &Surreal<Any>) -> Vec<String> {
    let mut response = db.query("INFO FOR DB").await.expect("INFO FOR DB");
    let info: Vec<HashMap<String, String>> = response.take("tables").expect("tables");
    let mut names: Vec<String> = info
        .first()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

async fn history_versions(db: &Surreal<Any>) -> Vec<i64> {
    let mut response = db
        .query("SELECT VALUE version FROM refinery_schema_history ORDER BY version ASC")
        .await
        .expect("history query");
    response.take(0).expect("versions")
}

#[tokio::test]
async fn applies_migrations_in_version_order() {
    let db = fresh_db().await;
    let applied = run(&db, "tests/fixtures/v1_v2_v3").await.unwrap();
    assert_eq!(applied, ["one", "two", "three"]);
    assert_eq!(history_versions(&db).await, [1, 2, 3]);
}

#[tokio::test]
async fn a_second_run_applies_nothing() {
    let db = fresh_db().await;
    run(&db, "tests/fixtures/v1_v2_v3").await.unwrap();

    // Asserted on the Report rather than on refinery's log wording: the old
    // log-string assertion passed even when zero migrations were applied.
    let applied = run(&db, "tests/fixtures/v1_v2_v3").await.unwrap();
    assert!(applied.is_empty(), "re-applied {applied:?}");
    assert_eq!(history_versions(&db).await, [1, 2, 3]);
}

/// Regression: `get_applied_migrations_query` used to order by version
/// DESC. refinery derives the current schema version from the *last* row, so
/// descending order made it read the lowest applied version and silently apply
/// a migration out of order instead of aborting.
#[tokio::test]
async fn aborts_when_a_migration_is_missing_from_history() {
    let db = fresh_db().await;
    run(&db, "tests/fixtures/v1_v3").await.unwrap();
    assert_eq!(history_versions(&db).await, [1, 3]);

    let error = run(&db, "tests/fixtures/v1_v2_v3")
        .await
        .expect_err("V2 arriving below the current version must abort");
    assert!(
        matches!(error.kind(), Kind::MissingVersion(_)),
        "expected MissingVersion, got {:?}",
        error.kind()
    );

    // Nothing was applied, and V2's table was never created.
    assert_eq!(history_versions(&db).await, [1, 3]);
    assert!(!tables(&db).await.contains(&"thing_two".to_string()));
}

#[tokio::test]
async fn aborts_when_an_applied_migration_diverges() {
    let db = fresh_db().await;
    run(&db, "tests/fixtures/v1_v3").await.unwrap();

    let error = run(&db, "tests/fixtures/divergent")
        .await
        .expect_err("a changed migration body must abort");
    assert!(
        matches!(error.kind(), Kind::DivergentVersion(_, _)),
        "expected DivergentVersion, got {:?}",
        error.kind()
    );
}

/// Regression: the migration body and the history row are now written in one
/// transaction. A migration that fails must leave no history row behind, or the
/// next run would skip it.
#[tokio::test]
async fn a_failing_migration_is_not_recorded() {
    let db = fresh_db().await;
    let error = run(&db, "tests/fixtures/bad_sql")
        .await
        .expect_err("invalid SurrealQL must fail the run");
    let message = format!("{error:?}");
    assert!(
        message.contains("V1__bad"),
        "should name the migration: {message}"
    );
    assert!(
        message.contains("Parse error"),
        "should be a parse error: {message}"
    );

    // The history table exists (it is asserted before migrating) but is empty.
    assert!(history_versions(&db).await.is_empty());
}

/// Regression: a migration whose later statements fail must roll back its
/// earlier ones. Previously each statement was its own transaction, so the
/// first `DEFINE TABLE` would persist.
#[tokio::test]
async fn a_partially_failing_migration_rolls_back() {
    let db = fresh_db().await;
    run(&db, "tests/fixtures/partial")
        .await
        .expect_err("the ASSERT should reject the insert");

    assert!(history_versions(&db).await.is_empty());
    assert!(
        !tables(&db).await.contains(&"created_first".to_string()),
        "the table defined before the failure should have been rolled back, tables: {:?}",
        tables(&db).await
    );
}

/// The driver opens the transaction, so a migration must not open its own.
/// SurrealDB refuses the nested BEGIN and the batch is cancelled, which must
/// leave no trace rather than half-applying the body.
#[tokio::test]
async fn refuses_a_migration_that_opens_its_own_transaction() {
    let db = fresh_db().await;
    let error = run(&db, "tests/fixtures/wrapped")
        .await
        .expect_err("a self-wrapping migration must fail");
    assert!(
        format!("{error:?}").contains("BEGIN a transaction within a transaction"),
        "unhelpful error: {error:?}"
    );

    assert!(history_versions(&db).await.is_empty());
    let tables = tables(&db).await;
    assert!(!tables.contains(&"table_1".to_string()), "{tables:?}");
    assert!(!tables.contains(&"table_2".to_string()), "{tables:?}");
}

#[tokio::test]
async fn accepts_a_migration_without_its_own_transaction() {
    let db = fresh_db().await;
    let applied = run(&db, "tests/fixtures/plain").await.unwrap();
    assert_eq!(applied, ["plain"]);
    assert!(tables(&db).await.contains(&"plain_table".to_string()));
}

/// Regression: the history table had no uniqueness on `version`, so two
/// concurrent runners could both record the same migration.
#[tokio::test]
async fn the_history_table_rejects_a_duplicate_version() {
    let db = fresh_db().await;
    run(&db, "tests/fixtures/v1_v2_v3").await.unwrap();

    let mut response = db
        .query(
            "INSERT INTO refinery_schema_history \
             (version, name, applied_on, checksum) \
             VALUES (1, 'one', '2024-01-01T00:00:00Z', '1')",
        )
        .await
        .expect("query");
    let errors = response.take_errors();
    assert!(!errors.is_empty(), "a duplicate version must be rejected");
    assert!(
        errors
            .values()
            .any(|e| e.to_string().contains("already contains")),
        "{errors:?}"
    );
}

#[tokio::test]
async fn round_trips_a_full_range_checksum_and_sub_second_timestamp() {
    // The checksum column is a string, not an int, because refinery checksums
    // are u64 and SurrealDB integers are i64. applied_on is a real datetime.
    // Both are written by refinery's own INSERT, so pin the extremes here.
    let db = fresh_db().await;
    run(&db, "tests/fixtures/plain").await.unwrap();

    db.query(
        "UPDATE refinery_schema_history SET \
         checksum = '18446744073709551615', \
         applied_on = <datetime>'2024-01-01T00:00:00.123456789Z'",
    )
    .await
    .expect("query")
    .check()
    .expect("update the history row");

    let migrations = load_migrations("tests/fixtures/plain").unwrap();
    let mut connection = MigrationConnection(&db);
    let last = Runner::new(&migrations)
        .get_last_applied_migration_async(&mut connection)
        .await
        .expect("decoding a u64::MAX checksum must not fail")
        .expect("one applied migration");

    assert_eq!(last.checksum(), u64::MAX);
    let applied_on = last.applied_on().expect("applied_on");
    assert_eq!(applied_on.unix_timestamp(), 1_704_067_200);
    assert_eq!(applied_on.nanosecond(), 123_456_789);
}

#[tokio::test]
async fn uses_a_custom_history_table() {
    let db = fresh_db().await;
    let migrations = load_migrations("tests/fixtures/plain").unwrap();
    let mut connection = MigrationConnection(&db);
    Runner::new(&migrations)
        .set_migration_table_name("my_history")
        .run_async(&mut connection)
        .await
        .expect("run with a custom table");

    assert!(tables(&db).await.contains(&"my_history".to_string()));
    assert!(
        !tables(&db)
            .await
            .contains(&"refinery_schema_history".to_string())
    );
}

/// A table name needing quotes cannot work, because refinery interpolates it
/// into its own INSERT unquoted. It must be refused, not half-applied.
#[tokio::test]
async fn rejects_a_history_table_name_that_needs_quoting() {
    let db = fresh_db().await;
    let migrations = load_migrations("tests/fixtures/plain").unwrap();
    let mut connection = MigrationConnection(&db);
    let error = Runner::new(&migrations)
        .set_migration_table_name("not-an-identifier")
        .run_async(&mut connection)
        .await
        .expect_err("an unquotable table name must be refused");

    let message = format!("{:?}", error);
    assert!(
        message.contains("invalid migration table name"),
        "unhelpful error: {message}"
    );
    assert!(!tables(&db).await.contains(&"plain_table".to_string()));
}

/// The table name reaches four SurrealQL templates by interpolation, so a name
/// carrying statement separators must not be able to execute them.
#[tokio::test]
async fn a_history_table_name_cannot_inject_surrealql() {
    let db = fresh_db().await;
    let migrations = load_migrations("tests/fixtures/plain").unwrap();
    let mut connection = MigrationConnection(&db);
    let error = Runner::new(&migrations)
        .set_migration_table_name("t; DEFINE TABLE evil SCHEMALESS; --")
        .run_async(&mut connection)
        .await
        .expect_err("an injection payload must be refused");

    assert!(format!("{error:?}").contains("invalid migration table name"));
    let tables = tables(&db).await;
    assert!(
        !tables.contains(&"evil".to_string()),
        "injected: {tables:?}"
    );
    assert!(!tables.contains(&"t".to_string()), "injected: {tables:?}");
}

#[tokio::test]
async fn grouped_mode_applies_every_migration() {
    let db = fresh_db().await;
    let migrations = load_migrations("tests/fixtures/v1_v2_v3").unwrap();
    let mut connection = MigrationConnection(&db);
    let report = Runner::new(&migrations)
        .set_grouped(true)
        .run_async(&mut connection)
        .await
        .expect("grouped run");

    assert_eq!(report.applied_migrations().len(), 3);
    assert_eq!(history_versions(&db).await, [1, 2, 3]);
}

#[tokio::test]
async fn stops_at_a_target_version() {
    let db = fresh_db().await;
    let migrations = load_migrations("tests/fixtures/v1_v2_v3").unwrap();
    let mut connection = MigrationConnection(&db);
    Runner::new(&migrations)
        .set_target(surrealdb_refinery::Target::Version(2))
        .run_async(&mut connection)
        .await
        .expect("targeted run");

    assert_eq!(history_versions(&db).await, [1, 2]);
}
