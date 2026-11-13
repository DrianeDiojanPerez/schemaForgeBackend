# Deploying to Fly.io

`fly.toml` describes the app `schemaforge-api`, built from the `production`
target of the Dockerfile in the same region as the frontend. The database is
on Neon, so nothing but the API runs on Fly. How the API runs is in the
`[env]` table; where the data lives and every credential is a secret, set once:

```bash
fly secrets set DB_HOST=... DB_DATABASE=... DB_USERNAME=... DB_PASSWORD=... \
  JWT_SECRET=... \
  GOOGLE_CLIENT_ID=... GOOGLE_CLIENT_SECRET=... GOOGLE_REDIRECT_URI=... \
  MAIL_HOST=... MAIL_PORT=... MAIL_USERNAME=... MAIL_PASSWORD=...
just deploy
```

Mail goes out through Gmail: `MAIL_HOST=smtp.gmail.com`, `MAIL_PORT=587`, the
account as `MAIL_USERNAME` and an app password as `MAIL_PASSWORD`. Gmail only
sends as the account that signed in, so `MAIL_FROM_ADDRESS` in `fly.toml` is
that same address, and `MAIL_ENCRYPTION` is `starttls` because port 587 wants
it.

The HTTP port is public. The gRPC port is not: the frontend reaches it over
the private network as `schemaforge-api.internal:50051`, which is what its
`SCHEMAFORGE_GRPC_ADDRESS` secret holds. A private port is only there while
the machine runs, so the app keeps one machine up rather than stopping when
idle. The log directory is a volume, read with `fly ssh console`; nothing is
written to stdout in production.
