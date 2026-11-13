# Tests

Four layers, all runnable with one command each:

| Layer                                | Where                             | Needs a database |
| ------------------------------------ | --------------------------------- | ---------------- |
| Unit tests, fakes for the boundaries  | `#[cfg(test)]` next to the code    | no               |
| HTTP tests through the real router    | `tests/auth_routes.rs`, `tests/iam_routes.rs`, `tests/schema_routes.rs` | no |
| gRPC tests through a real socket      | `tests/schema_rpc.rs`, `tests/auth_rpc.rs` | no      |
| Repository, transaction and RBAC tests | `tests/postgres.rs`               | yes              |

`tests/support/mod.rs` holds the fakes that stand in for the auth, RBAC, user
and permission services, so the HTTP tests exercise the real middleware stack,
extractors, routing and error rendering without any infrastructure. It also
holds a fake `SchemaRepository`: the transport tests are about the JSON and
protobuf mapping, and the store itself is covered in `tests/postgres.rs`.

`tests/schema_rpc.rs` starts a tonic server on port 0 and talks to it with the
generated client, so it exercises the wire rather than the handler in process.
That server carries the same layers the real one does, and its client sends a
token, which is what `tests/auth_rpc.rs` takes away to check the refusals.

`tests/postgres.rs` **skips itself** unless `TEST_DATABASE_URL` is set, which
keeps `cargo test` green on a bare checkout. An empty database is enough,
since the tests apply the embedded migrations themselves:

```bash
just test        # skips the database layer
just test-all    # starts postgres and runs everything
```

Those tests namespace every row they insert, so they are safe to run in
parallel and against a database that already holds the seed data.
