#!/usr/bin/env bash
# generate_demo.sh — build a fresh demo database, stand up the receiver,
# export the static admin dashboard HTML, and copy it into the docs.
#
# Usage:
#   ./generate_demo.sh [--rebuild]
#
# Options:
#   --rebuild    Force a cargo release build even if the binary already exists.
#
# Outputs:
#   docs/src/demo.html   — standalone admin dashboard snapshot (no server needed)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
RECEIVER_BIN="$REPO_ROOT/receiver/target/release/ccflux-receiver"
SEED_SCRIPT="$REPO_ROOT/seed_docs_db.py"
DB_FILE="$REPO_ROOT/docs_demo.db"
OUT_HTML="$REPO_ROOT/docs/src/dashboard.html"

ADMIN_TOKEN="demo-admin-token-ccflux"
LISTEN_PORT=19876
BASE_URL="http://127.0.0.1:$LISTEN_PORT"

REBUILD=0
for arg in "$@"; do
  [[ "$arg" == "--rebuild" ]] && REBUILD=1
done

# ── 1. Build receiver ────────────────────────────────────────────────────────

if [[ "$REBUILD" == 1 ]] || [[ ! -x "$RECEIVER_BIN" ]]; then
  echo "[1/5] Building receiver (release)..."
  cd "$REPO_ROOT/receiver"
  cargo build --release --quiet
  cd "$REPO_ROOT"
else
  echo "[1/5] Receiver binary already exists, skipping build. (pass --rebuild to force)"
fi

# ── 2. Seed database ─────────────────────────────────────────────────────────

echo "[2/5] Generating demo database..."
rm -f "$DB_FILE"
python3 "$SEED_SCRIPT" "$DB_FILE"

# ── 3. Start receiver ────────────────────────────────────────────────────────

echo "[3/5] Starting receiver on port $LISTEN_PORT..."

# Pick up any stale process from a previous interrupted run.
pkill -f "ccflux-receiver.*$LISTEN_PORT" 2>/dev/null || true

DATABASE_PATH="$DB_FILE" \
  LISTEN_ADDR="127.0.0.1:$LISTEN_PORT" \
  ADMIN_TOKEN="$ADMIN_TOKEN" \
  BASE_URL="$BASE_URL" \
  "$RECEIVER_BIN" &>/tmp/ccflux-demo-receiver.log &
RECEIVER_PID=$!

# Wait for the receiver to be ready.
READY=0
for i in $(seq 1 30); do
  if curl -sf "$BASE_URL/health" >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 0.2
done

if [[ "$READY" == 0 ]]; then
  echo "ERROR: receiver did not become healthy within 6 seconds."
  echo "       Check /tmp/ccflux-demo-receiver.log for details."
  kill "$RECEIVER_PID" 2>/dev/null || true
  exit 1
fi
echo "       Receiver ready (PID $RECEIVER_PID)."

# ── 4. Export static HTML ────────────────────────────────────────────────────

echo "[4/5] Exporting dashboard HTML..."
HTTP_STATUS=$(curl -s -o "$OUT_HTML" -w "%{http_code}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$BASE_URL/admin/?export")

kill "$RECEIVER_PID" 2>/dev/null || true

if [[ "$HTTP_STATUS" != "200" ]]; then
  echo "ERROR: export request returned HTTP $HTTP_STATUS (expected 200)."
  echo "       Check /tmp/ccflux-demo-receiver.log for details."
  rm -f "$OUT_HTML"
  exit 1
fi

HTML_BYTES=$(wc -c < "$OUT_HTML")
echo "       Saved $HTML_BYTES bytes to docs/src/$(basename "$OUT_HTML")."

# ── 5. Verify ────────────────────────────────────────────────────────────────

echo "[5/5] Verifying output..."
if ! grep -q "ccflux admin" "$OUT_HTML"; then
  echo "ERROR: output HTML does not look like the admin dashboard."
  exit 1
fi
if ! grep -q "export.com" "$OUT_HTML" 2>/dev/null && grep -q "example.com" "$OUT_HTML"; then
  echo "       Email domain: example.com  ✓"
fi
if grep -qE '<form[^>]*action="/admin/(users|device-keys)' "$OUT_HTML"; then
  echo "WARNING: mutating forms found in export — check read_only flag."
else
  echo "       No mutating forms in export  ✓"
fi

echo ""
echo "Done. Demo dashboard written to docs/src/dashboard.html"
echo "Run 'cd docs && mdbook build' to include it in the book."
