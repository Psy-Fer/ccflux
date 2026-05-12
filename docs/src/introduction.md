# Getting Started

## What is ccflux?

`ccflux` is a Claude Code plugin that collects opt-in, per-turn token usage telemetry from Claude Code sessions and ships it to a self-hosted receiver. It is built for organisations on seat-based Enterprise Claude Code plans where Anthropic's Analytics dashboard does not expose per-user token counts.

### What it enables

- Per-user token consumption visible to IT admins
- Usage mapped against Claude Code's 5-hour reset windows — your primary tool for assessing seat pressure
- Model distribution across the organisation
- Evidence base for moving power users to higher-tier seats

### What it never collects

No message content, prompts, file paths, code, or anything identifying project content. Only usage metadata: token counts, model name, session/turn identifiers, and timestamps.

---

## How it works

A small Rust binary is invoked by Claude Code hook events (`SessionStart`, `Stop`, `SessionEnd`). After each assistant turn, it reads the new usage data from the session transcript, aggregates token counts by model, and POSTs a signed JSON payload to your organisation's receiver. The receiver stores events in SQLite for querying.

```
User's machine                        Your server
──────────────                        ───────────
Claude Code session
  └─ hook fires (Stop)
       └─ ccflux binary
            ├─ reads transcript
            ├─ signs payload (Ed25519)
            └─ POST /report ──────────► receiver
                                          └─ verify signature
                                          └─ INSERT usage_events
```

The receiver exposes an admin dashboard, Prometheus metrics, and a SQLite database you can query directly.

---

## Requirements

### Server

- Linux x86_64 or aarch64
- A TLS-terminating reverse proxy (nginx, Caddy, etc.) — the receiver speaks plain HTTP
- Persistent storage for the SQLite database file (~1 KB per user-turn, very small)

### User machines

- Claude Code installed and configured
- One of: Linux x86_64, Linux aarch64, macOS x86_64, macOS Apple Silicon, Windows x86_64
- Network access to your receiver endpoint over HTTPS

---

## Quick start (for the impatient)

1. **Deploy the receiver** on your server — see [Server & IT Setup](./server-setup.md)
2. **Provision refresh tokens** — one per user, inserted into SQLite
3. **Distribute the plugin** — users drop it into their Claude Code plugins directory and enter the endpoint and token in plugin settings
4. **Query usage** — via the admin dashboard or SQL directly

The full flow from first deploy to first data takes about 15 minutes for IT, plus a minute per user to install the plugin.
