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

## Exporting a static snapshot

Append `?export` to the dashboard URL to download a self-contained HTML file. The export is read-only — all mutating forms (Revoke, Reissue, Add user) are stripped. Everything else works offline: charts render, search filters work, panels expand and collapse.

```bash
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://ccflux.example.org/admin/?export" \
  -o dashboard-snapshot.html
```

The file has no external dependencies and can be shared with stakeholders, attached to reports, or hosted as a static page. See the [Live Demo](./demo.md) for an example.

> **Note:** The export contains user email addresses, device hostnames, and usage data for your whole organisation. No credentials are included — admin token, refresh tokens, and access tokens are all stripped. Treat the file the same way you would treat a spreadsheet export: don't commit it to a public repository or share it beyond the intended recipients.

---

## Summary cards

The top of the dashboard shows org-wide totals across all time:

| Card | Description |
|------|-------------|
| **Users** | Distinct user emails with at least one recorded event |
| **Sessions** | Distinct session IDs |
| **Turns** | Total usage events recorded |
| **Input tokens** | Sum of `input_tokens` across all events |
| **Output tokens** | Sum of `output_tokens` across all events |
| **Cache hit rate** | `cache_read_tokens / (input_tokens + cache_read_tokens + cache_write_tokens)`, expressed as a percentage |

A high cache hit rate (>70%) indicates users are working within long sessions and Sonnet/Opus prompt caching is active. A low rate suggests frequent cold-start sessions or short prompts.

---

## Daily billed tokens chart

A 30-day line chart of daily token consumption (input + output, excluding cache). This is the metric that drives billing for seat-based plans.

Each data point is midnight-to-midnight UTC. Use this chart to spot usage spikes or trends before the billing cycle closes.

---

## Billed tokens by user

A horizontal bar chart of total billed tokens per user over the last 30 days. The primary tool for identifying power users who may need a higher-tier seat.

---

## Billed tokens by model

A horizontal bar chart of total billed tokens grouped by model (e.g. `claude-sonnet-4-6`, `claude-opus-4-7`). Use this to understand your model distribution and estimate cost.

---

## Usage by user table

A table with one row per active user (last 30 days):

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
| **Tier** | Inferred seat tier — see [Tier classification](#tier-classification) below |

---

## Model breakdown table

Token consumption and cache hit rate per model across all time:

| Column | Description |
|--------|-------------|
| **Model** | Model identifier |
| **Users** | Distinct users who have used this model |
| **Turns** | Number of usage events |
| **Input tokens** | Sum of `input_tokens` |
| **Output tokens** | Sum of `output_tokens` |
| **Cache reads** | Sum of `cache_read_tokens` |
| **Cache writes** | Sum of `cache_write_tokens` |
| **Cache hit %** | `cache_read / (input + cache_read + cache_write)` |

---

## 5-hour billing windows

The 5-hour window panel shows usage bucketed into Claude Code's rolling 5-hour billing reset windows. This is the key indicator for seat pressure.

**Peak window bar chart** — maximum token consumption in any single 5-hour window per user, with the average window size shown alongside the peak. Users with peaks approaching their seat limit are candidates for upgrades.

**Window detail table** — per-window breakdown showing start time, end time, status (open/closed), total tokens, turn count, and contributing session count.

---

## Tier classification

Each user's row in the usage table includes a **Tier** badge — an automated estimate of which Claude Code seat tier that user is on.

Tiers are inferred from the distribution of completed 5-hour billing window peaks across the organisation. Users whose peak windows cluster together get the same tier label. The algorithm uses a 1.8× gap ratio to split tiers: if one group's peaks are consistently 1.8× higher than another group's, they are classified as a different tier.

**Confidence levels:**

| Badge colour | Confidence | Meaning |
|---|---|---|
| Green | **High** | Confirmed via a 429 rate-limit event (exact tier known) |
| Blue | **Medium** | Inferred from 10+ completed windows |
| Yellow | **Low** | Inferred from 3–9 completed windows |
| Grey | **Unknown** | Fewer than 3 completed windows — not enough data |

Tier inference runs every `TIER_INFERENCE_INTERVAL_SECS` (default 600 seconds) in the background. Labels are persisted across restarts.

> **Note:** Tier labels are estimates. They reflect usage patterns, not Anthropic's internal account configuration. A user on a Max 5× seat who has never approached their limit may show as a lower tier until enough window data accumulates.

---

## Device keys table

Lists all registered Ed25519 device keys:

| Column | Description |
|--------|-------------|
| **User** | Email of the user who registered the key |
| **Device ID** | Hostname reported at registration time |
| **Registered** | When the key was first registered |
| **Last seen** | Most recent signed report from this device |
| **Status** | Active or Revoked |
| **Action** | **Revoke** button |

### Revoking a device

Click **Revoke** next to a device to revoke it immediately. The next report from that device receives a `403 key-revoked` response. The binary logs the error, clears its local pending queue, and goes silent until re-provisioned.

To re-provision a revoked device, the user deletes two files and restarts a CC session:

```bash
rm ~/.claude/ccflux/signing_key
rm ~/.claude/ccflux/key_revoked
rm ~/.claude/ccflux/key_registered  # if present
```

A new keypair is generated and registered automatically on the next turn.

Revoking a device does **not** revoke the user's refresh token — their other devices continue reporting normally.

---

## Recent events table

The last 50 usage events across all users, showing received timestamp, user, device, session ID, turn index, model, and token counts. Useful for confirming that a newly installed plugin is reporting correctly.

---

## User provisioning

The **User provisioning** panel is the primary interface for managing refresh tokens. It is visible at the bottom of the dashboard.

### Adding a user

Fill in the form at the top of the panel:

| Field | Description |
|---|---|
| **Email** | The user's email address (must match their Claude Code account) |
| **Division** | Optional organisational label (team, department) — for your records only |
| **Days valid** | How many days before the token expires from the last use (default 365; rolling — resets on each use) |

Click **Add user**. The next page shows the generated refresh token alongside the endpoint URL, ready to copy and send to the user.

> The token is only shown once. If you lose it before sending it to the user, use **Reissue** to replace it.

### Revoking a user

Click **Revoke** next to an active token to revoke it immediately. The user's binary will receive a `401` response on the next token exchange and stop reporting. Use this when a user leaves the organisation or their token is compromised.

### Reissuing a token

Click **Reissue** to atomically revoke the old token and generate a new one. The new token page shows the replacement token ready to copy. Use this for periodic rotation or when a user reports their token was exposed.

Reissuing preserves the user's usage history — all past events remain associated with their email address.

---

## All timestamps

All timestamps in the dashboard are displayed in your browser's local timezone via inline JavaScript. Raw values in the database are stored as UTC.
