#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# run_comparison.sh — compare rs-snap7 vs C (libsnap7) vs Python (python-snap7)
#
# Usage:
#   ./bench-compare/run_comparison.sh [--port PORT] [--iters N] [--rust-only]
#
# Flags:
#   --port  PORT   Port for the test server (default: random free port)
#   --iters N      Iterations for Python/C benchmarks (default: 1000)
#   --rust-only    Skip Python and C benchmarks (useful without libsnap7)
#   --no-rust      Skip Rust benchmark (run only Python/C)
#
# Requirements:
#   Rust bench:   cargo (in PATH), aarch64-unknown-linux-gnu target
#   Python bench: pip install python-snap7; libsnap7.so in LD_LIBRARY_PATH
#   C bench:      gcc + libsnap7-dev (-lsnap7)
# ---------------------------------------------------------------------------
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET=aarch64-unknown-linux-gnu
ITERS=1000
PORT=""
RUN_RUST=1
RUN_PYTHON=1
RUN_C=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)   PORT="$2"; shift 2 ;;
        --iters)  ITERS="$2"; shift 2 ;;
        --rust-only) RUN_PYTHON=0; RUN_C=0; shift ;;
        --no-rust)   RUN_RUST=0; shift ;;
        *) echo "Unknown flag: $1" >&2; exit 1 ;;
    esac
done

# ----- helpers -----
die() { echo "ERROR: $*" >&2; exit 1; }

free_port() {
    python3 -c "
import socket
s = socket.socket()
s.bind(('127.0.0.1', 0))
port = s.getsockname()[1]
s.close()
print(port)
"
}

if [[ -z "$PORT" ]]; then
    PORT=$(free_port)
fi

echo "=== rs-snap7 comparison benchmark ==="
echo "  Server port: $PORT"
echo "  Iterations:  $ITERS (Python/C)"
echo ""

# ----- Build test server binary (host arch — must override .cargo/config.toml default) -----
HOST_TARGET=$(rustc --print host-tuple 2>/dev/null || echo "aarch64-unknown-linux-gnu")
echo "[build] snap7-test-server (host: $HOST_TARGET)..."
(cd "$REPO_ROOT" && cargo build --release --bin snap7-test-server --target "$HOST_TARGET" 2>&1 | tail -3)
SERVER_BIN="$REPO_ROOT/target/$HOST_TARGET/release/snap7-test-server"
[[ -x "$SERVER_BIN" ]] || die "snap7-test-server not found at $SERVER_BIN"

# ----- Start server -----
echo "[server] starting on port $PORT..."
"$SERVER_BIN" "$PORT" &
SERVER_PID=$!
trap "kill $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null; true" EXIT

# Wait for server to accept connections (retry up to 2s)
for i in $(seq 1 20); do
    if python3 -c "
import socket, sys
s = socket.socket()
s.settimeout(0.1)
try:
    s.connect(('127.0.0.1', $PORT))
    s.close()
    sys.exit(0)
except:
    sys.exit(1)
" 2>/dev/null; then
        break
    fi
    sleep 0.1
done

# ----- Rust benchmark -----
RUST_JSON=""
RUST_SYNC_JSON=""
if [[ "$RUN_RUST" -eq 1 ]]; then
    echo "[rust async] running criterion benchmarks..."
    (cd "$REPO_ROOT" && SNAP7_BENCH_PORT="$PORT" cargo bench -p snap7-bench --bench s7_ops --target "$TARGET" 2>&1 | grep -E "^(db_read|db_write|roundtrip|time:|\s+\[)" || true) | head -40 || true

    echo "[rust sync] running criterion benchmarks (single-threaded runtime)..."
    (cd "$REPO_ROOT" && SNAP7_BENCH_PORT="$PORT" cargo bench -p snap7-bench --bench s7_ops_sync --target "$TARGET" 2>&1 | grep -E "^(db_read|db_write|time:|\s+\[)" || true) | head -40 || true

    CRITERION_DIR="$REPO_ROOT/target/criterion"
    if [[ -d "$CRITERION_DIR" ]]; then
        # Extract median times from criterion JSON estimates
        _EXTRACT_SCRIPT='
import os, json, sys

base = sys.argv[1]
results = {}
for root, dirs, files in os.walk(base):
    est = os.path.join(root, "new", "estimates.json")
    if os.path.exists(est):
        with open(est) as f:
            data = json.load(f)
        parts = root.replace(base, "").strip("/").split("/")
        if len(parts) >= 2:
            key = "/".join(parts[:2]).replace("\\", "/")
            results[key] = {
                "median_us": data.get("median", {}).get("point_estimate", 0) / 1000.0,
                "mean_us": data.get("mean", {}).get("point_estimate", 0) / 1000.0,
            }
