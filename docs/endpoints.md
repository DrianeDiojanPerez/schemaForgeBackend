# Endpoints

Public:

| Method | Path                  | Description                     |
| ------ | --------------------- | ------------------------------- |
| GET    | `/v1/healthcheck`     | Liveness probe                  |
| GET    | `/docs`               | API reference, rendered         |
| GET    | `/openapi.yaml`       | The spec behind it              |
| POST   | `/v1/login`           | Issue an access + refresh token |
| POST   | `/v1/refresh-token`   | Exchange a refresh token        |
| POST   | `/v1/forgot-password` | Mail a password reset link      |
| POST   | `/v1/reset-password`  | Consume a reset token           |

Schemas, also public for now:

| Method | Path                       | Description                        |
| ------ | -------------------------- | ---------------------------------- |
| GET    | `/v1/schemas`              | List schemas, summaries only       |
| POST   | `/v1/schemas`              | Create a schema                    |
| GET    | `/v1/schemas/{schema-id}`  | Read one schema                    |
| PUT    | `/v1/schemas/{schema-id}`  | Replace one schema                 |
| DELETE | `/v1/schemas/{schema-id}`  | Delete one schema                  |
| POST   | `/v1/schemas/validate`     | Validate a stored schema or draft  |
| POST   | `/v1/schemas/generate-ddl` | Generate DDL from a schema or draft |

Sharing a schema, the way a document is shared. Only the owner decides who
gets in, each person as a `viewer` or an `editor`:

| Method | Path                                                   | Description                            |
| ------ | ------------------------------------------------------ | -------------------------------------- |
| GET    | `/v1/schemas/shared-with-me`                           | Schemas others let me in on            |
| GET    | `/v1/schemas/{schema-id}/people`                       | Who has access, and who is invited     |
| POST   | `/v1/schemas/{schema-id}/shares`                       | Share with an address, or invite it    |
| PATCH  | `/v1/schemas/{schema-id}/shares/{user-id}`             | Change someone's role                  |
| DELETE | `/v1/schemas/{schema-id}/shares/{user-id}`             | Take access away, or leave             |
| DELETE | `/v1/schemas/{schema-id}/invitations/{invitation-id}`  | Cancel an invitation                   |
| POST   | `/v1/invitations/{token}/accept`                       | Open the link from the invitation mail |

Authenticated (`Authorization: Bearer <token>`):

| Method | Path                    | Permission        |
| ------ | ----------------------- | ----------------- |
| GET    | `/v1/users`             | `Users.View All`  |
| POST   | `/v1/users`             | authenticated     |
| GET    | `/v1/users/my-user`     | authenticated     |
| PATCH  | `/v1/users/my-user`     | authenticated     |
| GET    | `/v1/users/{user-id}`   | authenticated     |
| PATCH  | `/v1/users/{user-id}`   | authenticated     |
| DELETE | `/v1/users/{user-id}`   | authenticated     |
| GET    | `/v1/permissions`       | authenticated     |

Members of the role named by `SUPER_ROLE` bypass every permission check.
It defaults to `Admin`.

Every response uses one envelope:

```json
{ "data": { }, "pagination": { }, "error": null }
```

## API reference

`GET /docs` renders the spec at `src/server/openapi.yaml`, which is embedded
into the binary, so the production image serves its own documentation with no
extra files to deploy. `GET /openapi.yaml` returns the raw spec for a client
generator.

The spec is not generated from the code, so it can drift. A test in
`src/server/docs.rs` fails if a route is served without being documented,
which catches the most common half of that.

## API collection

`api/bruno/API` is a [Bruno](https://usebruno.com) collection covering every
endpoint, laid out as `auth/` and `iam/{user,permission}/`.

Open the folder in Bruno and pick the `development` environment, or run it
headless:

```bash
just api                # whole collection
just api-folder auth    # one folder
```

`Login` stores `token`, `refresh_token` and `user_id` on the environment, and
every other request reads them from there, so run it first.

`Reset Password` is the one manual request: the reset token only exists in the
mail `Forgot Password` sends, since the database holds a digest of it. Grab it
from Mailpit on http://localhost:8025 and put it in the `reset_token`
environment variable.
