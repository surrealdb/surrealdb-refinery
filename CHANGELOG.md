# Changelog

## 0.1.0 — unreleased

First release. Previously the crate was unpublishable: it depended on a git
fork of refinery, which `cargo publish` refuses.

### Added

- `load_migrations`, which discovers `.surql` (and `.sql`) migration files.
  refinery's `embed_migrations!` only recognises `.sql` and `.rs`, and doing
  discovery here is what removed the need for the fork.
- A `UNIQUE` index on `version` in the history table, so a double-apply is an
  error rather than a duplicate row.
- Feature flags for the SurrealDB engines and protocols. Embedded engines are
  now off by default; enable `kv-mem`, `kv-rocksdb` or `kv-surrealkv` as needed.
- Re-exports of `Migration`, `Report`, `Runner`, `SchemaVersion` and `Target`,
  so callers need only depend on this crate.
- `Error`, a real error enum carrying per-statement failures, replacing the
  opaque `SurrealError`.

### Fixed

- Applied migrations were queried in descending version order. refinery reads
  the current schema version from the last row, so this disabled its
  out-of-order protection: a migration arriving below the current version was
  applied silently instead of aborting.
- The last-applied-migration query had an empty `ORDER BY` key, so it returned
  an arbitrary row rather than the highest version.
- A migration body and the history row recording it were written in separate
  transactions, so a failure between them left the migration applied but
  unrecorded. They are now one transaction, and a failure rolls back the whole
  migration including DDL.
- `State::into_value` mapped both variants to `1`. The `state` column is gone
  entirely: refinery never wrote it, so every applied row held `0`, which this
  driver decoded as `Unapplied`.
- Converting a history row could panic on a timestamp that did not parse, or on
  a row with no `applied_on`. Conversion is now fallible.
- The migration table name was interpolated into four SurrealQL templates
  unchecked.

### Changed

- `MigrationConnection` is generic over `surrealdb::Connection`, defaulting to
  `Any`, so local and remote handles both work.
- `applied_on` is stored as a SurrealDB `datetime` rather than a string.
- Migrations must no longer carry their own `BEGIN`/`COMMIT`; the driver
  provides the transaction, and SurrealDB refuses a nested `BEGIN`.

### Upgrading

The history table schema changed, and its DDL is `IF NOT EXISTS`, so it is not
altered on an existing deployment. A database migrated by a pre-release build
has `applied_on` as a string and will fail to decode. Drop
`refinery_schema_history` and re-run, or migrate the column by hand.

Re-running re-executes every migration, so remove any `BEGIN` and `COMMIT` lines
from your `.surql` files first: the driver now supplies the transaction.
