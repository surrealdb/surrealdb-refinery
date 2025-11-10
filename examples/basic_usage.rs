use surrealdb_refinery::MigrationConnection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("SurrealDB Refinery Example");
    // Define the migrations to run, the macro: refinery::embed_migrations can
    // be also used to define the migrations to run.
    let migrations = vec![
        refinery_core::Migration::unapplied(
            "V1__add_test_table.surql",
            "CREATE TABLE test SCHEMALESS",
        )
        .unwrap(),
    ];
    let db = surrealdb::engine::any::connect("memory").await.unwrap();
    db.use_ns("ns").await.unwrap();
    db.use_db("app").await.unwrap();
    let mut connection = MigrationConnection(&db);
    let runner = refinery::Runner::new(&migrations);

    // Run migrations
    println!("Running migrations...");
    runner.run_async(&mut connection).await.unwrap();
    println!("Example completed successfully!");
    Ok(())
}
