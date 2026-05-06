#!/usr/bin/env python3
"""
python-snap7 throughput benchmark.

Connects to a running S7Server at the given address, measures db_read/db_write
latency for several payload sizes, and prints JSON results to stdout.

Usage:
    python3 bench_python.py <host> <port> [iterations]

Requires:
    pip install python-snap7
    The target system must have libsnap7.so available (e.g. from snap7 package).
"""

import json
import sys
import time

def usage():
    print(__doc__, file=sys.stderr)
    sys.exit(1)

def main():
    if len(sys.argv) < 3:
        usage()

    host = sys.argv[1]
    port = int(sys.argv[2])
    iters = int(sys.argv[3]) if len(sys.argv) > 3 else 1000

    try:
        import snap7
        from snap7.util import set_int
    except ImportError:
        print(json.dumps({"error": "python-snap7 not installed: pip install python-snap7"}))
        sys.exit(2)

    client = snap7.client.Client()
    try:
        client.connect(host, 0, 1, tcp_port=port)
    except Exception as e:
        print(json.dumps({"error": f"connect failed: {e}"}))
        sys.exit(3)

    results = {}
    sizes = [1, 4, 8, 64, 240]

    # db_read
    for size in sizes:
        times = []
        for _ in range(iters):
            t0 = time.perf_counter()
            client.db_read(1, 0, size)
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1e6)  # microseconds
        times.sort()
        n = len(times)
        results[f"db_read/{size}"] = {
            "median_us": times[n // 2],
            "p95_us": times[int(n * 0.95)],
            "p99_us": times[int(n * 0.99)],
            "mean_us": sum(times) / n,
            "ops_per_sec": n / (sum(times) / 1e6),
            "size_bytes": size,
            "iters": iters,
        }

    # db_write
    for size in sizes:
        payload = bytes([0xAB] * size)
        times = []
        for _ in range(iters):
            t0 = time.perf_counter()
            client.db_write(2, 0, bytearray(payload))
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1e6)
        times.sort()
        n = len(times)
        results[f"db_write/{size}"] = {
            "median_us": times[n // 2],
            "p95_us": times[int(n * 0.95)],
            "p99_us": times[int(n * 0.99)],
            "mean_us": sum(times) / n,
            "ops_per_sec": n / (sum(times) / 1e6),
            "size_bytes": size,
            "iters": iters,
        }

    client.disconnect()
    print(json.dumps(results, indent=2))

if __name__ == "__main__":
    main()
