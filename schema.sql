-- Legacy static tokens (kept for reference; new deployments use refresh_tokens).
CREATE TABLE IF NOT EXISTS user_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    division    TEXT,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    revoked     INTEGER DEFAULT 0
);

-- IT-issued long-lived tokens. Users configure these once in config.json.
-- Never sent to /report; exchanged for short-lived access tokens via /token.
CREATE TABLE IF NOT EXISTS refresh_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    division    TEXT,
    expires_at  TIMESTAMP NOT NULL,
    revoked     INTEGER DEFAULT 0,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Short-lived access tokens issued by the /token endpoint.
-- These are what the plugin sends to /report.
CREATE TABLE IF NOT EXISTS access_tokens (
    token           TEXT PRIMARY KEY,
    refresh_token   TEXT NOT NULL REFERENCES refresh_tokens(token),
    email           TEXT NOT NULL,
    expires_at      TIMESTAMP NOT NULL,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_access_refresh ON access_tokens(refresh_token);
CREATE INDEX IF NOT EXISTS idx_access_expires  ON access_tokens(expires_at);

-- Per-device Ed25519 public keys. One row per device per user.
-- Revoke a device: UPDATE device_keys SET revoked = 1 WHERE public_key = '...';
CREATE TABLE IF NOT EXISTS device_keys (
    public_key      TEXT PRIMARY KEY,       -- base64-encoded 32-byte Ed25519 public key
    email           TEXT NOT NULL,
    device_id       TEXT,                   -- hostname at registration time (informational)
    registered_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_seen_at    TIMESTAMP,
    revoked         INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_device_keys_email ON device_keys(email);

CREATE TABLE IF NOT EXISTS usage_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    received_at         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    user_email          TEXT NOT NULL,
    user_token          TEXT NOT NULL,   -- refresh_token for stable audit trail
    device_id           TEXT NOT NULL DEFAULT '',
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

CREATE INDEX IF NOT EXISTS idx_usage_user     ON usage_events(user_email);
CREATE INDEX IF NOT EXISTS idx_usage_session  ON usage_events(session_id);
CREATE INDEX IF NOT EXISTS idx_usage_time     ON usage_events(timestamp_utc);

-- Inferred or confirmed tier classification per user.
-- method: 'inferred' (from usage pattern) | 'limit_hit' (confirmed from 429 event — future).
-- Rows where method = 'limit_hit' are never overwritten by the inference pass.
CREATE TABLE IF NOT EXISTS tier_hints (
    email           TEXT PRIMARY KEY,
    tier_label      TEXT NOT NULL DEFAULT 'unknown',
    peak_tokens     INTEGER,
    method          TEXT NOT NULL DEFAULT 'inferred',
    window_count    INTEGER NOT NULL DEFAULT 0,
    updated_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
