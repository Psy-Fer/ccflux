#!/usr/bin/env python3
"""
Generate a realistic docs/demo SQLite database for ccflux.
Creates 15 users with varied tiers, 45 days of usage history.

Usage:
    python3 seed_docs_db.py [output_path]
    Default output: docs_demo.db
"""

import sqlite3
import random
import hashlib
import secrets
import sys
from datetime import datetime, timedelta, timezone

DB_PATH = sys.argv[1] if len(sys.argv) > 1 else "docs_demo.db"

SCHEMA = """
CREATE TABLE IF NOT EXISTS user_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    division    TEXT,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    revoked     INTEGER DEFAULT 0
);
CREATE TABLE IF NOT EXISTS refresh_tokens (
    token       TEXT PRIMARY KEY,
    email       TEXT NOT NULL,
    division    TEXT,
    expires_at  TIMESTAMP NOT NULL,
    revoked     INTEGER DEFAULT 0,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS access_tokens (
    token           TEXT PRIMARY KEY,
    refresh_token   TEXT NOT NULL REFERENCES refresh_tokens(token),
    email           TEXT NOT NULL,
    expires_at      TIMESTAMP NOT NULL,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_access_refresh ON access_tokens(refresh_token);
CREATE INDEX IF NOT EXISTS idx_access_expires  ON access_tokens(expires_at);
CREATE TABLE IF NOT EXISTS device_keys (
    public_key      TEXT PRIMARY KEY,
    email           TEXT NOT NULL,
    device_id       TEXT,
    registered_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_seen_at    TIMESTAMP,
    revoked         INTEGER DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_device_keys_email ON device_keys(email);
CREATE TABLE IF NOT EXISTS usage_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    received_at         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    user_email          TEXT NOT NULL,
    user_token          TEXT NOT NULL,
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
CREATE TABLE IF NOT EXISTS tier_hints (
    email           TEXT PRIMARY KEY,
    tier_label      TEXT NOT NULL DEFAULT 'unknown',
    peak_tokens     INTEGER,
    method          TEXT NOT NULL DEFAULT 'inferred',
    window_count    INTEGER NOT NULL DEFAULT 0,
    updated_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
"""

# ---------------------------------------------------------------------------
# User definitions
# ---------------------------------------------------------------------------

USERS = [
    # (email, division, tier, devices, activity_profile)
    # tier: "max20" ~390k/5h window, "max5" ~97k, "standard" ~19.5k, "light" ~5k
    ("alex.chen@example.com",        "Engineering",    "max20",    2, "heavy"),
    ("sarah.thompson@example.com",   "Engineering",    "max20",    2, "heavy"),
    ("james.wilson@example.com",     "Data Science",   "max20",    1, "heavy"),
    ("emily.rodriguez@example.com",  "Engineering",    "max5",     2, "moderate_high"),
    ("michael.park@example.com",     "Engineering",    "max5",     1, "moderate_high"),
    ("lisa.anderson@example.com",    "Research",       "max5",     2, "moderate_high"),
    ("david.kim@example.com",        "Engineering",    "max5",     1, "moderate"),
    ("rachel.nguyen@example.com",    "Research",       "max5",     2, "moderate"),
    ("thomas.brown@example.com",     "IT",             "standard", 1, "moderate"),
    ("jennifer.lee@example.com",     "Engineering",    "standard", 1, "moderate_low"),
    ("chris.martin@example.com",     "Research",       "standard", 1, "moderate_low"),
    ("amanda.white@example.com",     "Data Science",   "standard", 2, "moderate_low"),
    ("kevin.taylor@example.com",     "Engineering",    "standard", 1, "moderate_low"),
    ("stephanie.garcia@example.com", "Research",       "light",    1, "light"),
    ("brian.jones@example.com",      "IT",             "light",    1, "light"),
]

# Session count per day by profile (weekday, weekend)
SESSIONS_PER_DAY = {
    "heavy":        (4, 2),
    "moderate_high":(3, 1),
    "moderate":     (2, 1),
    "moderate_low": (2, 0),
    "light":        (1, 0),
}

# Turns per session by profile
TURNS_PER_SESSION = {
    "heavy":         (8, 25),
    "moderate_high": (5, 18),
    "moderate":      (4, 14),
    "moderate_low":  (3, 10),
    "light":         (2, 7),
}

