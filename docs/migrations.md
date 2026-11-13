# Migrations

Migrations live in `migrations/<module>/` as reversible sqlx pairs
(`<version>_<name>.up.sql` and `.down.sql`), **one directory per module**, so a
module can be migrated on its own. They are compiled into the binary by
`sqlx::migrate!`, so the production image ships without any SQL files and every
environment runs byte for byte the same schema.

```
migrations/
├── iam/
│   ├── 20250609223816_create_schema.up.sql
│   ├── 20250609223816_create_schema.down.sql
│   └── ...
└── schema/
    ├── 20260908090000_create_schema.up.sql
    └── ...
```

Each module owns a PostgreSQL schema of its own, `iam` and `schemaforge`, so
two modules cannot collide on a table name.

`src/database/migrations.rs` is the registry, and its order is the order they
are applied. That matters once one module references another's tables: put a
module after the ones it depends on.

```rust
pub fn all() -> Vec<ModuleMigrations> {
    vec![
        module("iam", sqlx::migrate!("./migrations/iam")),
        module("schema", sqlx::migrate!("./migrations/schema")),
        // module("catalog", sqlx::migrate!("./migrations/catalog")),
    ]
}
```

The server applies everything pending when it starts. sqlx takes an advisory
lock first, so several replicas booting together is safe. Set
`DB_RUN_MIGRATIONS=false` to gate schema changes behind a separate step, and
run them with the binary itself:

```bash
server migrate           # every module, in registry order
server migrate iam       # one module
```

All modules share the single `_sqlx_migrations` ledger. Each module's migrator
ignores the versions the others recorded, and versions are timestamps, so they
never collide.

## Adding a module

```bash
just migrate-new catalog create_catalogs_table   # creates migrations/catalog/
```

Then add one line to `all()` in `src/database/migrations.rs`. The macro reads
the directory at compile time, so the binary always carries what the source
tree holds.

## Recipes

`migrate` and `migrate-module` go through the binary, so they respect the
registry order. The rest need `sqlx-cli`, installed once with
`just migrate-install`.

```bash
just migrate                          # every module, registry order
just migrate-module iam               # one module
just migrate-down iam                 # revert that module's last migration
just migrate-status                   # per module, what has been applied
just migrate-status iam               # one module
just migrate-new iam create_foo_table # scaffold a reversible pair
just migrate-reset                    # drop, recreate, migrate (destructive)
```
