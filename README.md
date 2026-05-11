# ccflux

A Claude Code plugin that collects opt-in, per-turn token usage telemetry and ships it to a self-hosted receiver. Built for organisations on seat-based Enterprise Claude Code plans where Anthropic's Analytics dashboard does not expose per-user token counts.

**What it collects:** token counts, model name, session/turn identifiers, timestamps.  
**What it never collects:** message content, prompts, file paths, code, or anything identifying project content.

---

## How it works

A small Rust binary is invoked by Claude Code hook events (`SessionStart`, `Stop`, `SessionEnd`). After each assistant turn, it reads the new usage data from the session transcript, aggregates token counts by model, and POSTs a JSON payload to your organisation's receiver endpoint. The receiver stores events in SQLite for querying.

Multiple Claude Code aliases (e.g. `claude` for personal, `claude-work` for work) are handled automatically — the plugin only reports for the CC instance it is installed in, determined by comparing the transcript path against `CLAUDE_PLUGIN_ROOT`. An unconfigured installation does nothing silently.

---

## Repository layout

```
ccflux/
├── ccflux-core/        # Rust binary — plugin-side CLI
├── receiver/           # Rust Axum + SQLite self-hosted collector
├── plugin/             # CC plugin files (distribute to users)
│   ├── .claude-plugin/ # Plugin manifest
│   ├── hooks/          # Hook → script mapping
│   ├── scripts/        # Wrapper scripts (sh + ps1)
│   └── bin/            # Pre-built binaries (populated by CI)
├── dashboard/          # Example SQL queries for IT/admins
├── schema.sql          # SQLite schema
└── .github/workflows/  # Cross-compilation + release workflow
```

---

## Deploying for your organisation

### 1. Deploy the receiver

```bash
cd receiver
cargo build --release

# Minimal — all other values use defaults
DATABASE_PATH=/var/lib/ccflux/ccflux.db \
LISTEN_ADDR=0.0.0.0:8080 \
./target/release/ccflux-receiver
```

The receiver creates the SQLite database and schema on first start. Put it behind a TLS-terminating reverse proxy (nginx, Caddy, etc.) — the binary speaks plain HTTP.

**All env vars with defaults:**

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_PATH` | `ccflux.db` | SQLite file path |
| `LISTEN_ADDR` | `0.0.0.0:8080` | Bind address |
| `ACCESS_TOKEN_EXPIRY_SECS` | `28800` | How long access tokens live (seconds) |
| `REFRESH_TOKEN_ROLLING_DAYS` | `90` | Each token exchange extends the refresh token by this many days |
| `RATE_LIMIT_PER_MINUTE` | `30` | Max `/report` + `/token` + `/register-key` calls per token per minute |
| `BODY_LIMIT_KB` | `64` | Max request body size (KB) |
| `REQUIRE_SIGNATURES` | `false` | Reject `/report` requests that lack a valid Ed25519 signature. Set to `1` or `true` once all devices have registered their keys |

**Endpoints:**

| Endpoint | Auth | Description |
|----------|------|-------------|
| `POST /token` | Refresh token | Exchange refresh token for a short-lived access token |
| `POST /report` | Access token | Ingest a usage payload |
| `POST /register-key` | Access token | Register a device Ed25519 public key |
| `GET /health` | None | DB liveness check — returns `{"status":"ok","db":"ok"}` or 503 |
| `GET /metrics` | None | Prometheus-format counters and gauges. Restrict at the reverse proxy if needed |

### 2. Provision user refresh tokens

Insert a row into `refresh_tokens` for each user:

```sql
INSERT INTO refresh_tokens (token, email, org_id, expires_at)
VALUES (
    'rtok_abc123...',
    'jsmith@example.org',
    'engineering',
    datetime('now', '+365 days')
);
```

Tokens can be any opaque string — generate them with `openssl rand -hex 32`. Each successful use of the token automatically extends its expiry by `REFRESH_TOKEN_ROLLING_DAYS`, so active users never need a new token. Inactive users (no activity for longer than `REFRESH_TOKEN_ROLLING_DAYS`) will need a new token issued by IT.

To revoke a user immediately:

```sql
UPDATE refresh_tokens SET revoked = 1 WHERE email = 'jsmith@example.org';
```

To revoke a specific device (e.g. lost laptop) without revoking the user:

```sql
UPDATE device_keys SET revoked = 1 WHERE public_key = '<base64-public-key>';
```

The device goes silent on its next turn. The user's other devices and their refresh token are unaffected.

### 3. Distribute the plugin

Download a release and give users the `plugin/` directory. Users install it via the Claude Code plugin marketplace or by dropping it into their CC plugins directory.

Users configure the plugin with two values (via `userConfig` if CC supports it, or by creating `~/.claude-work/ccflux/config.json`):

```json
{
  "endpoint": "https://ccflux.yourorg.example/report",
  "token": "their-personal-refresh-token"
}
```

If using CC `userConfig`, the values are set via the plugin settings UI and passed automatically as `CLAUDE_PLUGIN_OPTION_API_ENDPOINT` / `CLAUDE_PLUGIN_OPTION_API_TOKEN`.

The `token` field is the long-lived refresh token issued by IT. The binary automatically exchanges it for a short-lived access token on each invocation (cached in `ccflux/token_cache.json`). Users never need to rotate this manually as long as they use Claude Code within the rolling window.

On first run, the binary also generates a device-specific Ed25519 signing key (`ccflux/signing_key`, readable only by the user) and registers the public key with the receiver. Every subsequent report is signed with this key. This happens silently — no user action required. If reports are generated before the key registers (e.g. network down on first session), they are queued locally and drained automatically once registration succeeds.

### 4. Query usage

See `dashboard/` for ready-made SQL queries:

- `usage_by_user.sql` — total tokens per user, last 30 days
- `five_hour_windows.sql` — usage within 5-hour reset windows (seat pressure indicator)
- `model_breakdown.sql` — token consumption and cache hit rate by model

---

## Building from source

```bash
# Core binary
cd ccflux-core && cargo build --release

# Receiver
cd receiver && cargo build --release

# Cross-compile all targets (requires `cross`)
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu  # Linux ARM
```

Releases are built automatically by `.github/workflows/release.yml` on tag push and attached to the GitHub release. Download a release and drop the binaries into `plugin/bin/` — no build step required for end-users.

---

## Forking for your organisation

1. Fork this repo
2. Deploy `receiver/` to your internal infrastructure
3. Provision refresh tokens in the `refresh_tokens` table
4. Tag a release — CI cross-compiles all binaries and attaches them
5. Distribute install instructions pointing at your fork

The receiver contains no organisation-specific logic. All org customisation lives in the `refresh_tokens` table and your reverse proxy config.

---

## Platform support

| Platform | Binary |
|----------|--------|
| Linux x86_64 | `ccflux-linux-x86_64` |
| Linux aarch64 | `ccflux-linux-aarch64` |
| macOS x86_64 | `ccflux-macos-x86_64` |
| macOS Apple Silicon | `ccflux-macos-aarch64` |
| Windows x86_64 | `ccflux-windows-x86_64.exe` |

WSL is treated as Linux. Native Windows uses the `.ps1` wrapper scripts.

---

## Known limitations

- **SessionEnd unreliability:** CC kills `SessionEnd` hooks before async work completes. The `nohup`/`disown` pattern mitigates this but is not guaranteed. `Stop`-per-turn is the primary mechanism.
- **SIGKILL crashes:** No hooks fire on SIGKILL. At most one in-flight turn is lost per crash.
- **JSONL schema instability:** CC's transcript format is undocumented. If the parser starts returning empty data, check whether field names have changed.
