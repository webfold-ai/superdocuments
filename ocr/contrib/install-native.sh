#!/usr/bin/env bash
# Install superdocuments-bench-ocr natively on a Linux host (tested on
# Ubuntu 22.04 / Debian 12). Run as a privileged user.
#
# Usage:
#   ./install-native.sh
#   # (run via your privilege-escalation tool of choice — doas or a root shell)
#
# Idempotent. Re-run after rebuilding the bench binary.
#
# What it does:
#   1. Creates /opt/superdocuments-bench with bin/, fixtures/, results/
#   2. Drops the bench binary (built locally) into bin/
#   3. Copies the fixtures/ tree
#   4. Installs the systemd unit (replacing <UPSTREAM> with the literal base URL
#      of the local OpenAI-compat gateway, defaulting to http://localhost:3100/v1
#      which is ai-gateway's default port)
#   5. Enables (but does not start) the unit. Run with `systemctl start superdocs-bench-ocr`
#      when the gateway is up.
set -euo pipefail

# Resolve the repo root from this script's location. The script lives at
# ocr/contrib/install-native.sh relative to the repo.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OCR_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$OCR_DIR/.." && pwd)"

if [[ ! -x "$OCR_DIR/target/release/superdocuments-bench-ocr" ]]; then
    echo "error: $OCR_DIR/target/release/superdocuments-bench-ocr not found." >&2
    echo "build it first: cargo build --release --manifest-path $OCR_DIR/Cargo.toml" >&2
    exit 1
fi

UPSTREAM="${OPENAI_BASE_URL:-http://localhost:3100/v1}"

echo "[1/5] Creating /opt/superdocuments-bench"
install -d -m 0755 -o root -g root /opt/superdocuments-bench
install -d -m 0755 -o root -g root /opt/superdocuments-bench/bin
install -d -m 0755 -o root -g root /opt/superdocuments-bench/fixtures
install -d -m 0755 -o root -g root /var/log/superdocs-bench

echo "[2/5] Installing binary"
install -m 0755 -o root -g root "$OCR_DIR/target/release/superdocuments-bench-ocr" \
    /opt/superdocuments-bench/bin/superdocuments-bench-ocr

echo "[3/5] Installing fixtures"
cp -r "$OCR_DIR/fixtures/." /opt/superdocuments-bench/fixtures/

echo "[4/5] Installing systemd unit"
TMP_UNIT="$(mktemp)"
sed "s|<UPSTREAM>|${UPSTREAM}|g" "$OCR_DIR/contrib/superdocuments-bench-ocr.service" > "$TMP_UNIT"
install -m 0644 -o root -g root "$TMP_UNIT" /etc/systemd/system/superdocuments-bench-ocr.service
rm -f "$TMP_UNIT"

echo "[5/5] Reloading systemd (does NOT start the unit)"
systemctl daemon-reload

echo
echo "Done. To run once:"
echo "  systemctl start superdocuments-bench-ocr   # via your privilege-escalation tool"
echo "To enable on boot:"
echo "  systemctl enable superdocuments-bench-ocr"
echo "To inspect:"
echo "  journalctl -u superdocuments-bench-ocr"
echo "To update the upstream after install:"
echo "  systemctl edit superdocuments-bench-ocr   # override OPENAI_BASE_URL"
