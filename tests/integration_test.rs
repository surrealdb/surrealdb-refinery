use surrealdb_refinery::{load_migrations, MigrationConnection};

#[tokio::test]
async fn applies_surql_migrations_without_the_fork() {
    let migrations = load_migrations("tests/migrations").expect("discovery failed");
    assert_eq!(migrations.len(), 1);

    let db = surrealdb::engine::any::connect("mem://").await.unwrap();
    db.use_ns("test").await.unwrap();
    db.use_db("test").await.unwrap();

    let runner = refinery_core::Runner::new(&migrations);
    let mut conn = MigrationConnection(&db);
    let report = runner.run_async(&mut conn).await.unwrap();

    // assert on the Report, not on refinery's log wording
    let applied: Vec<_> = report.applied_migrations().iter().map(|m| m.name().to_string()).collect();
    assert_eq!(applied, vec!["first".to_string()]);

    let mut res = db.query("INFO FOR DB").await.unwrap();
    let tables: Vec<std::collections::HashMap<String, String>> = res.take("tables").unwrap();
    let names: Vec<&String> = tables[0].keys().collect();
    assert!(names.iter().any(|n| *n == "table_1"), "got {names:?}");
    assert!(names.iter().any(|n| *n == "table_2"), "got {names:?}");
}
