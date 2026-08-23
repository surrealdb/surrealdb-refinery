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
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

refinery's own types (`Runner`, `Migration`, `Report`, `Target`) are re-exported,
so there is no need to depend on `refinery` or `refinery-core` directly.

The minimum supported Rust version is 1.89.

### Features

`protocol-ws` and `protocol-http` are enabled by default, which is what
`connect("ws://…")` and `connect("http://…")` need. Embedded storage engines are
off by default, because a migration driver only needs the client API and
`kv-rocksdb` in particular forces a C++ RocksDB build on every consumer:

```toml
# for an embedded database, or for mem:// in tests
surrealdb-refinery = { version = "0.1", features = ["kv-mem"] }
```

Available: `protocol-ws`, `protocol-http`, `kv-mem`, `kv-rocksdb`, `kv-surrealkv`.

## Usage

Create migration files in a `migrations/` directory:

**`migrations/V1__create_users.surql`**

```surrealql
DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD name ON users TYPE string;
DEFINE FIELD email ON users TYPE string;
DEFINE INDEX email_idx ON users FIELDS email UNIQUE;
```

Run them from your application:

```rust
use surrealdb_refinery::{MigrationConnection, Runner, load_migrations};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Discover `V{version}__{name}.surql` files on disk, ordered by version.
    let migrations = load_migrations("migrations")?;

    let db = surrealdb::engine::any::connect("mem://").await?;
    // A namespace and database must be selected before migrating.
    db.use_ns("myapp").await?;
    db.use_db("myapp").await?;

    let mut connection = MigrationConnection(&db);
    let report = Runner::new(&migrations).run_async(&mut connection).await?;

    for migration in report.applied_migrations() {
        println!("applied V{} {}", migration.version(), migration.name());
    }

    Ok(())
}
```

A runnable version is in [`examples/basic_usage.rs`](examples/basic_usage.rs):

```sh
cargo run --example basic_usage
```

`MigrationConnection` borrows the `Surreal` handle, so pass `&db`. It is generic
over the connection type, so a local `Surreal<Db>` or a remote
`Surreal<Client>` works as well as `Surreal<Any>`. Do not change the session's
namespace or database while a run is in progress — the run holds no session of
its own and would continue against the new target.

## Migration files

Migration files follow the pattern `V{version}__{description}.surql`:

- `V1__initial_schema.surql`
- `V2__add_users_table.surql`
- `V3__add_indexes.surql`

The `.sql` extension is also accepted. A file carrying either extension whose
name does not match the pattern is an error, not a warning — a silently skipped
migration is how a database ends up half-migrated.

Migrations should **not** wrap themselves in `BEGIN` / `COMMIT`. The driver runs
each migration in a transaction of its own, and SurrealDB rejects a nested
`BEGIN`. Migrations written in the older wrapping style still work: a
`BEGIN`/`COMMIT` pair around the whole body is detected and removed.

## How migrations are applied

Each migration body and the history row recording it are executed in a single
SurrealDB transaction. A migration therefore cannot be applied without being
recorded, and if any statement fails the transaction is cancelled and nothing
from that migration persists — including DDL, which SurrealDB rolls back too.

Applied migrations are recorded in `refinery_schema_history`, which can be
renamed with `Runner::set_migration_table_name`. A `UNIQUE` index on `version`
makes double-application an error rather than a duplicate row, which is what
stops two concurrent runners from both applying the same migration.

The table name must be a bare SurrealDB identifier: ASCII letters, digits and
underscores, not starting with a digit. refinery interpolates it into its own
`INSERT` unquoted, so a name needing quotes is rejected rather than silently
mishandled.

## Notes and limitations

- **Async only.** There is no blocking `Migrate` implementation, just
  `AsyncMigrate`.
- **Migrations are read at runtime**, not embedded at compile time. The
  directory must exist wherever the binary runs. refinery's
  `embed_migrations!` only discovers `.sql` and `.rs` files, which is why this
  crate does its own discovery.
- Migration file names must not contain an apostrophe. refinery interpolates
  the name into the history `INSERT` unquoted, and this driver cannot override
  that query.
- The `surrealdb` client crate is BUSL-1.1 licensed, so that applies to your
  dependency graph even though this driver is Apache-2.0.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
