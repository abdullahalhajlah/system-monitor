# System Monitor

A Linux system monitoring service written in Rust that collects real-time system metrics and exposes them via a REST API.

## Overview

The service runs as a long-lived process that:
- Reads system data directly from the Linux `/proc` and `/sys` filesystems
- Maintains an in-memory rolling history of readings
- Exposes current and historical metrics via HTTP endpoints in JSON format

It is configurable via a TOML file and includes a systemd unit file for production deployment.

## Requirements

- Rust 1.75 or newer (install via [rustup](https://rustup.rs/))
- A Linux system with access to `/proc` and `/sys` (or WSL2 on Windows)

## Build and Run

Clone the repository and build:

    git clone https://github.com/abdullahalhajlah/system-monitor.git
    cd system-monitor
    cargo build

Run with the provided configuration file:

    cargo run -- --config config.toml

The service will start and print its listen address:

    Listening on 0.0.0.0:8080

You can then access the API endpoints from any HTTP client (browser, `curl`, etc.).

### Notes on development environment

This service was developed and tested under WSL2 (Ubuntu 22.04) on Windows. Most metrics (uptime, memory, CPU, network) work identically under WSL2 and on bare-metal Linux. CPU temperature reads from `/sys/class/thermal/thermal_zone*/temp`, which is not exposed by WSL2; the service handles this by returning `null` for that field. On bare-metal Linux with thermal sensors, the same code returns real temperature readings.

## Configuration

The service reads its settings from a TOML file specified by the `--config` argument. An example `config.toml` is included in the repository:

    listen_address = "0.0.0.0:8080"
    poll_interval_seconds = 10
    primary_interface = "eth0"
    history_retention_seconds = 3600

### Settings

| Setting | Type | Description |
|---------|------|-------------|
| `listen_address` | string | IP and port the HTTP server binds to (e.g. `"0.0.0.0:8080"`) |
| `poll_interval_seconds` | integer | How often the background collector takes a snapshot of system metrics |
| `primary_interface` | string | Network interface to mark as primary in `/api/network` responses |
| `history_retention_seconds` | integer | How long readings are kept in the in-memory history before being dropped |

Changes to `config.toml` take effect on service restart.

## API Documentation

All endpoints return JSON unless otherwise noted. Examples below assume the service is running locally on port 8080.

### `GET /api/health`

Simple liveness check. Used to verify the service is running.

**Response:** `200 OK` with body `OK` 

**Example:**

    $ curl -i http://localhost:8080/api/health
    HTTP/1.1 200 OK
    content-type: text/plain; charset=utf-8
    content-length: 2
    date: Sun, 31 May 2026 10:37:05 GMT

    OK

### `GET /api/system`

Returns the current snapshot of all system metrics.

**Response:** `200 OK` with a JSON object.

| Field | Type | Description |
|-------|------|-------------|
| `uptime_seconds` | number | Seconds since the system booted (from `/proc/uptime`) |
| `memory_total_kb` | integer | Total RAM in kilobytes (from `/proc/meminfo` → `MemTotal`) |
| `memory_used_kb` | integer | Used RAM in kilobytes (`MemTotal - MemAvailable`) |
| `memory_percent` | number | Memory utilization as a percentage, rounded to 2 decimals |
| `cpu_usage_percent` | number | Aggregate CPU usage across all cores over a 1-second sampling interval (from `/proc/stat`) |
| `cpu_temperature_celsius` | number or `null` | CPU temperature in °C (from `/sys/class/thermal/thermal_zone0/temp`); `null` if the sensor is not exposed (e.g. under WSL2) |

**Example:**

    $ curl -i http://localhost:8080/api/system
    HTTP/1.1 200 OK
    content-type: application/json
    content-length: 158
    date: Sun, 31 May 2026 10:39:21 GMT

    {
      "uptime_seconds": 283361.51,
      "memory_total_kb": 8078432,
      "memory_used_kb": 1759998,
      "memory_percent": 21.79,
      "cpu_usage_percent": 0.58,
      "cpu_temperature_celsius": null
    }

> The endpoint takes approximately 1 second to respond, because CPU usage is calculated from two readings of `/proc/stat` taken 1 second apart.

### `GET /api/network`

Returns a list of all network interfaces with per-interface statistics. Data is read from `/proc/net/dev` (byte counters) and `/sys/class/net/<interface>/operstate` (link state).

**Response:** `200 OK` with a JSON array of interface objects.

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Interface name (e.g. `eth0`, `lo`) |
| `rx_bytes` | integer | Total bytes received on this interface since boot |
| `tx_bytes` | integer | Total bytes transmitted on this interface since boot |
| `link_state` | string | Operational state: `up`, `down`, or `unknown` |
| `is_primary` | boolean | `true` if this interface matches `primary_interface` in `config.toml` |

**Example:**

    $ curl -i http://localhost:8080/api/network
    HTTP/1.1 200 OK
    content-type: application/json
    content-length: 193
    date: Sun, 31 May 2026 10:42:33 GMT

    [
      {
        "name": "lo",
        "rx_bytes": 294940809,
        "tx_bytes": 294940809,
        "link_state": "unknown",
        "is_primary": false
      },
      {
        "name": "eth0",
        "rx_bytes": 610248839,
        "tx_bytes": 139313803,
        "link_state": "up",
        "is_primary": true
      }
    ]

> Byte counters are cumulative since system boot. To compute rate (bytes per second), poll the endpoint twice and divide the difference by the interval.

### `GET /api/network/{interface}`

Returns statistics for a specific network interface. The `{interface}` path parameter is the interface name (e.g. `eth0`).

**Responses:**
- `200 OK` with a JSON object if the interface exists
- `404 Not Found` (empty body) if the interface does not exist

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Interface name |
| `rx_bytes` | integer | Total bytes received since boot |
| `tx_bytes` | integer | Total bytes transmitted since boot |
| `link_state` | string | Operational state: `up`, `down`, or `unknown` |

> The `is_primary` field is omitted from this endpoint, since the caller has already selected a specific interface by name.

**Example — existing interface:**

    $ curl -i http://localhost:8080/api/network/eth0
    HTTP/1.1 200 OK
    content-type: application/json
    content-length: 75
    date: Sun, 31 May 2026 10:44:56 GMT

    {
      "name": "eth0",
      "rx_bytes": 610250171,
      "tx_bytes": 139322077,
      "link_state": "up"
    }

**Example — interface that does not exist:**

    $ curl -i http://localhost:8080/api/network/wlan999
    HTTP/1.1 404 Not Found
    content-length: 0
    date: Sun, 31 May 2026 10:44:56 GMT

    ### `GET /api/history?minutes=N`

Returns historical snapshots from the in-memory ring buffer. A background task records a snapshot every `poll_interval_seconds` (configured in `config.toml`); entries older than `history_retention_seconds` are dropped automatically.

**Query parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `minutes` | integer | `5` | Return only entries from the last *N* minutes |

**Response:** `200 OK` with a JSON array of history entries (oldest first). Each entry has the same fields as `/api/system`, plus a `timestamp` field (seconds since Unix epoch, UTC).

**Example — last 1 minute of history:**

    $ curl -i "http://localhost:8080/api/history?minutes=1"
    HTTP/1.1 200 OK
    content-type: application/json
    content-length: 1089
    date: Sun, 31 May 2026 10:47:40 GMT

    [
      {
        "timestamp": 1780224406,
        "uptime_seconds": 283834.80,
        "memory_total_kb": 8078432,
        "memory_used_kb": 1768684,
        "memory_percent": 21.89,
        "cpu_usage_percent": 0.25,
        "cpu_temperature_celsius": null
      },
      {
        "timestamp": 1780224415,
        "uptime_seconds": 283845.81,
        "memory_total_kb": 8078432,
        "memory_used_kb": 1766532,
        "memory_percent": 21.87,
        "cpu_usage_percent": 3.67,
        "cpu_temperature_celsius": null
      }
    ]

> The response above is truncated to two entries for brevity. The actual response contained six entries — one approximately every 10 seconds (matching `poll_interval_seconds`).

**Example — default 5-minute window:**

    $ curl http://localhost:8080/api/history

## Architecture

The service is structured around two concurrent components sharing a single piece of in-memory state:
                    ┌────────────────────┐
                    │  Shared History    │
                    │  Arc<Mutex<Vec<T>>>│
                    └────────┬───────────┘
                   writes    │    reads
                             │
          ┌──────────────────┴───────────────────────┐
          │                                          │
 ┌────────▼─────────┐                     ┌──────────▼──────────┐
 │  Background      │                     │   axum HTTP server  │
 │  Collector       │                     │   (handles requests │
 │  (tokio task,    │                     │    on demand)       │
 │   ticks every    │                     │                     │
 │   N seconds)     │                     │                     │
 └────────┬─────────┘                     └─────────────────────┘
          │ reads
 ┌────────▼─────────┐
 │  /proc and /sys  │
 │  filesystems     │
 └──────────────────┘

 - **Background collector** — a `tokio::spawn` task that wakes every `poll_interval_seconds`, reads system data from `/proc` and `/sys`, and appends a timestamped entry to the shared history buffer. Old entries beyond `history_retention_seconds` are dropped on each tick.

- **HTTP server** — an `axum` router that serves the API endpoints. Handlers either read directly from `/proc` and `/sys` (for `/api/system` and `/api/network`) or from the shared history buffer (for `/api/history`).
- **Shared state** — the history buffer is wrapped in `Arc<Mutex<Vec<HistoryInfo>>>` so the collector (writer) and HTTP handlers (readers) can access it safely from independent async tasks.

System-reading logic is factored into a single `collect_snapshot()` function used by both the live `/api/system` endpoint and the background collector, avoiding duplication.

## Production Considerations

This service is functional but intentionally minimal. The following are improvements that would be required (or recommended) before a real production deployment:

### Reliability

- **Replace `.unwrap()` and `.expect()` with graceful error handling.** The current code panics on unexpected conditions (missing files, parse errors). In production, parsing errors on individual `/proc` reads should be logged and returned as `null` or partial responses rather than crashing the service.
- **Add a `tokio::sync::RwLock` instead of `std::sync::Mutex` for the history buffer.** This would allow multiple concurrent readers (HTTP handlers) without blocking each other, while still serializing writes.
- **Handle the case where the configured `primary_interface` does not exist.** Currently the field is read but never validated against actual interfaces; an invalid name silently results in `is_primary: false` for everything.

### Security

- **Run as a non-privileged user.** The included systemd unit file already runs the service as a dedicated `monitor` user with hardening directives (`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`).
- **Restrict network exposure.** Binding to `0.0.0.0` exposes the service on all interfaces. In production this should bind to a private interface or be placed behind a reverse proxy with authentication.
- **Add authentication.** The current API is open to anyone who can reach the listening port. A real deployment should require an API token, mutual TLS, or live on a trusted internal network.

### Observability

- **Structured logging.** Replace ad-hoc `println!` calls with a proper logging framework (e.g. the `tracing` crate) supporting log levels, structured fields, and machine-parseable output.
- **Metrics endpoint.** Expose service-internal metrics (collector tick latency, buffer size, failed reads) at a `/metrics` endpoint in Prometheus format so the monitor itself can be monitored.
- **Health beyond `/api/health`.** The current health endpoint only confirms the HTTP listener is up. A production-grade liveness check would also verify the collector has produced a recent snapshot.

### Resource management

- **Bound CPU sampling cost.** Each `/api/system` request currently spends 1 second sampling `/proc/stat`. Under heavy load this is wasteful; the collector already produces snapshots, so `/api/system` could return the most recent buffered entry instead of re-sampling.
- **Configurable history size limit.** History retention is currently time-based (`history_retention_seconds`). A defensive maximum entry count would protect against memory growth if `poll_interval_seconds` is set very low.

### Testing

- **Integration tests** that spin up the server and verify HTTP responses end-to-end.
- **Tests against captured `/proc` fixtures** to verify parsing handles real-world variations (different kernel versions, missing fields, etc.).
- **Load testing** to characterize behavior under sustained request volume.

## Project Structure

    system-monitor/
    ├── Cargo.toml                 # Dependencies and build configuration
    ├── Cargo.lock                 # Locked dependency versions
    ├── config.toml                # Example service configuration
    ├── system-monitor.service     # systemd unit file for production deployment
    ├── README.md                  # This file
    ├── .gitignore                 # Excludes /target and IDE files
    └── src/
        └── main.rs                # All service logic (handlers, collector, parsing)