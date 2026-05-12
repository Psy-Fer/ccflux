# Server & IT Setup

This guide walks through deploying the receiver, provisioning user tokens, and verifying the installation.

---

## Step 1: Download the receiver binary

Download the latest release from [GitHub Releases](https://github.com/Psy-Fer/ccflux/releases):

```bash
# Linux x86_64
wget https://github.com/Psy-Fer/ccflux/releases/latest/download/ccflux-receiver-linux-x86_64
chmod +x ccflux-receiver-linux-x86_64
```

Or build from source:

```bash
cd receiver
cargo build --release
# binary is at target/release/ccflux-receiver
```

---

## Step 2: Configure the receiver

All configuration is via environment variables. Minimum required:

```bash
DATABASE_PATH=/var/lib/ccflux/ccflux.db \
LISTEN_ADDR=127.0.0.1:8080 \
ADMIN_TOKEN="$(openssl rand -hex 32)" \
./ccflux-receiver
```

The receiver creates the SQLite database and schema on first start. On startup it prints all resolved configuration values:

```
ccflux-receiver config:
  DATABASE_PATH              = /var/lib/ccflux/ccflux.db
  LISTEN_ADDR                = 127.0.0.1:8080
  ACCESS_TOKEN_EXPIRY_SECS   = 28800
  REFRESH_TOKEN_ROLLING_DAYS = 90
  RATE_LIMIT_PER_MINUTE      = 30
  BODY_LIMIT_KB              = 64
  REQUIRE_SIGNATURES         = false
  ADMIN_TOKEN                = set
  COOKIE_SECURE              = false
ccflux-receiver listening on 127.0.0.1:8080
```

See [Configuration Reference](./configuration.md) for all available variables.

---

## Step 3: Put it behind a reverse proxy

The receiver speaks plain HTTP. Always put it behind a TLS-terminating reverse proxy in production.

### nginx example

```nginx
server {
    listen 443 ssl;
    server_name ccflux.example.org;

    ssl_certificate     /etc/letsencrypt/live/ccflux.example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/ccflux.example.org/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

### Caddy example

```
ccflux.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy handles certificate provisioning automatically.

---

## Step 4: Run as a systemd service

```ini
# /etc/systemd/system/ccflux-receiver.service
[Unit]
Description=ccflux receiver
After=network.target

[Service]
Type=simple
User=ccflux
WorkingDirectory=/var/lib/ccflux
Environment=DATABASE_PATH=/var/lib/ccflux/ccflux.db
Environment=LISTEN_ADDR=127.0.0.1:8080
Environment=ADMIN_TOKEN=your-strong-admin-token-here
Environment=COOKIE_SECURE=1
Environment=REQUIRE_SIGNATURES=false
ExecStart=/usr/local/bin/ccflux-receiver
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ccflux-receiver
```

---

## Step 5: Verify the deployment

```bash
curl https://ccflux.example.org/health
# {"status":"ok","db":"ok"}
```

If the database is unreachable the response will be `503` with `{"status":"degraded","db":"error"}`.

---

## Step 6: Provision user refresh tokens

Each user needs a personal long-lived refresh token. Insert one row per user:

```bash
sqlite3 /var/lib/ccflux/ccflux.db
```

```sql
INSERT INTO refresh_tokens (token, email, org_id, expires_at)
VALUES (
    'rtok_abc123...',          -- generate with: openssl rand -hex 32
    'jsmith@example.org',
    'engineering',             -- optional group label
    datetime('now', '+365 days')
);
```

Generate tokens securely:

```bash
openssl rand -hex 32
```

**Important:** use one token per person. Sharing tokens prevents per-user attribution.

### Token lifecycle

- Tokens are long-lived (default: 1 year at issuance)
- Each successful use automatically extends expiry by `REFRESH_TOKEN_ROLLING_DAYS` (default 90)
- Active users never need a new token issued — they roll automatically
- Inactive users (no activity for > 90 days) will need a new token from IT

---

## Step 7: Send tokens to users

Give each user:
1. Their personal refresh token
2. Your receiver URL (e.g. `https://ccflux.example.org/report`)
3. A link to the [User Setup](./user-setup.md) guide

Direct them to configure both values in the plugin settings. That's all users need to do.

---

## Revoking access

### Revoke a user immediately

```sql
UPDATE refresh_tokens SET revoked = 1 WHERE email = 'jsmith@example.org';
```

The user's next turn will receive a `401` and the binary will log the error silently. No CC interruption.

### Revoke a specific device (e.g. lost laptop)

Find the device's public key in the admin dashboard under **Device Keys**, then:

```sql
UPDATE device_keys SET revoked = 1 WHERE public_key = '<base64-public-key>';
```

Or use the **Revoke** button in the admin dashboard directly. The device goes silent on its next turn. The user's other devices and their refresh token are unaffected.

### Re-provision a user after revocation

Delete the revoked refresh token and insert a new one:

```sql
DELETE FROM refresh_tokens WHERE email = 'jsmith@example.org' AND revoked = 1;
INSERT INTO refresh_tokens (token, email, expires_at)
VALUES ('rtok_newtoken...', 'jsmith@example.org', datetime('now', '+365 days'));
```

Send the new token to the user. They update it in the plugin settings. The binary will pick it up on the next turn.

---

## Enabling signature enforcement

Once all your users have registered their device keys (visible in the admin dashboard), you can enforce signatures to prevent unsigned requests:

```bash
REQUIRE_SIGNATURES=1
```

Restart the receiver. After this, any binary that hasn't registered a key will have its reports rejected with `403 signature-required`. The binary handles this gracefully by queuing the report locally and retrying registration.

Check the admin dashboard's **Device Keys** table to see which devices have registered before flipping this flag.
