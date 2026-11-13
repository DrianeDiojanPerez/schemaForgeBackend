# gRPC

`build.rs` runs `tonic-prost-build` over `proto/schemaforge/v1/*.proto` on
every build, so the Rust types are generated from the contract rather than
written twice. `src/rpc.rs` is where the generated module is included.

| Service         | Methods                                                     |
| --------------- | ----------------------------------------------------------- |
| `HealthService` | `Check`                                                      |
| `AuthService`   | `Login`, `Register`, `RefreshToken`, `GoogleLoginUrl`, `LoginWithGoogle`, `GetCurrentUser` |
| `SchemaService` | `CreateSchema`, `GetSchema`, `ListSchemas`, `UpdateSchema`, `DeleteSchema`, `ValidateSchema`, `GenerateDdl` |

The two transports share everything below the adapter. `SchemaService` in
`core/ports.rs` is what both call, so a rule fixed in the core is fixed for
both at once. Only the mapping differs: `adapter/handler/` maps JSON and
`adapter/rpc/` maps protobuf.

There is no gRPC-web layer. The frontend server calls this port over native
gRPC and the browser talks JSON to `APP_PORT`.

The build needs `protoc`. It is in the Docker image already; locally install
`protobuf-compiler`.

## Who may call what

`HealthService`, `AuthService` and reflection are open. Everything on
`SchemaService` needs an access token in `authorization` metadata and the
permission for the method:

| Method                                                     | Permission         |
| ---------------------------------------------------------- | ------------------ |
| `GetSchema`, `ListSchemas`, `ValidateSchema`, `GenerateDdl` | `Schemas.View All` |
| `CreateSchema`                                              | `Schemas.Create`   |
| `UpdateSchema`                                              | `Schemas.Update`   |
| `DeleteSchema`                                              | `Schemas.Delete`   |

`AuthLayer` in `src/server/middlewares/rpc_auth.rs` is where that happens. It
goes through the same `Auth` and `Engine` the HTTP middleware uses, so a token
and a permission mean the same thing on either port, and it refuses a call
before the handler runs rather than inside it. gRPC has no route table to hang
a guard on, so the method path is the route and `access_for` is the table. A
method added to `SchemaService` and left off it answers denied.

Note the asymmetry while it lasts: the same schema operations over HTTP are
still open, so the gRPC check is a policy for the frontend server rather than
a lock on the data.

## Errors

`errdef::Error` renders as a `tonic::Status` the same way it renders as an
HTTP response. The transport carries only a coarse code and a string, so the
application code rides in `x-app-code` metadata and validation violations
ride in `x-validation-violations` as JSON.

## Reflection

Reflection is served outside production, which is what lets `grpcurl` work
without a copy of the `.proto` files:

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext localhost:50051 schemaforge.v1.HealthService/Check
```

`APP_ENVIRONMENT=production` turns it off, so the contract is not enumerable
by anyone who reaches the port. There is no separate switch to leave in the
wrong position.
