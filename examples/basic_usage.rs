//! Applies the SurrealQL migrations in `examples/migrations` to an in-memory
//! database.
//!
//! Run with: `cargo run --example basic_usage`

use surrealdb_refinery::{MigrationConnection, Runner, load_migrations};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Discover `V{version}__{name}.surql` files on disk, ordered by version.
    let migrations = load_migrations("examples/migrations")?;
    println!("discovered {} migration(s)", migrations.len());

    let db = surrealdb::engine::any::connect("mem://").await?;
    // A namespace and database must be selected before migrating.
    db.use_ns("example").await?;
    db.use_db("example").await?;

    let mut connection = MigrationConnection(&db);
    let report = Runner::new(&migrations).run_async(&mut connection).await?;

    for migration in report.applied_migrations() {
        println!("applied V{} {}", migration.version(), migration.name());
    }

    Ok(())
}