print(json.dumps(results, indent=2))
'
        RUST_JSON=$(python3 - "$CRITERION_DIR" <<< "$_EXTRACT_SCRIPT")
        # Remap sync group names: db_read_sync/N -> db_read/N for table lookup
        RUST_SYNC_JSON=$(python3 - "$CRITERION_DIR" <<< "$_EXTRACT_SCRIPT" | python3 -c '
import json, sys
d = json.load(sys.stdin)
out = {}
for k, v in d.items():
    if "_sync/" in k:
        out[k.replace("_sync/", "/")] = v
print(json.dumps(out, indent=2))
')
    fi
fi

# ----- Python benchmark -----
PYTHON_JSON=""
if [[ "$RUN_PYTHON" -eq 1 ]]; then
    echo "[python] running python-snap7 benchmark (myenv)..."
    PYENV_ROOT="${PYENV_ROOT:-$HOME/.pyenv}"
    PYTHON_BIN="$PYENV_ROOT/versions/myenv/bin/python3"
    if [[ ! -x "$PYTHON_BIN" ]]; then
        # fallback: find myenv python via pyenv
        PYTHON_BIN=$(PYENV_VERSION=myenv "$PYENV_ROOT/bin/pyenv" which python3 2>/dev/null || echo "")
    fi
    if [[ -z "$PYTHON_BIN" || ! -x "$PYTHON_BIN" ]]; then
        PYTHON_JSON='{"error":"myenv python not found — run: pyenv virtualenv <version> myenv && pip install python-snap7"}'
    else
        PYTHON_JSON=$(
            LD_LIBRARY_PATH="/usr/lib:${LD_LIBRARY_PATH:-}" \
                "$PYTHON_BIN" "$SCRIPT_DIR/bench_python.py" 127.0.0.1 "$PORT" "$ITERS" 2>/tmp/bench_python_err
        ) || {
            ERR=$(cat /tmp/bench_python_err 2>/dev/null | tail -5)
            PYTHON_JSON="{\"error\":\"python bench failed: $(echo "$ERR" | tr '\n' ' ')\"}"
        }
    fi
fi

# ----- C benchmark -----
SNAP7_HDR_DIR="/tmp/snap7/release/wrappers/c-cpp"
C_JSON=""
if [[ "$RUN_C" -eq 1 ]]; then
    echo "[c] building and running libsnap7 C benchmark..."
    C_BIN="$SCRIPT_DIR/bench_c_bin"
    if gcc -O2 -std=c11 \
        -I"$SNAP7_HDR_DIR" \
        -o "$C_BIN" "$SCRIPT_DIR/bench_c.c" \
        -L/usr/lib -lsnap7 -lm \
        -Wl,-rpath,/usr/lib 2>/tmp/bench_c_build_err; then
        C_JSON=$("$C_BIN" 127.0.0.1 "$PORT" "$ITERS" 2>/tmp/bench_c_run_err) || {
            ERR=$(cat /tmp/bench_c_run_err 2>/dev/null | tail -3)
            C_JSON="{\"error\":\"C bench failed: $ERR\"}"
        }
    else
        ERR=$(cat /tmp/bench_c_build_err 2>/dev/null | tail -3)
        C_JSON="{\"error\":\"gcc build failed: $ERR\"}"
    fi
fi

# ----- Print comparison table -----
python3 - "$RUST_JSON" "$RUST_SYNC_JSON" "$PYTHON_JSON" "$C_JSON" <<'EOF'
import json, sys

def safe_parse(s):
    if not s:
        return {}
    try:
        return json.loads(s)
    except:
        return {"error": f"parse failed: {s[:80]}"}

rust      = safe_parse(sys.argv[1])
rust_sync = safe_parse(sys.argv[2])
python    = safe_parse(sys.argv[3])
c         = safe_parse(sys.argv[4])

ops = [
    ("db_read/1",    1),
    ("db_read/4",    4),
    ("db_read/8",    8),
    ("db_read/64",   64),
    ("db_read/240",  240),
    ("db_write/1",   1),
    ("db_write/4",   4),
    ("db_write/8",   8),
    ("db_write/64",  64),
    ("db_write/240", 240),
]

HDR = f"{'Op':<18} {'Size':>6}  {'Rust-async':>12}  {'Rust-sync':>12}  {'Python':>12}  {'C':>10}  {'Py/sync':>8}  {'C/sync':>7}"
SEP = "-" * len(HDR)

print()
print("=" * len(HDR))
print("  rs-snap7 (async & sync) vs python-snap7 vs libsnap7 (C)  —  median latency (μs)")
print("=" * len(HDR))
print(HDR)
print(SEP)

for key, size in ops:
    ra  = rust.get(key, {}).get("median_us")
    rs  = rust_sync.get(key, {}).get("median_us")
    py  = python.get(key, {}).get("median_us")
    c_v = c.get(key, {}).get("median_us")

    def fmt(v, w=12):
        return f"{'N/A':>{w}}" if v is None else f"{v:{w}.1f}"

    def ratio(a, b):
        return "   N/A" if (a is None or b is None or b == 0) else f"{a/b:8.2f}x"

    print(f"{key:<18} {size:>6}  {fmt(ra)}  {fmt(rs)}  {fmt(py)}  {fmt(c_v, 10)}  {ratio(py, rs)}  {ratio(c_v, rs)}")

print(SEP)
print()

for label, d in [("rust-async", rust), ("rust-sync", rust_sync), ("python", python), ("c", c)]:
    if "error" in d:
        print(f"  [{label}] {d['error']}")

print()
print("Notes:")
print("  All: same external snap7-test-server, loopback TCP")
print("  Rust-async: Criterion, multi-thread tokio runtime (task scheduler overhead per call)")
print("  Rust-sync:  Criterion, current_thread tokio runtime (no scheduler, fair vs Python/C)")
print("  Python/C:   wall-clock perf_counter / CLOCK_MONOTONIC, 1000 sequential calls")
print("  Ratio > 1x = slower than Rust-sync")
EOF

echo ""
echo "Criterion HTML report: target/criterion/report/index.html"