# Model mix by tier: list of (model, weight)
MODEL_WEIGHTS = {
    "max20": [
        ("claude-opus-4-7",         30),
        ("claude-sonnet-4-6",       55),
        ("claude-haiku-4-5-20251001", 15),
    ],
    "max5": [
        ("claude-opus-4-7",         10),
        ("claude-sonnet-4-6",       70),
        ("claude-haiku-4-5-20251001", 20),
    ],
    "standard": [
        ("claude-sonnet-4-6",       75),
        ("claude-haiku-4-5-20251001", 25),
    ],
    "light": [
        ("claude-sonnet-4-6",       60),
        ("claude-haiku-4-5-20251001", 40),
    ],
}

# Base token ranges (input, output, cache_write) per model, per turn
# cache_read grows after the first few turns in a session
TOKEN_RANGES = {
    "claude-opus-4-7": {
        "input":       (4000, 18000),
        "output":      (400,  2200),
        "cache_write": (2000, 8000),
    },
    "claude-sonnet-4-6": {
        "input":       (2000, 12000),
        "output":      (150,  1500),
        "cache_write": (1000, 6000),
    },
    "claude-haiku-4-5-20251001": {
        "input":       (800,  5000),
        "output":      (80,   600),
        "cache_write": (400,  2000),
    },
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

rng = random.Random(42)  # deterministic

def rand_token(n=32):
    return secrets.token_hex(n)

def fake_pubkey(email, device_idx):
    h = hashlib.sha256(f"{email}:{device_idx}".encode()).digest()
    return h.hex()[:64]

def fmt(dt):
    return dt.strftime("%Y-%m-%d %H:%M:%S")

def weighted_choice(pairs):
    items, weights = zip(*pairs)
    return rng.choices(items, weights=weights, k=1)[0]

def session_start_hour(profile):
    """Return a realistic start-of-session hour (UTC offset for Sydney ~+10)."""
    if profile in ("heavy", "moderate_high"):
        return rng.choices(range(22, 30), weights=[2,3,4,5,5,4,3,2], k=1)[0] % 24
    elif profile in ("moderate", "moderate_low"):
        return rng.choices(range(22, 29), weights=[2,3,5,5,4,3,2], k=1)[0] % 24
    else:
        return rng.choices(range(23, 28), weights=[3,5,5,3,2], k=1)[0] % 24

# ---------------------------------------------------------------------------
# Main generation
# ---------------------------------------------------------------------------

NOW = datetime.now(timezone.utc).replace(microsecond=0)
START = NOW - timedelta(days=45)

conn = sqlite3.connect(DB_PATH)
conn.executescript(SCHEMA)
cur = conn.cursor()

all_events = []

for email, division, tier, num_devices, profile in USERS:
    # --- refresh token ---
    refresh_tok = rand_token()
    expires = NOW + timedelta(days=365)
    cur.execute(
        "INSERT INTO refresh_tokens(token,email,division,expires_at,revoked,created_at) VALUES(?,?,?,?,0,?)",
        (refresh_tok, email, division, fmt(expires), fmt(START - timedelta(days=5)))
    )

    # --- access token (current, valid) ---
    access_tok = rand_token()
    cur.execute(
        "INSERT INTO access_tokens(token,refresh_token,email,expires_at,created_at) VALUES(?,?,?,?,?)",
        (access_tok, refresh_tok, email, fmt(NOW + timedelta(hours=4)), fmt(NOW - timedelta(hours=4)))
    )

    # --- device keys ---
    devices = []
    hostnames = [
        f"{email.split('.')[0]}-laptop",
        f"{email.split('.')[0]}-desktop",
    ]
    for i in range(num_devices):
        pk = fake_pubkey(email, i)
        reg_at = START + timedelta(days=rng.randint(0, 2))
        devices.append((pk, hostnames[i]))
        cur.execute(
            "INSERT INTO device_keys(public_key,email,device_id,registered_at,last_seen_at,revoked) VALUES(?,?,?,?,?,0)",
            (pk, email, hostnames[i], fmt(reg_at), fmt(NOW - timedelta(hours=rng.randint(0, 48))))
        )

    # --- usage events ---
    day = START.replace(hour=0, minute=0, second=0)
    while day < NOW:
        is_weekend = day.weekday() >= 5
        sessions_today = SESSIONS_PER_DAY[profile][1 if is_weekend else 0]
        # add some jitter — occasionally skip a day or do an extra session
        if rng.random() < 0.15:
            sessions_today = max(0, sessions_today - 1)
        elif rng.random() < 0.1:
            sessions_today += 1

        used_hours = set()
        for _ in range(sessions_today):
            # pick a start hour that doesn't collide with another session
            attempts = 0
            h = session_start_hour(profile)
            while h in used_hours and attempts < 10:
                h = session_start_hour(profile)
                attempts += 1
            used_hours.add(h)
            used_hours.add((h + 1) % 24)

            session_start = day.replace(hour=h, minute=rng.randint(0, 59), second=rng.randint(0, 59))
            if session_start >= NOW:
                continue

            session_id = secrets.token_hex(16)
            device_pk, device_id = rng.choice(devices)
            num_turns = rng.randint(*TURNS_PER_SESSION[profile])
            model_pool = MODEL_WEIGHTS[tier]

            accumulated_cache = 0  # grows through session

            for turn_idx in range(num_turns):
                ts = session_start + timedelta(minutes=turn_idx * rng.randint(2, 8))
                if ts >= NOW:
                    break

                model = weighted_choice(model_pool)
                ranges = TOKEN_RANGES[model]

                input_tok = rng.randint(*ranges["input"])
                output_tok = rng.randint(*ranges["output"])

                # cache write: higher on early turns, drops off
                if turn_idx < 3:
                    cw = rng.randint(*ranges["cache_write"])
                    accumulated_cache += cw
                else:
                    cw = rng.randint(0, ranges["cache_write"][0] // 2)

                # cache read: grows as context accumulates, heavy users hit it more
                cache_read_ratio = {
                    "max20": 0.75, "max5": 0.60, "standard": 0.40, "light": 0.20
                }[tier]
                if turn_idx > 0 and accumulated_cache > 0:
                    cr = int(accumulated_cache * cache_read_ratio * rng.uniform(0.5, 1.0))
                    # decay over time (cache TTL / context rollover)
                    cr = max(0, cr - rng.randint(0, 500))
                else:
                    cr = 0

                # Apply tier scaling to get realistic per-window totals
                scale = {"max20": 4.5, "max5": 1.8, "standard": 1.0, "light": 0.4}[tier]
                input_tok = int(input_tok * scale)
                output_tok = int(output_tok * scale)

                all_events.append((
                    fmt(ts),          # received_at
                    email,
                    refresh_tok,
                    device_id,
                    session_id,
                    turn_idx,
                    fmt(ts),          # timestamp_utc
                    fmt(session_start),
                    model,
                    input_tok,
                    output_tok,
                    cr,
                    cw,
                    "0.1.0",
                    1,
                ))

        day += timedelta(days=1)

print(f"Inserting {len(all_events)} usage events...")
cur.executemany(
    """INSERT OR IGNORE INTO usage_events
       (received_at,user_email,user_token,device_id,session_id,turn_index,
        timestamp_utc,session_start_utc,model,input_tokens,output_tokens,
        cache_read_tokens,cache_write_tokens,plugin_version,schema_version)
       VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
    all_events
)

# ---------------------------------------------------------------------------
# Tier hints — infer from peak window tokens
# ---------------------------------------------------------------------------

TIER_BOUNDARIES = {
    "max20":    350_000,
    "max5":      80_000,
    "standard":  15_000,
    "light":          0,
}

for email, _, tier, _, _ in USERS:
    peak = TIER_BOUNDARIES[tier] + rng.randint(-5000, 20000)
    window_count = rng.randint(12, 40)
    label = tier.replace("max20", "Max 20×").replace("max5", "Max 5×").replace("standard", "Standard").replace("light", "Light")
    cur.execute(
        "INSERT OR REPLACE INTO tier_hints(email,tier_label,peak_tokens,method,window_count,updated_at) VALUES(?,?,?,?,?,?)",
        (email, label, peak, "inferred", window_count, fmt(NOW))
    )

conn.commit()
conn.close()

# Summary
print(f"Done. Database written to: {DB_PATH}")
print(f"  Users:  {len(USERS)}")
print(f"  Events: {len(all_events)}")
