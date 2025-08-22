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

## Installation

```toml
[dependencies]
surrealdb-refinery = "0.1.0"
surrealdb = "2.3"
refinery = { git = "https://github.com/manelmontilla/refinery", branch = "surql-files" }
tokio = { version = "1.0", features = ["full"] }
```

## Usage

Create migration files in a `migrations/` directory:

**`migrations/V1__create_users.surql`**

```surrealql
DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD name ON users TYPE string;
DEFINE FIELD email ON users TYPE string;
DEFINE INDEX email_idx ON users COLUMNS email UNIQUE;
```

Run migrations in your application:

```rust
use surrealdb_refinery::MigrationConnection;
use refinery::embed_migrations;

embed_migrations!("migrations");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = surrealdb::engine::any::connect("memory").await?;
    db.use_ns("myapp").await?;
    db.use_db("myapp").await?;
    let mut connection = MigrationConnection(db);
    let runner = migrations::runner();
    runner.run_async(&mut connection).await?;
    Ok(())
}
```

## Migration Files

Migration files follow the pattern: `V{version}__{description}.surql`

- `V1__initial_schema.surql`
- `V2__add_users_table.surql`
- `V3__add_indexes.surql`
