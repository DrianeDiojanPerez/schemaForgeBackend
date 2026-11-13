# Commands

`just` is the task runner. Run `just` on its own for the full list.

```bash
just setup         # copy .env, start postgres and mailpit
just run           # cargo run
just watch         # cargo watch -x run
just test          # unit and HTTP tests, no infrastructure needed
just test-all      # the above plus the tests that need a database
just lint          # cargo clippy --all-targets -- -D warnings
just fmt           # cargo fmt --all
just check         # fmt-check + lint + test
just up            # build the image and start the stack
just start         # start it again without rebuilding
just rebuild       # rebuild from scratch, ignoring cached layers
just down          # stop it
just down-hard     # stop it and drop the volumes, cargo caches included
just logs / ps
just api           # run the Bruno collection against a running server
```

A `Makefile` mirroring the same targets is included as well.

## Running

With Docker, the server applies the migrations itself on start up, so there
is no separate migrate step.

On the host, `.env.example` points at `127.0.0.1`, so the tooling works out
of the box. Compose overrides `DB_HOST` and `MAIL_HOST` for the containerised
app.

`DB_SSL_MODE` defaults to `prefer`, which is what the local container wants. A
hosted database such as Neon needs `require`. The `prod` compose profile reads
`.env.production`, which stays out of git.

## Docker targets

The `Dockerfile` is multi stage:

- `development` hot reloads through `cargo watch`
- `test` runs `cargo test --all-targets`
- `builder` produces the release binary
- `production` ships that binary on `debian:bookworm-slim` as a non root user

Compose profiles: `dev`, `uat`, `prod`, `test`.
