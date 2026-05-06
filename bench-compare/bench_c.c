/*
 * libsnap7 C throughput benchmark.
 * Compile: gcc -O2 -std=c11 -D_POSIX_C_SOURCE=199309L ...
 *
 * Connects to a running S7 server, measures db_read/db_write latency for
 * several payload sizes, and prints JSON results to stdout.
 *
 * Build:
 *   gcc -O2 -o bench_c bench_c.c -lsnap7 -lm
 *
 *   (libsnap7-dev package, or build from source: https://snap7.sourceforge.net/)
 *
 * Usage:
 *   ./bench_c <host> <port> [iterations]
 */

#define _POSIX_C_SOURCE 199309L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <math.h>

#ifdef _WIN32
  #include <winsock2.h>
  typedef HANDLE S7Object;
#else
  #include <snap7.h>
#endif

#define MAX_ITERS 100000

static int cmp_double(const void *a, const void *b) {
    double da = *(double *)a;
    double db = *(double *)b;
    return (da > db) - (da < db);
}

static double now_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e6 + ts.tv_nsec / 1e3;
}

static void print_stats(const char *key, double *times, int n, int size) {
    qsort(times, n, sizeof(double), cmp_double);
    double sum = 0;
    for (int i = 0; i < n; i++) sum += times[i];
    double mean = sum / n;
    double total_s = sum / 1e6;
    printf(
        "  \"%s\": {\"median_us\": %.2f, \"p95_us\": %.2f, \"p99_us\": %.2f,"
        " \"mean_us\": %.2f, \"ops_per_sec\": %.1f, \"size_bytes\": %d, \"iters\": %d}",
        key,
        times[n / 2],
        times[(int)(n * 0.95)],
        times[(int)(n * 0.99)],
        mean,
        (double)n / total_s,
        size,
        n
    );
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <host> <port> [iterations]\n", argv[0]);
        return 1;
    }

    const char *host = argv[1];
    int port = atoi(argv[2]);
    int iters = argc > 3 ? atoi(argv[3]) : 1000;
    if (iters > MAX_ITERS) iters = MAX_ITERS;

    S7Object client = Cli_Create();

    /* p_u16_RemotePort = 2: set the server TCP port before connecting */
    int remote_port = port;
    Cli_SetParam(client, 2, &remote_port);

    int res = Cli_ConnectTo(client, host, 0, 1);
    if (res != 0) {
        char err_text[1024];
        Cli_ErrorText(res, err_text, sizeof(err_text));
        fprintf(stderr, "Connect failed: %s\n", err_text);
        fprintf(stdout, "{\"error\": \"connect failed: %s\"}\n", err_text);
        Cli_Destroy(&client);
        return 3;
    }

    int sizes[] = {1, 4, 8, 64, 240};
    int nsizes = (int)(sizeof(sizes) / sizeof(sizes[0]));
    static double times[MAX_ITERS];
    uint8_t buf[512];
    memset(buf, 0xAB, sizeof(buf));

    printf("{\n");
    int first = 1;

    /* db_read */
    for (int si = 0; si < nsizes; si++) {
        int size = sizes[si];
        for (int i = 0; i < iters; i++) {
            double t0 = now_us();
            Cli_DBRead(client, 1, 0, size, buf);
            double t1 = now_us();
            times[i] = t1 - t0;
        }
        char key[64];
        snprintf(key, sizeof(key), "db_read/%d", size);
        if (!first) printf(",\n"); first = 0;
        print_stats(key, times, iters, size);
    }

    /* db_write */
    for (int si = 0; si < nsizes; si++) {
        int size = sizes[si];
        for (int i = 0; i < iters; i++) {
            double t0 = now_us();
            Cli_DBWrite(client, 2, 0, size, buf);
            double t1 = now_us();
            times[i] = t1 - t0;
        }
        char key[64];
        snprintf(key, sizeof(key), "db_write/%d", size);
        if (!first) printf(",\n"); first = 0;
        print_stats(key, times, iters, size);
    }

    printf("\n}\n");

    Cli_Disconnect(client);
    Cli_Destroy(&client);
    return 0;
}
