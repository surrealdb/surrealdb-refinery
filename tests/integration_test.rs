use maplit::hashmap;
use refinery::embed_migrations;
use serde_json;
use std::collections::HashMap;
use surrealdb::Value;
use surrealdb_refinery::MigrationConnection;

#[tokio::test]
async fn test_applies_migrations() {
    testing_logger::setup();

    embed_migrations!("tests/migrations");

    let runner = migrations::runner();
    let db = surrealdb::engine::any::connect("memory").await.unwrap();
    db.use_ns("test").await.unwrap();
    db.use_db("test").await.unwrap();
    let db2 = db.clone();
    let mut connection = MigrationConnection(&db);
    runner.run_async(&mut connection).await.unwrap();

    let mut res = db2.query("INFO FOR DB").await.unwrap();
    let errors = res.take_errors();
    if !errors.is_empty() {
        panic!("error getting info from test db: {:?}", errors);
    }

    let tables: Vec<HashMap<String, String>> = res.take("tables").unwrap();
    let expected_tables: Vec<HashMap<String, String>> = vec![hashmap! {
        "table_1".to_string() => "DEFINE TABLE table_1 TYPE NORMAL SCHEMAFULL PERMISSIONS NONE".to_string(),
        "table_2".to_string() => "DEFINE TABLE table_2 TYPE NORMAL SCHEMAFULL PERMISSIONS NONE".to_string(),
        "refinery_schema_history".to_string() => "DEFINE TABLE refinery_schema_history TYPE NORMAL SCHEMAFULL PERMISSIONS NONE".to_string()
    }];
    assert_eq!(tables, expected_tables);

    // Validate that certain log messages were written
    testing_logger::validate(|captured_logs| {
        assert!(
            captured_logs
                .iter()
                .any(|log| { log.body.contains("schema history table is empty") })
        );
        assert!(
            captured_logs
                .iter()
                .any(|log| { log.body.contains("applying migration") })
        );
    });
}

#[tokio::test]
async fn test_applies_migrations_only_once() {
    testing_logger::setup();

    embed_migrations!("tests/migrations");

    let runner = migrations::runner();
    let db = surrealdb::engine::any::connect("memory").await.unwrap();
    db.use_ns("test").await.unwrap();
    db.use_db("test").await.unwrap();
    let mut connection = MigrationConnection(&db);
    let _ = runner.run_async(&mut connection).await.unwrap();
    let _ = runner.run_async(&mut connection).await.unwrap();

    // This ensures that refinery emitted the 'no migrations to apply' log,
    // confirming that migrations are not re-applied.
    testing_logger::validate(|captured_logs| {
        assert!(
            captured_logs
                .iter()
                .any(|log| { log.body.contains("no migrations to apply") })
        );
    });
}
