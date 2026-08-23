<br>

<p align="center">
    <img width=120 src="https://raw.githubusercontent.com/surrealdb/icons/main/surreal.svg" />
</p>

<h3 align="center">The official SurrealDB Driver for refinery</h3>

<br>

<p align="center">
    <a href="https://surrealdb.com/discord"><img src="https://img.shields.io/discord/902568124350599239?label=discord&style=flat-square&color=5a66f6"></a>
    &nbsp;
    <a href="https://twitter.com/surrealdb"><img src="https://img.shields.io/badge/twitter-follow_us-1d9bf0.svg?style=flat-square"></a>
    &nbsp;
    <a href="https://www.linkedin.com/company/surrealdb/"><img src="https://img.shields.io/badge/linkedin-connect_with_us-0a66c2.svg?style=flat-square"></a>
    &nbsp;
    <a href="https://www.youtube.com/channel/UCjf2teVEuYVvvVC-gFZNq6w"><img src="https://img.shields.io/badge/youtube-subscribe-fc1c1c.svg?style=flat-square"></a>
</p>

# SurrealDB Refinery

A [Refinery](https://github.com/rust-db/refinery) driver for [SurrealDB](https://surrealdb.com/), enabling database schema migrations using SurrealQL.

The API is not yet stable. Expect breaking changes before `1.0.0`.

## Installation

```toml
[dependencies]
surrealdb-refinery = "0.1"
surrealdb = "3.0"
refinery-core = "0.9"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`refinery-core` is needed for `Runner`, which drives the migration run. The
minimum supported Rust version is 1.89.

## Usage

Create migration files in a `migrations/` directory:

**`migrations/V1__create_users.surql`**

```surrealql
BEGIN;

DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD name ON users TYPE string;
DEFINE FIELD email ON users TYPE string;
DEFINE INDEX email_idx ON users FIELDS email UNIQUE;

COMMIT;
```

Run them from your application:

```rust
use surrealdb_refinery::{MigrationConnection, load_migrations};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Discover `V{version}__{name}.surql` files on disk, ordered by version.
    let migrations = load_migrations("migrations")?;

    let db = surrealdb::engine::any::connect("mem://").await?;
    // A namespace and database must be selected before migrating.
    db.use_ns("myapp").await?;
    db.use_db("myapp").await?;

    let mut connection = MigrationConnection(&db);
    let report = refinery_core::Runner::new(&migrations)
        .run_async(&mut connection)
        .await?;

    for migration in report.applied_migrations() {
        println!("applied V{} {}", migration.version(), migration.name());
    }

    Ok(())
}
```

A runnable version of this is in [`examples/basic_usage.rs`](examples/basic_usage.rs):

```sh
cargo run --example basic_usage
```

`MigrationConnection` borrows the `Surreal` handle, so pass `&db`. Do not change
the session's namespace or database while a migration run is in progress — the
run would continue against the new target.

## Migration files

Migration files follow the pattern `V{version}__{description}.surql`:

- `V1__initial_schema.surql`
- `V2__add_users_table.surql`
- `V3__add_indexes.surql`

The `.sql` extension is also accepted. A file carrying either extension whose
name does not match the pattern is an error, not a warning — a silently skipped
migration is how a database ends up half-migrated.

Wrapping a multi-statement migration in `BEGIN` / `COMMIT`, as above, makes that
migration's body apply atomically. Note that SurrealDB does not support nested
transactions.

Applied migrations are recorded in a `refinery_schema_history` table. Change the
name with `Runner::set_migration_table_name` if two applications share one
database, so they do not contend over the same history.

## Notes and limitations

- **Async only.** There is no blocking `Migrate` implementation, just
  `AsyncMigrate`.
- **Migrations are read at runtime**, not embedded at compile time. The
  directory must exist wherever the binary runs. refinery's
  `embed_migrations!` only discovers `.sql` and `.rs` files, which is why this
  crate does its own discovery.
- **The history record is a separate write** from the migration body. If the
  process dies between the two, a migration can be applied without being
  recorded, and will be applied again on the next run. Prefer idempotent
  migrations (`IF NOT EXISTS`) until this is addressed.
- Migration file names must not contain an apostrophe. refinery interpolates the
  name into the history `INSERT` unquoted, and this driver cannot override that.

## License

Licensed under the Apache License, Version 2.0.
