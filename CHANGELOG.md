# Changelog

All notable changes to this project will be documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-05-14

### Added

**Plugin binary (`ccflux-core`)**
- Per-turn token usage collection via Claude Code `Stop`, `SessionStart`, and `SessionEnd` hooks
- Parses `usage` fields from the CC JSONL transcript; aggregates all new entries since the last offset into a single payload per turn
- Atomic offset tracking per session (`<data_dir>/ccflux/<session_id>.offset`) so concurrent sessions never collide
- Two-token auth: long-lived refresh token → short-lived access token, cached in `token_cache.json` (0600), auto-refreshed within 5 minutes of expiry
- Per-device Ed25519 signing: keypair generated on first `SessionStart`, private key stored at `signing_key` (0600), public key registered with the receiver via `POST /register-key`
- Local queue (`pending_reports.jsonl`, cap 500) for payloads generated before the device key is registered; drained one entry per successful live report
- Key revocation handling: on `key-revoked` 403, logs to `errors.log`, clears the queue, and goes silent until re-provisioned
- Endpoint and token resolved from `CLAUDE_PLUGIN_OPTION_*` env vars or `<data_dir>/ccflux/config.json`; silently exits 0 if neither is set
- Data-dir safety check: ignores transcripts that belong to a different CC installation (prevents a plugin in `~/.claude-work/` from reporting on `~/.claude/` sessions)
- User email auto-read from `<data_dir>/.claude.json` — never a user-configurable field
- All errors logged to `errors.log`; binary always exits 0 so CC sessions are never interrupted

**Receiver (`ccflux-receiver`)**
- `POST /token` — exchanges a refresh token for a short-lived access token; rolling expiry extends the refresh token on every use
- `POST /register-key` — registers a device Ed25519 public key; idempotent (re-registration updates `last_seen_at`)
- `POST /report` — ingests a usage payload; verifies Ed25519 signature before deserialising; `INSERT OR IGNORE` for idempotent retries
- `GET /health` — DB ping, no auth required
- `GET /metrics` — Prometheus text format; counters for accepted/rejected/rate-limited reports, token exchanges, key registrations; live gauge for active access tokens
- Signature enforcement: `REQUIRE_SIGNATURES=1` rejects unsigned requests; defaults to permissive for gradual rollout
- Per-token rate limiting (default 30 req/min, configurable via `RATE_LIMIT_PER_MINUTE`)
- Request body size limit (default 64 KB, configurable via `BODY_LIMIT_KB`)
- Replay protection: `X-CCFLUX-Timestamp` header must be within 5 minutes
- Hourly background purge of expired access tokens
- All configuration via environment variables; startup prints resolved config

**Admin dashboard** (served at `/admin/`, enabled by `ADMIN_TOKEN` env var)
- Login form with `HttpOnly; SameSite=Strict` cookie; optional `; Secure` flag via `COOKIE_SECURE=1`
- `GET /admin/logout` — clears session cookie and redirects to login
- Org summary cards: users, sessions, turns, input tokens, output tokens, cache hit rate
- SVG line chart of daily billed tokens over the last 30 days (server-rendered, no JS dependencies)
- SVG horizontal bar charts: billed tokens by user and by model
- Usage-by-user table (input, output, cache reads, cache writes, sessions, turns, last active) with inferred tier column
- Model breakdown table with cache hit percentage
- Device key management table with one-click revoke
- Recent events table (last 50 turns)
- 5-hour billing window charts and table (bar chart of peak billed tokens per user, detailed window table for the last 7 days)
- All timestamps localised to the browser's timezone via inline JS
- Collapsible panels — click any panel header to collapse/expand; state persisted in `localStorage`
- Auto-refresh — configurable interval via number input (5 s – 3600 s), live countdown ("next in Xs"), on/off state and interval persisted in `localStorage`

**Tier inference**
- Per-user billing tier classification (`Unknown` / `Tier 1` / `Tier 2` / …) inferred from 5-hour window history
- Algorithm: per user, takes the 75th-percentile of completed window peaks (drops inactivity-reset outliers); gap-based clustering (1.8× ratio) assigns tier boundaries across the org
- Requires ≥ 3 completed windows to classify; confidence badge (low / medium) shown in the user table
- `tier_hints` table persists classifications across restarts; rows with `method = 'limit_hit'` are never overwritten by inference (reserved for a future exact signal from confirmed 429 events)
- Recomputed in background every `TIER_INFERENCE_INTERVAL_SECS` seconds (default 600); cache seeded from DB on startup so dashboard is populated immediately after restart
- Lookback window configurable via `TIER_INFERENCE_DAYS` (default 90) — avoids scanning unbounded history on large deployments

**Plugin wrappers**
- Bash scripts for Linux/macOS/WSL/Git Bash; PowerShell scripts for native Windows
- Platform/arch detection in `.sh` scripts covers Linux, macOS, WSL, and Git Bash (MSYS/MINGW/Cygwin → Windows binary)
- `hooks-windows.json` variant ships alongside `hooks.json` for native PowerShell installs; uses `.ps1` wrappers via `powershell -ExecutionPolicy Bypass -File`
- `session_end.sh` uses `nohup`/`disown` to survive the CC hook timeout

**CI**
- `ci.yml`: fmt check, clippy (`-D warnings`), and `cargo test` for both crates on every push and PR
- `release.yml`: cross-platform release builds for all five plugin targets and both Linux receiver targets; creates a GitHub Release with all binaries attached

### Security
- Constant-time token comparison using `subtle::ConstantTimeEq` throughout the receiver
- HTML output in admin dashboard fully escaped via `esc()` helper
- All SQL uses parameterized `sqlx` queries — no string interpolation
- Registration body built with `serde_json::json!` macro (no manual JSON formatting)
- Server-supplied `x-ccflux-error` header values sanitised (ASCII printable, max 64 chars) before logging
