# ccflux

A Claude Code plugin that collects opt-in, per-turn token usage telemetry and ships it to a self-hosted receiver. Built for organisations on seat-based Enterprise Claude Code plans where Anthropic's Analytics dashboard does not expose per-user token counts.

**What it collects:** token counts, model name, session/turn identifiers, timestamps.  
**What it never collects:** message content, prompts, file paths, code, or anything identifying project content.

---

## Documentation

**[Full docs →](https://psy-fer.github.io/ccflux)**

- [Getting started](https://psy-fer.github.io/ccflux/introduction.html) — overview and requirements
- [Server & IT setup](https://psy-fer.github.io/ccflux/server-setup.html) — deploy the receiver, provision tokens
- [User setup](https://psy-fer.github.io/ccflux/user-setup.html) — install the plugin, configure endpoint and token
- [Admin dashboard](https://psy-fer.github.io/ccflux/admin-dashboard.html) — usage charts, device key management
- [Security guide](https://psy-fer.github.io/ccflux/security.html) — production hardening checklist
- [Configuration reference](https://psy-fer.github.io/ccflux/configuration.html) — all env vars, state files, error codes
- [Troubleshooting](https://psy-fer.github.io/ccflux/troubleshooting.html) — `errors.log` interpretation, common issues

---

## Quick start

**IT / server setup**
1. Deploy `receiver/` behind a TLS proxy — see [Server & IT setup](https://psy-fer.github.io/ccflux/server-setup.html)
2. Insert a refresh token per user into the `refresh_tokens` table
3. Distribute the receiver URL and each user's token

**User install**

Linux / macOS / WSL / Git Bash:
```bash
bash install.sh
```

Native Windows PowerShell:
```powershell
.\install.ps1
# or with -UseStandardHooks if CC runs via Git Bash rather than native PS
```

Both scripts find all Claude Code data directories on the machine (including aliased ones), let you pick the install target, copy all plugin files, and register the plugin in CC's plugin registry.

To uninstall:
```bash
bash uninstall.sh
```
```powershell
.\uninstall.ps1
```

See [User setup](https://psy-fer.github.io/ccflux/user-setup.html) for full instructions.

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
├── docs/               # mdbook documentation source
├── dashboard/          # Example SQL queries
└── schema.sql          # SQLite schema
```

---

## Building from source

```bash
cd ccflux-core && cargo build --release
cd receiver && cargo build --release
```

Releases are built automatically by `.github/workflows/release.yml` on tag push. Download a release and drop the binaries into `plugin/bin/` — no build step required for end-users.

---

## Platform support

| Platform | Binary | Hook config |
|----------|--------|-------------|
| Linux x86_64 | `ccflux-linux-x86_64` | `hooks.json` |
| Linux aarch64 | `ccflux-linux-aarch64` | `hooks.json` |
| macOS x86_64 | `ccflux-macos-x86_64` | `hooks.json` |
| macOS Apple Silicon | `ccflux-macos-aarch64` | `hooks.json` |
| Windows via WSL | `ccflux-linux-x86_64` | `hooks.json` |
| Windows via Git Bash | `ccflux-windows-x86_64.exe` | `hooks.json` |
| Windows native PowerShell | `ccflux-windows-x86_64.exe` | `hooks-windows.json` |

**Windows notes:**
- **WSL** (recommended): CC runs inside WSL and presents as Linux — no special configuration needed.
- **Git Bash**: `hooks.json` works as-is; the `.sh` scripts detect MSYS/MINGW and select the Windows binary automatically.
- **Native PowerShell**: copy `plugin/hooks/hooks-windows.json` over `hooks.json` before installing. The `.ps1` wrappers are used instead of `.sh`.

---

## Forking for your organisation

1. Fork this repo
2. Deploy `receiver/` to your internal infrastructure
3. Provision refresh tokens and distribute install instructions pointing at your fork
4. Tag a release — CI cross-compiles all binaries and attaches them
