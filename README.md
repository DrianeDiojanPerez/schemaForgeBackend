<p align="center"><img src="assets/banner.png" alt="schemaforge: the API behind the schema designer" width="100%"></p>

- [Run with Docker](#run-with-docker)
- [Run locally](#run-locally)
- [Test](#test)
- [Docs](#docs)

## Run with Docker

```
cp .env.example .env
just up
```

- API: http://localhost:3000
- API reference: http://localhost:3000/docs
- Mailpit inbox: http://localhost:8025

The seeded administrator is `admin@example.com`.

## Run locally

```
just setup
just run
```

`just setup` copies `.env` and starts postgres and mailpit. The build needs `protoc`, so install `protobuf-compiler` first.

## Test

```
just test
just test-all
```

`just test` needs nothing running. `just test-all` starts postgres and adds the database tests.

## Docs

- [How it is built](docs/how-it-is-built.md): the stack, the layout, the `package` folder and design notes
- [Endpoints](docs/endpoints.md): every HTTP route, the API reference and the Bruno collection
- [gRPC](docs/grpc.md): the services, who may call what, and reflection
- [Commands](docs/commands.md): `just`, running, and the Docker targets
- [Tests](docs/tests.md)
- [Migrations](docs/migrations.md)
- [Logging](docs/logging.md)
- [Deploying](docs/deploying.md): Fly.io
