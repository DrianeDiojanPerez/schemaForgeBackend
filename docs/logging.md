# Logging

Structured JSON is written to a daily rotated file under `LOGGER_DIRECTORY`
(`storage/logs/log.<date>.log`) and, outside production, to stdout. Every log
line inside an HTTP request carries `route_path`, `request_id`, `ip_address`
and `method`; a gRPC call carries `request_id` and `path`.

Every request logs twice, on the way in and on the way out:

```json
{"level":"INFO","fields":{"message":"request started","method":"POST","uri":"/v1/login"}}
{"level":"INFO","fields":{"message":"request completed","method":"POST","uri":"/v1/login","status":200,"latency_ms":301.02}}
```

`latency_ms` is fractional, since most requests finish well inside a
millisecond and would otherwise all read as `0`.

## Lines and levels

Both ports use the same levels:

| Line              | Level | Carries                                   |
| ----------------- | ----- | ----------------------------------------- |
| `request started` | info  | method, uri, query                        |
| `request headers` | debug | the request headers                       |
| `request completed`| info | status, latency                           |
| `response headers`| debug | the response headers                      |
| `request rejected`| warn  | a 4xx status and the latency              |
| `request failed`  | error | a 5xx status and the latency              |
| `rpc started`     | info  |                                           |
| `rpc metadata`    | debug | the call metadata                         |
| `rpc request`     | debug | the request message as JSON, credentials blanked |
| `rpc response`    | debug | the response message as JSON              |
| `rpc completed`   | info  | latency                                   |
| `rpc rejected`    | warn  | a client `grpc-status` and the latency    |
| `rpc failed`      | error | `grpc-status` 2, 13, 14 or 15             |

`Authorization` is marked sensitive on both ports, so it prints as `Sensitive`
rather than as the token.

Below the transport, every service writes one line per action in the past
tense: `schema created`, `signed in`, `user deleted`. Writes are `info`,
reads such as `schemas listed` are `debug`, and a refused sign in is `warn`.
At `debug` sqlx adds a line per statement with its elapsed time. A statement
that fails is a `warn` with the database's own wording, and one that takes
over a second is a `warn` too, at any level.

## Settings

`LOGGER_LEVEL` takes one of `trace|debug|info|warn|error` and defaults to
`info`, which shows every request, every service action and every failure.
`.env.example` ships `debug`, which adds the headers, the message bodies and
the statements while the frontend is being written. `RUST_LOG` overrides it
when set, and accepts the full tracing filter syntax.

`APP_ENVIRONMENT` takes one of `local|development|production` and defaults to
`development`; only `production` turns the stdout logger off. Both variables
reject anything else at start up rather than falling back to a default.
