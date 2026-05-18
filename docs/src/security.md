# Security Guide

This page covers production hardening requirements and things to avoid.

---

## DOs

### Run the receiver behind a TLS-terminating reverse proxy

Bearer tokens are transmitted in plain HTTP headers. Without TLS, any network observer between the user and your server can extract a valid access token and use it to submit forged reports.

**Always** put the receiver behind nginx, Caddy, or another TLS-terminating proxy in production. Never expose the receiver's plain HTTP port directly.

### Set `COOKIE_SECURE=1` when serving the admin dashboard over HTTPS

The admin session cookie is `HttpOnly; SameSite=Strict` by default. When you set `COOKIE_SECURE=1`, the receiver also adds `; Secure`, which prevents the browser from sending the cookie over unencrypted connections.

```bash
COOKIE_SECURE=1
```

This must be set if your admin dashboard is served over HTTPS (i.e. always in production).

### Enable `REQUIRE_SIGNATURES=1` once all devices have registered

Ed25519 device signing provides replay protection and non-repudiation: each report is signed with a per-device private key that never leaves the user's machine. Once your initial users have set up the plugin, set this permanently:

```bash
REQUIRE_SIGNATURES=1
```

**This flag does not need to be toggled for new users.** Key registration (`POST /register-key`) has no signature requirement — it only checks an access token. A new user's binary registers its key on the first turn, then sends all reports signed. If registration is temporarily delayed, reports queue locally and are drained signed once registration succeeds. IT never needs to touch this setting after enabling it.

The only hard failure case is a binary older than v0.1.0, which predates signing support. Confirm existing devices have registered (admin dashboard → Device Keys) before enabling, then leave it on.

### Use a strong, unique `ADMIN_TOKEN`

The admin dashboard token is the only credential protecting access to all usage data. Generate it with:

```bash
openssl rand -hex 32
```

Store it in a secrets manager or environment file with restricted permissions (`chmod 600`). Rotate it periodically. Do not reuse it for any other purpose.

### Use one refresh token per user

Issuing a shared token to a team means all usage is attributed to one identity. You lose per-user visibility, which defeats the purpose of the tool. IT should issue exactly one token per person.

---

## DON'Ts

### Don't commit `config.json` or any file containing tokens

The file at `~/.claude/ccflux/config.json` contains a long-lived refresh token. If this file is committed to version control, anyone with repo access can use it to submit usage reports as that user until the token is revoked.

Add it to `.gitignore` in any project where users might run Claude Code from the project root:

```
ccflux/config.json
```

### Don't expose `/admin/` to the public internet without additional network controls

The admin dashboard is protected by a single bearer token. Consider also restricting it at the network level: firewall the `/admin/` path to your office IP range, VPN, or internal network. Defence in depth.

### Don't disable TLS (`CCFLUX_ALLOW_HTTP=1`) in production

The binary has a `CCFLUX_ALLOW_HTTP=1` escape hatch for local development. If this variable is set in a production wrapper script, bearer tokens are transmitted in plaintext. Remove it before distributing wrapper scripts to users.

Check your `plugin/scripts/` directory before tagging a release:

```bash
grep -r CCFLUX_ALLOW_HTTP plugin/scripts/
```

This should produce no output in a release build.

### Don't share the admin token with end users

End users have no reason to access the admin dashboard. The admin token grants full read access to all usage data for all users. Treat it like a root password.

---

## Security architecture notes

### Request signing

Every report is signed with the device's Ed25519 private key (`~/.claude/ccflux/signing_key`, mode `0600`). The signing message is:

```
<body bytes>\n<X-CCFLUX-Timestamp value>
```

The receiver verifies the signature against the registered public key for the user's email. The `X-CCFLUX-Timestamp` header must be within 5 minutes of the server clock (replay protection).

Signature errors return `403` with an `X-CCFLUX-Error` header. See [Configuration Reference — 403 error codes](./configuration.md#403-error-codes) for the full list.

### Token model

Users hold a long-lived **refresh token** (issued by IT). The binary exchanges it for a short-lived **access token** (default 8-hour lifetime) via `POST /token`. The access token is cached in `~/.claude/ccflux/token_cache.json` (mode `0600`) and refreshed automatically when within 5 minutes of expiry.

Access tokens are what reach `/report`. The refresh token never leaves the user's machine.

### Rate limiting

The receiver applies a per-token rate limit (default 30 requests per minute) across `/report`, `/token`, and `/register-key` endpoints. This prevents a leaked token from being used to flood the database.

### SQL injection prevention

All database queries use `sqlx` parameterised bindings. There is no string interpolation in SQL queries.

### XSS prevention

All user-supplied values rendered in the admin dashboard HTML are escaped through an `esc()` helper that HTML-encodes `&`, `<`, `>`, `"`, and `'`. Stored values like `device_id` and `user_email` cannot inject scripts into the dashboard.

### Constant-time comparisons

Token comparisons in the receiver use `subtle::ConstantTimeEq` to prevent timing side-channel attacks. This applies to both the access token verification and the admin token check.

### CSRF protection

All admin mutating endpoints require a hidden `csrf_token` form field verified server-side with a constant-time comparison. This covers device revoke, user provision, user revoke, and token reissue. Cross-origin form submissions cannot forge a valid CSRF token without knowing the `ADMIN_TOKEN`.

### Input length limits

The receiver enforces field length limits at the HTTP boundary before any database or cryptographic work:

| Endpoint | Field | Limit |
|---|---|---|
| `POST /register-key` | `public_key` | 64 characters |
| `POST /register-key` | `device_id` | 255 characters |
| `POST /report` | `session_id` | 64 characters |
| `POST /report` | `timestamp_utc`, `session_start_utc`, `plugin_version` | 64 characters each |
| `POST /report` | model names in `models` map | 128 characters each |
| `POST /report` | number of models in `models` map | 20 maximum |

Requests exceeding any limit are rejected with `400 Bad Request` before signature verification runs.
