# Admin Dashboard

The admin dashboard is served at `/admin/` on your receiver. It is disabled by default and enabled by setting the `ADMIN_TOKEN` environment variable to a non-empty string.

---

## Accessing the dashboard

Navigate to `https://ccflux.example.org/admin/` in your browser.

You will see a login form. Enter the value of your `ADMIN_TOKEN` environment variable. On success, an `HttpOnly; SameSite=Strict` session cookie is set — you stay logged in until the cookie expires or you clear it.

You can also authenticate via bearer token for programmatic access:

```bash
curl -H "Authorization: Bearer $ADMIN_TOKEN" https://ccflux.example.org/admin/
```

---

## Summary cards

The top of the dashboard shows org-wide totals for the last 30 days:

| Card | Description |
|------|-------------|
| **Users** | Distinct user emails that have sent at least one report |
| **Sessions** | Distinct session IDs |
| **Turns** | Total usage events recorded |
| **Input tokens** | Sum of `input_tokens` across all events |
| **Output tokens** | Sum of `output_tokens` across all events |
| **Cache hit rate** | `cache_read_tokens / (input_tokens + cache_read_tokens)`, expressed as a percentage |

A high cache hit rate (>70%) indicates users are working within long sessions and Sonnet/Opus caching is active. A low rate suggests frequent cold-start sessions.

---

## Daily billed tokens chart

A 30-day line chart of daily token consumption (input + output, excluding cache reads). This is the metric that drives billing for seat-based plans.

Each data point is midnight-to-midnight UTC. Use this chart to spot usage spikes or trends before the billing cycle closes.

---

## Billed tokens by user

A horizontal bar chart of total billed tokens per user over the last 30 days. This is the primary tool for identifying power users who may need a higher-tier seat.

---

## Billed tokens by model

A horizontal bar chart of total billed tokens grouped by model (e.g. `claude-sonnet-4-6`, `claude-opus-4-5`). Use this to understand your model distribution and plan cost.

---

## Usage by user table

A sortable table with one row per user:

| Column | Description |
|--------|-------------|
| **User** | Email address |
| **Input tokens** | Sum of `input_tokens` |
| **Output tokens** | Sum of `output_tokens` |
| **Cache reads** | Sum of `cache_read_tokens` |
| **Cache writes** | Sum of `cache_write_tokens` |
| **Sessions** | Distinct session count |
| **Turns** | Total turns |
| **Last active** | Most recent event timestamp |

---

## Model breakdown table

Token consumption and cache hit rate per model:

| Column | Description |
|--------|-------------|
| **Model** | Model identifier |
| **Turns** | Number of usage events |
| **Input tokens** | Sum of `input_tokens` |
| **Output tokens** | Sum of `output_tokens` |
| **Cache hit %** | `cache_read / (input + cache_read)` |

---

## 5-hour billing windows

The 5-hour window panel shows usage bucketed into Claude Code's rolling 5-hour billing reset windows. This is the key indicator for seat pressure.

**Peak window** — the bar chart shows the maximum token consumption in any single 5-hour window per user. Users with peak windows approaching their seat limits are candidates for seat upgrades.

**Active window badge** — a live badge shows whether a user currently has an open window (a session active within the last 5 hours). This helps identify users who are in an ongoing heavy session.

**Window detail table** — the per-window breakdown table shows each window's start time, end time, status (open/closed), total tokens, turn count, and how many distinct sessions contributed.

---

## Device keys table

Lists all registered Ed25519 device keys with:

| Column | Description |
|--------|-------------|
| **User** | Email of the user who registered the key |
| **Device ID** | Hostname reported at registration time (informational) |
| **Registered** | When the key was first registered |
| **Last seen** | Most recent report signed with this key |
| **Status** | Active or Revoked |
| **Action** | **Revoke** button |

### Revoking a device

Click **Revoke** next to a device to revoke it immediately. The next report signed by that device will receive a `403 key-revoked` response. The binary logs the error, clears its local queue, and goes silent until re-provisioned.

To re-provision a device: the user deletes `~/.claude/ccflux/signing_key` and `~/.claude/ccflux/key_registered` (if present), then restarts a CC session. A new keypair is generated and registered automatically on the next turn.

Revoking a device does **not** revoke the user's refresh token — their other devices continue reporting normally.

---

## Recent events table

The last 50 usage events across all users, showing session ID, turn index, model, token counts, and received timestamp. Useful for confirming that a newly installed plugin is reporting correctly.

---

## All timestamps

All timestamps in the dashboard are displayed in your browser's local timezone via inline JavaScript. Raw values in the database are stored as UTC.
