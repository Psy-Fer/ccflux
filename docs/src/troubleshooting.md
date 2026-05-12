# Troubleshooting

---

## Where to look first

The binary logs all errors to:

```
~/.claude/ccflux/errors.log
```

For a custom CC config dir (e.g. `claude-work`):

```
~/.claude-work/ccflux/errors.log
```

Check this file first. Each line is a timestamped error message. The binary always exits 0 — errors are never shown in your CC session.

---

## Common errors and fixes

### `endpoint must use https://`

```
POST failed: endpoint must use https:// — plain HTTP would expose the bearer token; got: http://...
```

**Cause:** The configured endpoint uses `http://` instead of `https://`.

**Fix:** Update your endpoint to use `https://`. If you're setting up a dev environment and need plain HTTP, set `CCFLUX_ALLOW_HTTP=1` in the wrapper script — but never do this in production.

---

### `refresh token expired or revoked`

```
refresh token expired or revoked — contact your IT admin to issue a new one
```

**Cause:** Your refresh token has been revoked by IT, or it expired due to inactivity (no CC use for longer than `REFRESH_TOKEN_ROLLING_DAYS`, default 90 days).

**Fix:** Contact your IT admin to issue a new refresh token. Update it in your plugin settings or `config.json`.

---

### `ccflux: device key revoked — contact your IT admin to re-provision`

**Cause:** Your device's Ed25519 signing key was revoked by IT (e.g. you reported a lost laptop).

**Fix:** Ask IT to re-provision you. Once they confirm, delete the marker file and regenerate your key:

```bash
rm ~/.claude/ccflux/key_revoked
rm ~/.claude/ccflux/signing_key
rm ~/.claude/ccflux/key_registered  # if present
```

On the next CC session, a new keypair is generated and registered automatically.

---

### `ccflux: request rejected as timestamp-stale (clock skew?)`

**Cause:** The `X-CCFLUX-Timestamp` header value is more than 5 minutes from the server's clock. This means your machine's system clock is significantly off.

**Fix:** Sync your system clock:

```bash
# Linux
sudo timedatectl set-ntp true

# macOS
sudo sntp -sS time.apple.com
```

If clock skew is persistent, check your NTP configuration.

---

### `ccflux: signature-invalid — this is unexpected, retrying next turn`

**Cause:** The Ed25519 signature was rejected despite the key being registered. This is unusual and typically indicates a transient issue.

**Fix:** Usually resolves itself on the next turn. If it persists across many turns, delete and regenerate the signing key:

```bash
rm ~/.claude/ccflux/signing_key
rm ~/.claude/ccflux/key_registered
```

The new key is generated and registered on the next turn.

---

### `HTTP 401` errors

**Cause:** The access token is invalid or expired, or the bearer token in the request is malformed.

**Fix:** Check that your refresh token in plugin settings (or `config.json`) is correct. Delete the token cache to force a fresh exchange:

```bash
rm ~/.claude/ccflux/token_cache.json
```

If the error persists, the refresh token itself may be revoked — contact IT.

---

### No data in the admin dashboard

**Symptom:** You've installed the plugin and completed a few CC turns, but your email doesn't appear in the dashboard.

**Steps to diagnose:**

1. Check `errors.log` for any errors.

2. Confirm the plugin is installed in the right CC config directory. If you use a custom alias, make sure the plugin is in the matching plugins directory.

3. Confirm the endpoint is correct. Test it directly:
   ```bash
   curl https://ccflux.example.org/health
   # expect: {"status":"ok","db":"ok"}
   ```

4. Check whether an offset file was created:
   ```bash
   ls ~/.claude/ccflux/*.offset
   ```
   If no `.offset` file exists, the `SessionStart` hook may not have fired. Check that the plugin's `hooks.json` is present and the scripts are executable:
   ```bash
   ls -la ~/.claude/plugins/ccflux/scripts/
   ```

5. Check the `pending_reports.jsonl` queue:
   ```bash
   wc -l ~/.claude/ccflux/pending_reports.jsonl
   ```
   If this has entries and keeps growing, reports are being queued but not sent. The device key is probably not registered yet — check `errors.log` for registration errors.

---

### `pending_reports.jsonl` growing indefinitely

**Cause:** The device key failed to register (network issue on first session, or the receiver was unreachable).

**What happens:** Reports queue up locally (max 500 entries; oldest are dropped when full). On each successful live report, one queued entry is drained. Once the key registers, the queue drains automatically.

**Fix:**
- Confirm the receiver is reachable: `curl https://ccflux.example.org/health`
- Check `errors.log` for registration errors
- If the key is stuck, try deleting `key_registered` (if it exists) to force a re-registration attempt:
  ```bash
  rm ~/.claude/ccflux/key_registered
  ```

---

### Windows / PowerShell issues

If running natively on Windows (not WSL), the `.ps1` wrapper scripts must be permitted to execute:

```powershell
Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned
```

If the binary produces no errors but no data appears in the dashboard, check whether the `plugin/bin/ccflux-windows-x86_64.exe` binary is present. If it's missing, download the release and copy it into `plugin/bin/`.

---

## Verifying a specific turn was reported

To confirm a specific session's data reached the receiver:

```sql
SELECT * FROM usage_events
WHERE user_email = 'jsmith@example.org'
ORDER BY received_at DESC
LIMIT 10;
```

The `received_at` column is when the receiver stored the event. `timestamp_utc` is when the turn occurred on the user's machine.

---

## Known limitations

### SessionEnd unreliability

The `SessionEnd` hook is killed by Claude Code before asynchronous work can complete. The `nohup`/`disown` pattern in `session_end.sh` mitigates this but is not guaranteed. The `Stop` per-turn hook is the primary reporting path — `SessionEnd` is best-effort for the final turn of a session.

In practice, if a user ends their session abruptly (closes the terminal), the last turn may be reported late or not at all. All previous turns are unaffected.

### SIGKILL crashes

If Claude Code is killed with SIGKILL (e.g. `kill -9`, OOM killer), no hooks fire. At most one in-flight turn is lost. The offset file is not updated, so the next session will re-read from the last successful position — no duplicate reporting.

### JSONL schema instability

Claude Code's transcript format is undocumented. If the parser starts returning `0` tokens for all turns, the `sessionId` or `usage` field names may have changed in a CC update. Check `errors.log` for unexpected-structure warnings, then inspect a recent transcript file:

```bash
# Find a recent session transcript
ls -lt ~/.claude/projects/*/  | head -5

# Check the first assistant entry
grep '"type":"assistant"' ~/.claude/projects/<hash>/<session>.jsonl | head -1 | python3 -m json.tool
```

Compare the field names against what the [CLAUDE.md](../CLAUDE.md) documents as the confirmed schema.
