CREATE TABLE IF NOT EXISTS user_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    org_id      TEXT,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    revoked     INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS usage_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    received_at         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    user_email          TEXT NOT NULL,
    user_token          TEXT NOT NULL,
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
