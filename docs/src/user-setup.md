# User Setup

This guide is for Claude Code users whose IT team has deployed ccflux. You will need two things from IT before you start:

- Your **receiver endpoint** URL (e.g. `https://ccflux.example.org/report`)
- Your personal **refresh token** (a long hex string like `rtok_abc123...`)

---

## Step 1: Install the plugin

Download the latest release from [GitHub Releases](https://github.com/Psy-Fer/ccflux/releases) and extract it. You will find a `plugin/` directory inside.

### Option A: Claude Code plugin marketplace

If your organisation's fork is registered in the CC plugin marketplace, search for **ccflux** in the Claude Code plugin settings and install from there.

### Option B: Manual install

Copy the `plugin/` directory into your Claude Code plugins directory:

```bash
# Default CC installation
cp -r plugin/ ~/.claude/plugins/ccflux/

# If you use a custom CC config dir (e.g. alias claude-work)
cp -r plugin/ ~/.claude-work/plugins/ccflux/
```

On Windows (PowerShell):

```powershell
Copy-Item -Recurse plugin\ "$env:APPDATA\Claude\plugins\ccflux\"
```

---

## Step 2: Configure the endpoint and token

Open Claude Code and navigate to **Settings → Plugins → ccflux**.

Fill in:

| Field | Value |
|-------|-------|
| **Receiver endpoint** | The URL your IT team gave you, e.g. `https://ccflux.example.org/report` |
| **API token** | Your personal refresh token |

The token is stored in your system keychain (or `~/.claude/.credentials.json` on systems without a keychain). It is never written to disk in plaintext.

### Alternative: config file

If plugin settings aren't available in your CC version, create a config file instead:

```bash
mkdir -p ~/.claude/ccflux
cat > ~/.claude/ccflux/config.json << 'EOF'
{
  "endpoint": "https://ccflux.example.org/report",
  "token": "rtok_abc123..."
}
EOF
chmod 600 ~/.claude/ccflux/config.json
```

For a custom CC config dir:

```bash
mkdir -p ~/.claude-work/ccflux
cat > ~/.claude-work/ccflux/config.json << 'EOF'
{
  "endpoint": "https://ccflux.example.org/report",
  "token": "rtok_abc123..."
}
EOF
chmod 600 ~/.claude-work/ccflux/config.json
```

---

## Step 3: Verify the first report

Start a new Claude Code session and complete a turn (send a message and get a response). After the turn, the plugin should have:

1. Generated a device signing key (`~/.claude/ccflux/signing_key`, readable only by you)
2. Registered the public key with the receiver
3. Sent the first usage report

To confirm data is flowing, ask your IT admin to check the admin dashboard for your email address. The first report typically appears within a few seconds of a turn completing.

### Check for errors locally

If something is misconfigured, errors are logged silently to:

```
~/.claude/ccflux/errors.log
```

Common entries and what they mean are covered in the [Troubleshooting](./troubleshooting.md) guide.

---

## Multiple Claude Code aliases

If you use multiple CC instances (e.g. `claude` for personal, `claude-work` for work), install the plugin in each one separately. Each instance only reports for sessions that use its own config directory — they don't interfere with each other.

An unconfigured installation (no endpoint/token set) does nothing silently. You can safely have the plugin installed in a CC instance without configuring it.

---

## Opting out

To stop reporting:

- Remove the endpoint and token from plugin settings (or delete `config.json`)
- Or uninstall the plugin entirely

The binary exits silently with no reporting when no endpoint is configured. Your existing data in the receiver is not affected.

---

## Privacy notes

The plugin collects **only** token counts, model name, session/turn identifiers, and timestamps. It reads your email address from `~/.claude/.claude.json` (the file CC maintains for your logged-in account) — you never need to enter it manually.

No message content, prompts, file names, code, or project paths are ever collected or transmitted.
