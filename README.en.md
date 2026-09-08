# Delta Bypass for Rust

English · [简体中文](README.md)

A high-performance automated verification bypass service for Platoboost written in Rust. Automatically processes sequential checkpoints to extract access keys, providing both a Command Line Interface (CLI) and an HTTP REST API.

---

## Overview

Delta Bypass for Rust automates the entire Platoboost verification workflow. Given a target authorization URL or credential string, the service automatically navigates sequential checkpoints and retrieves the final access key.

Developed by **Hasl_Team**, this project is a high-performance Rust rewrite of Hasl_Team's original Python implementation ([Delta-bypass Source Repository](https://github.com/AbabaHnb/Delta-bypass)), significantly improving execution speed and concurrent throughput.

## System Requirements & Prerequisites

* **Zero Language Runtimes Required**: No Python, Node.js, .NET, or interpreter installation required.
* **No External Shared Library Dependencies**: OpenSSL is not required (cryptography routines are written in pure Rust and statically linked).
* **Network & Storage Requirements**:
  * Internet access is required to reach `auth.platorelay.com`.
  * The working directory must be **writable**. The service automatically creates and maintains `.key_cache.json` (deleting this file is safe; it automatically rebuilds).

---

## Building and Installation

Rust 1.75 or higher is required.

### Standard Build from Source

```bash
# Clone repository and build release binary
cargo build --release
```

The output binary is placed at `target/release/delta-bypass` (`delta-bypass.exe` on Windows). The single binary can be copied and deployed independently.

### Automated Cloud Build via GitHub Actions (Recommended)

