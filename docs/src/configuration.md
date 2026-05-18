# Configuration Reference

---

## Receiver environment variables

All receiver configuration is via environment variables. Unrecognised variables are ignored. On startup the receiver prints all resolved values.

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_PATH` | `ccflux.db` | Path to the SQLite database file. Created on first start. |
| `LISTEN_ADDR` | `0.0.0.0:8080` | TCP bind address. In production, bind to `127.0.0.1:8080` and put a reverse proxy in front. |
| `ACCESS_TOKEN_EXPIRY_SECS` | `28800` | How long access tokens live, in seconds. Default is 8 hours. |
| `REFRESH_TOKEN_ROLLING_DAYS` | `90` | Each successful token exchange extends the refresh token's expiry by this many days. Active users never need a new token. |
| `RATE_LIMIT_PER_MINUTE` | `30` | Max requests per access token per minute across `/report`, `/token`, and `/register-key`. Returns `429` when exceeded. |
| `BODY_LIMIT_KB` | `64` | Maximum request body size in kilobytes. Requests larger than this receive `413`. |
| `REQUIRE_SIGNATURES` | `false` | Set to `1` or `true` to reject `/report` requests that lack a valid Ed25519 signature. Enable once all devices have registered keys. |
| `ADMIN_TOKEN` | _(unset)_ | Enables the admin dashboard at `/admin/`. Must be a strong random string. Dashboard is disabled when unset. |
| `COOKIE_SECURE` | `false` | Set to `1` or `true` to add `; Secure` to the admin session cookie. Set this when serving over HTTPS. |

---

## Plugin binary environment variables

These are set automatically by Claude Code from the plugin's `userConfig` and do not need to be configured manually.

| Variable | Source | Description |
|----------|--------|-------------|
| `CLAUDE_PLUGIN_OPTION_API_ENDPOINT` | Plugin userConfig `api_endpoint` | The receiver URL for `/report`. |
| `CLAUDE_PLUGIN_OPTION_API_TOKEN` | Plugin userConfig `api_token` | The user's long-lived refresh token. |
| `CLAUDE_PLUGIN_ROOT` | Set by Claude Code | Absolute path to the plugin directory. Used by the binary to verify the transcript belongs to this CC instance. |

### Binary fallback: `config.json`

If the `CLAUDE_PLUGIN_OPTION_*` env vars are empty (e.g. when the plugin settings UI is unavailable), the binary reads:

```
<data_dir>/ccflux/config.json
```

Format:

```json
{
  "endpoint": "https://ccflux.example.org/report",
  "token": "rtok_abc123..."
}
```

Set this file to mode `0600`. If both the env vars and the config file are absent, the binary exits silently — no reporting occurs.

### Development-only variables

| Variable | Description |
|----------|-------------|
| `CCFLUX_ALLOW_HTTP=1` | Allows the binary to POST to `http://` endpoints. For local development only. **Never set this in production.** |
| `CCFLUX_CA_CERT=<path>` | Path to a PEM-encoded CA certificate to add to the TLS trust store. Use this when your receiver is behind a reverse proxy with a self-signed or internal CA cert (e.g. Caddy local CA). The cert is added on top of the bundled Mozilla root CAs — public CAs work without this variable. See [TLS with an internal CA](#tls-with-an-internal-ca) in Troubleshooting. |

---

## Plugin userConfig fields

Configured via Claude Code plugin settings UI or `CLAUDE_PLUGIN_OPTION_*` env vars.

| Field | Type | Description |
|-------|------|-------------|
| `api_endpoint` | string | Your receiver's `/report` URL, e.g. `https://ccflux.example.org/report` |
| `api_token` | string (sensitive) | Your personal refresh token. Stored in the system keychain. |

---

## State files

The binary stores state in `<data_dir>/ccflux/`. For the default CC installation this is `~/.claude/ccflux/`.

| File | Permissions | Description |
|------|-------------|-------------|
| `signing_key` | `0600` | Ed25519 private key bytes (raw, 64 bytes). Never transmitted. |
| `key_registered` | `0644` | Contains the base64-encoded public key that was successfully registered. Absent means unregistered. |
| `key_revoked` | `0644` | Marker file. Present means the device key was revoked by IT. Binary goes silent while this exists. |
| `token_cache.json` | `0600` | Cached access token and expiry. Refreshed automatically near expiry. |
| `pending_reports.jsonl` | `0644` | Queue of reports generated before the device key was registered. Max 500 entries. |
| `<session_id>.offset` | `0644` | Per-session offset: `{ "line": N, "turn": N, "session_start": "...", "closed": false }` |
| `activity.log` | `0644` | Rolling diagnostic log (~64 KB cap). Records token refreshes, key registrations, reports sent/queued, and errors. Check this first when troubleshooting. |
| `errors.log` | `0644` | Append-only error log. Errors are also mirrored here with an `ERROR` prefix. |
| `config.json` | `0600` | Optional fallback config (endpoint + token). Preferred: plugin settings UI. |

---

## SQLite schema

```sql
CREATE TABLE refresh_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    division      TEXT,
    expires_at  TIMESTAMP NOT NULL,
    revoked     INTEGER DEFAULT 0,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE access_tokens (
    token           TEXT PRIMARY KEY,
    refresh_token   TEXT NOT NULL REFERENCES refresh_tokens(token),
    email           TEXT NOT NULL,
    expires_at      TIMESTAMP NOT NULL,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE device_keys (
    public_key      TEXT PRIMARY KEY,
    email           TEXT NOT NULL,
    device_id       TEXT,
    registered_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_seen_at    TIMESTAMP,
    revoked         INTEGER DEFAULT 0
);

CREATE TABLE usage_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    received_at         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    user_email          TEXT NOT NULL,
    user_token          TEXT NOT NULL,
    session_id          TEXT NOT NULL,
    turn_index          INTEGER NOT NULL,
    timestamp_utc       TIMESTAMP NOT NULL,
    session_start_utc   TIMESTAMP,
    model               TEXT NOT NULL,
    input_tokens        INTEGER NOT NULL DEFAULT 0,
    output_tokens       INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens   INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens  INTEGER NOT NULL DEFAULT 0,
    plugin_version      TEXT,
    schema_version      INTEGER NOT NULL DEFAULT 1,
    UNIQUE(session_id, turn_index, model)
);
```

The `UNIQUE(session_id, turn_index, model)` constraint handles idempotent retries: duplicate POSTs use `INSERT OR IGNORE`.

---

## Receiver endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/token` | Refresh token (Bearer) | Exchange a refresh token for a short-lived access token. Returns `{"access_token": "...", "expires_at": "..."}`. |
| `POST` | `/register-key` | Access token (Bearer) | Register a device Ed25519 public key. Body: `{"public_key": "<base64>", "device_id": "<hostname>"}`. |
| `POST` | `/report` | Access token (Bearer) | Ingest a usage payload. See payload schema below. |
| `GET` | `/health` | None | Returns `{"status":"ok","db":"ok"}` or `503` if the DB is unreachable. |
| `GET` | `/metrics` | None | Prometheus text format counters and gauges. Restrict at the reverse proxy if exposing externally. |
| `GET` | `/admin/` | Admin token (cookie or Bearer) | Admin dashboard. Disabled unless `ADMIN_TOKEN` is set. |

---

## Usage payload schema

The binary POSTs this JSON structure to `/report`:

```json
{
  "schema_version": 1,
  "session_id": "uuid",
  "user_email": "jsmith@example.org",
  "turn_index": 42,
  "timestamp_utc": "2026-05-11T04:32:10Z",
  "session_start_utc": "2026-05-10T09:15:00Z",
  "models": {
    "claude-sonnet-4-6": {
      "input_tokens": 14200,
      "output_tokens": 1800,
      "cache_read_tokens": 9400,
      "cache_write_tokens": 0
    }
  },
  "plugin_version": "0.1.0"
}
```

A single turn may contain usage from multiple models (e.g. Sonnet for the main response plus a tool-use round with a different model). Each model gets its own key in `models`.

---

## Prometheus metrics

Available at `GET /metrics`. All counters reset on receiver restart (in-memory).

| Metric | Type | Description |
|--------|------|-------------|
| `ccflux_reports_accepted_total` | counter | Usage reports that returned HTTP 200 |
| `ccflux_reports_auth_rejected_total` | counter | Reports rejected due to invalid or expired token |
| `ccflux_reports_sig_rejected_total` | counter | Reports rejected due to signature failure |
| `ccflux_reports_rate_limited_total` | counter | Reports dropped due to rate limiting |
| `ccflux_token_exchanges_total` | counter | Successful refresh → access token exchanges |
| `ccflux_key_registrations_total` | counter | Successful device key registrations |
| `ccflux_active_access_tokens` | gauge | Current number of non-expired access tokens (queries DB) |

---

## 403 error codes

When `/report` returns `403`, the `X-CCFLUX-Error` response header contains a machine-readable code. The binary reads this and responds accordingly.

| Code | Description | Binary behaviour |
|------|-------------|-----------------|
| `key-revoked` | The device's Ed25519 key has been revoked by IT. | Logs to `errors.log`, clears `pending_reports.jsonl`, writes `key_revoked` marker. Goes silent until re-provisioned. |
| `timestamp-stale` | The `X-CCFLUX-Timestamp` header is more than 5 minutes old. | Logs to `errors.log`. For live reports this indicates >5 min clock skew. For queued reports, discards the entry (cannot be resent with a valid timestamp). |
| `signature-invalid` | The Ed25519 signature does not verify. | Logs to `errors.log`. Retries on the next turn. |
| `key-not-registered` | The public key in the signature header is not in the receiver's database. | Clears the `key_registered` marker file, queues the payload, retries registration on the next turn. |
| `signature-required` | `REQUIRE_SIGNATURES=1` is set and no signature headers were present. | Logged as a generic 403 failure. Upgrade the binary — all current versions sign requests. |