The project includes an automated GitHub Actions CI/CD workflow ([.github/workflows/build.yml](file:///.github/workflows/build.yml)).
- **On Push / PR**: Binaries are automatically built for Windows (x64), Linux (GNU), and Linux (Musl static). Artifacts can be downloaded directly from the **Actions** tab on GitHub.
- **On Version Tag Push**: Pushing a version tag (e.g., `git tag v1.2.0 && git push origin v1.2.0`) automatically publishes a GitHub Release with pre-compiled binaries attached.

### Linux C Toolchain Prerequisites

If a C compiler is missing on your Linux distribution, install standard build utilities:

```bash
# Debian / Ubuntu
sudo apt install build-essential

# CentOS / RHEL / Rocky Linux
sudo dnf groupinstall "Development Tools"

# Alpine Linux
apk add build-base
```

### Static Binary Compilation with Musl (Linux)

To prevent `GLIBC_2.xx not found` errors on older Linux distributions, build a fully static binary independent of system `glibc`:

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools          # Debian/Ubuntu (or musl-gcc on other distros)
cargo build --release --target x86_64-unknown-linux-musl
```

The resulting binary at `target/x86_64-unknown-linux-musl/release/delta-bypass` depends on no shared libraries and runs seamlessly across any Linux distribution (e.g., CentOS 7, Alpine, Arch Linux).

---

## Performance Benchmark

Measured on an 8-core CPU system running Debian Linux:

| Benchmark Metric | Python Original | Rust Rewrite | Improvement / Comparison |
|---|---|---|---|
| End-to-End Single Bypass | ~6.8 seconds | **~5.5 seconds** | ~20% overall speedup |

> **Note**: Out of the ~5.5-second total elapsed time, approximately 5.0 seconds consist of mandatory upstream checkpoint cooldown intervals (enforced server delays). Local CPU processing and algorithms take only ~0.5 seconds.

---

## Command Line Interface (CLI) Guide

### Usage Examples

> **Warning (Command Line Quotes)**: Authorization URLs contain special characters like `&`. Always enclose URLs in **double quotes** on Windows CMD/PowerShell and Linux Shells to avoid parameter truncation.

```bash
# Bypass a specific Platoboost URL
delta-bypass "https://auth.platorelay.com/a?d=<pass_credential>"

# Pass a credential string directly or specify a file containing credentials
delta-bypass "<pass_credential>"
delta-bypass tickets.txt

# Generate 3 test URLs and automatically execute bypass
delta-bypass --generate 3

# Generate test URLs only (without executing bypass)
delta-bypass --generate 5 --no-auto

# Start the HTTP REST API server
delta-bypass --serve --port 2233
```

### Windows Terminal Encoding Setup

Legacy Windows Command Prompt (`cmd.exe`) defaults to non-UTF-8 encodings, which may render Chinese log characters as corrupted symbols.
1. **Temporary UTF-8 Session**: Run `chcp 65001` in CMD before executing the binary.
2. **Recommended Environment**: Use **Windows Terminal** or **PowerShell 7+**, which natively support UTF-8 encoding.

### CLI Options Reference

| Option | Default | Description |
|---|---|---|
| `<link>` | — | Target URL, credential string, or file path containing credentials |
| `--generate N` / `-g` | 0 | Generate N test verification URLs |
| `--quiet` / `-q` | Off | Silent mode: output final key only and suppress progress logs |
| `--max-rounds N` | 3 | Maximum round count (automatically adjusted to server checkpoint count) |
| `--no-auto` | Off | Used with `--generate` to generate URLs without running bypass |
| `--serve` | Off | Run in HTTP API server mode |
| `--host` | 0.0.0.0 | Server listen address (use `127.0.0.1` for local access only) |
| `--port` / `-p` | 2233 | Server listen port |

---

## HTTP REST API Specification

### Starting the Server

```bash
delta-bypass --serve --port 2233
```

### Request Example

```bash
curl -G http://127.0.0.1:2233/delta \
     --data-urlencode "url=https://auth.platorelay.com/a?d=<pass_credential>"
```

### Response Example

```json
{
  "key": "FREE_xxxxxxxx",
  "cached": false,
  "error": null,
  "made_by": "Hasl_Team",
  "qq_group": "277707901",
  "times": "5.512340000000s"
}
```

### Response Schema

| Field | Type | Description |
|---|---|---|
| `key` | String \| null | Retrieved access key (`null` on failure) |
| `cached` | Boolean | `true` indicates a hit in the 24-hour result cache |
| `error` | String \| null | Error message (`null` on success) |
| `times` | String | Actual bypass execution time (returns original duration on cache hit) |

### Error Code Dictionary

| Error Message | Cause & Resolution |
|---|---|
| `链接格式无效 / Malformed link` | Invalid URL format or failure to extract pass credentials |
| `链接无效 / Invalid link: <Server Message>` | Credential is invalid or expired (rejected by upstream, non-retryable) |
| `绕过失败 / Bypass failed` | Key extraction failed after two attempts (includes one retry) |
| `内部执行异常 / Internal execution error` | Unexpected internal runtime exception |
| `未获得结果 / No result returned` | Channel closed unexpectedly during execution |

---

## Performance Optimizations

The application incorporates two key architectural optimizations to minimize latency:

1. **Send-Time Cooldown Tracking (saves ~0.2s)**  
   Upstream evaluates the 5-second checkpoint cooldown based on server request arrival timestamps. Tracking is calculated from client **request dispatch** rather than response receipt, saving one full Network Round-Trip Time (RTT). An adaptive self-tuning margin absorbs network jitter while preventing rate limits.

2. **HTTP Connection Pooling (saves ~0.6s)**  
   Persistent TCP/TLS connections are maintained with the upstream server, avoiding repeated handshake costs.

3. **Same-Link Request Merging**  
   Concurrent requests for one link trigger only a single real bypass; the rest share the same result.

---

## Production Deployment & Service Management

### 1. Windows Service Setup

#### Option A: Windows Task Scheduler (Native)
1. Open Task Scheduler and select "Create Task".
2. **General**: Select "Run whether user is logged on or not" and "Run with highest privileges".
3. **Triggers**: Add a new trigger set to "At startup".
4. **Actions**: Add a new action. Program: absolute path to `delta-bypass.exe`. Arguments: `--serve --host 127.0.0.1 --port 2233`. **Set "Start in" to the directory containing the executable** (omitting this causes working directory redirection to `C:\Windows\System32`).
5. **Settings**: Check "If the task fails, restart every...".

#### Option B: NSSM Service Manager (Recommended with Auto-Restart & Logs)
Download NSSM from [nssm.cc](https://nssm.cc), then execute in an Administrator Command Prompt:

```cmd
nssm install DeltaBypass C:\delta-bypass\delta-bypass.exe
nssm set DeltaBypass AppParameters "--serve --host 127.0.0.1 --port 2233"
nssm set DeltaBypass AppDirectory C:\delta-bypass
nssm set DeltaBypass AppStdout C:\delta-bypass\out.log
nssm set DeltaBypass AppStderr C:\delta-bypass\err.log
nssm start DeltaBypass
```

### 2. Linux Systemd Service

Deploy using the included service configuration at `deploy/delta-bypass.service`:

```bash
# 1. Create installation directory and copy binary
sudo mkdir -p /opt/delta-bypass
sudo cp target/release/delta-bypass /opt/delta-bypass/
sudo chown -R www:www /opt/delta-bypass

# 2. Install and activate Systemd service
sudo cp deploy/delta-bypass.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now delta-bypass

# 3. Check service status and live logs
systemctl status delta-bypass
journalctl -u delta-bypass -f
```

*Create the non-login system user `www` if it does not exist*:
```bash
sudo useradd -r -s /usr/sbin/nologin www
```

### 3. Docker Container Deployment

#### Multi-Stage Dockerfile

```dockerfile
# Build stage
FROM rust:1-alpine AS builder
RUN apk add --no-cache build-base
WORKDIR /build
COPY . .
RUN cargo build --release

# Runtime stage
FROM alpine:latest
RUN apk add --no-cache ca-certificates && \
    adduser -D -H app
WORKDIR /app
COPY --from=builder /build/target/release/delta-bypass /app/
RUN chown -R app:app /app
USER app
EXPOSE 2233
CMD ["/app/delta-bypass", "--serve", "--host", "0.0.0.0", "--port", "2233"]
```

#### Build & Run Commands

```bash
docker build -t delta-bypass .

# Run container with volume mount for key cache persistence
docker run -d --name delta-bypass \
  -p 127.0.0.1:2233:2233 \
  -v delta-keys:/app \
  --restart unless-stopped \
  delta-bypass
```

> **Notes**:
> 1. Container image must include `ca-certificates` for outbound HTTPS connections.
> 2. Set `--host 0.0.0.0` inside the container; constrain public exposure via host port binding `-p 127.0.0.1:2233:2233`.

### 4. Nginx Reverse Proxy & Security Hardening

**Security Warning**: The HTTP API features no authentication mechanisms by default. Anyone with network access to the port can consume bypass capacity. **Do not expose the API unauthenticated to the public Internet.**

Configure Nginx authentication and rate limiting:

```nginx
limit_req_zone $binary_remote_addr zone=delta:10m rate=10r/s;

location /delta {
    limit_req zone=delta burst=20 nodelay;
    auth_basic "Delta Bypass for Rust API Authorization";
    auth_basic_user_file /etc/nginx/.htpasswd;
    proxy_pass http://127.0.0.1:2233;
    proxy_read_timeout 120s;
}
```

---

## Project Structure

```
src/
├── main.rs         Command line entry point (CLI parsing and task dispatch)
├── lib.rs          Library entry point (module exports)
├── config.rs       Global configuration constants
├── api.rs          HTTP API service, key cache, and request merging
├── chain.rs        Main bypass flow (submit -> checkpoint progression -> key extraction)
├── auth.rs         Upstream auth server protocol implementation
├── crypto.rs       Upstream custom encryption implementation
├── net.rs          HTTP connection management
├── useragent.rs    User-Agent spoofing and mobile browser fingerprint generation
├── link.rs         Test link generator
└── timing.rs       High-resolution timing and performance tracing
```

---

## Core Configuration Parameters

All parameters are defined in `src/config.rs`:

| Constant | Default | Notes & Constraints |
|---|---|---|
| `MIN_STEP_GAP` | 5 s | Mandatory upstream checkpoint cooldown, **do not lower** |
| `GAP_MARGIN_START` | 250 ms | Initial adaptive delay margin |
| `POLL_MAX_ATTEMPTS` | 10 | Maximum key polling attempts |
| `MAX_ROUNDS_HARD_CAP` | 12 | Hard cap on checkpoint rounds (prevents infinite loops) |

---

## Troubleshooting

| Symptom | Root Cause | Resolution |
|---|---|---|
| `链接无效 / Invalid link` | Credential expired or rejected upstream | Use a new Platoboost link (upstream rejection) |
| `绕过失败 / Bypass failed` | A step in the flow did not succeed | Remove `--quiet` and check `终止于:` at the end of the log |
| Frequent "rate limit" logs | Checkpoint cooldown too short | Check `MIN_STEP_GAP`; raise if upstream tightens controls |
| Connection failures under high concurrency | System file descriptor limit | Run `ulimit -n 65535` or set `LimitNOFILE=65535` in Systemd |
| Windows CMD garbled Chinese output | Terminal not UTF-8 | Run `chcp 65001` or use Windows Terminal / PowerShell 7 |
| `GLIBC_2.xx not found` | Binary built against newer glibc | Build static musl target (`x86_64-unknown-linux-musl`) |
| `Permission denied` | Binary lacks executable bit | Run `chmod +x delta-bypass` |
| `.key_cache.json` not found | Working directory not the binary location | Check "Start in" (Task Scheduler) or `WorkingDirectory` (Systemd) |
| Certificate errors inside Docker | Missing CA root certificates | Install `ca-certificates` in the runtime stage |
| Port already in use | Another process bound to the port | Change `--port` or locate the conflict with `netstat` / `ss` |

---

## License

This project is licensed under the [MIT License](LICENSE).