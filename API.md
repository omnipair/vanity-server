# Vanity Server API

A blazingly fast REST API for grinding Solana vanity addresses synchronously.

## Quick Start

```bash
# Build and start server
cargo build --release --features server
cargo run --release --features server -- server

# Configure via .env file
cp .env.example .env
# Edit .env with your settings

# Grind addresses
curl -X GET 'http://localhost:8080/grind?base=3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd'
```

## Configuration

Server defaults and limits are read at startup from environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `VANITY_PORT` | Server port | `8080` |
| `VANITY_DEFAULT_TOKEN_PROGRAM` | Default owner pubkey | SPL Token program |
| `VANITY_DEFAULT_PREFIX` | Target prefix; counts toward the 4-character combined limit | None |
| `VANITY_DEFAULT_CASE_INSENSITIVE` | Case insensitive | `false` |
| `VANITY_TASK_TIMEOUT` | Job deadline in seconds; integer from 1 to 300 | `30` |
| `VANITY_MAX_CONCURRENT_TASKS` | Active jobs per instance; integer from 1 to 64 | `1` |
| `VANITY_MAX_REQUESTS_PER_MINUTE` | Per-instance token bucket refill per minute and maximum burst; integer from 1 to 10000 | `60` |
| `VANITY_CORS_ORIGINS` | CORS allowed origins (comma-separated) | Permissive (all origins) |

Invalid limits, an invalid owner, an invalid boolean, or a prefix that is not
valid Base58 or exceeds four characters cause startup to fail. Zero never means
unlimited. `VANITY_DEFAULT_CPUS` is no longer advertised: it did not limit the
HTTP worker threads. Each admitted job now uses one worker; configure
`VANITY_MAX_CONCURRENT_TASKS` according to the instance's CPU allocation.

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | API documentation |
| `/health` | GET | Health check |
| `/grind?base=<base>&suffix=<target>[&owner=<program>]` | GET | Grind vanity addresses (synchronous) |

### Grind Vanity Addresses
**GET** `/grind?base=<base>&suffix=<target>`

Waits for a match within the configured time budget, using query parameters and
server configuration. A match is not guaranteed within the budget.

**Query Parameters:**
- `base` (required): Base pubkey for grinding
- `suffix` (optional): Base58 target suffix. The configured prefix and requested
  suffix together must contain at most **four characters**, even in
  case-insensitive mode. With no prefix, `omni`, `yLP`, and `hLP` remain supported.
  Invalid characters are rejected rather than silently dropping the constraint.
- `owner` (optional): Program that will own the account created with the
  returned seed. Defaults to `VANITY_DEFAULT_TOKEN_PROGRAM`. The owner is part
  of the `create_with_seed` derivation, so a Token-2022 mint must be ground
  with `owner=TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`; a seed ground for
  the SPL Token program derives a different address under Token-2022.

**Response:**
```json
{
  "address": "H4rHNpqtJUZVRotbSxXTs8oWsL47V7wgPDxJAiuAomni",
  "seed": "gOUdv5rq5lf3Im0r",
  "seed_bytes": [103, 79, 85, 100, 118, 53, 114, 113, 53, 108, 102, 51, 73, 109, 48, 114],
  "base": "3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd",
  "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
  "prefix": null,
  "suffix": "omni",
  "case_insensitive": false,
  "attempts": 918349,
  "duration_seconds": 0.250335924,
  "attempts_per_second": 3668466
}
```

### Resource limits and errors

| Status | Meaning |
|--------|---------|
| `400` | Invalid public key, invalid Base58 target, or combined target longer than four characters |
| `429` | All worker slots are occupied or the per-instance start-rate bucket is empty |
| `504` | The job's time budget expired before finding a match |
| `503` | Work was cancelled |
| `500` | Unexpected worker failure |

Errors have the form `{"error":"description"}`. Clients should use bounded
retries with backoff for `429` and `504`; do not retry in a tight loop.

There is no queue for requests waiting on worker capacity. A job keeps its slot
until its CPU worker actually exits, including when its HTTP handler times out
or is dropped. The worker checks cancellation and its deadline before searching
and every 1,024 attempts. A detected client disconnect cancels the handler and
signals the worker. If a proxy keeps an upstream request open after the client
disconnects, the independent worker deadline still bounds the work.

The start-rate token bucket initially allows a burst of 60 admitted jobs and
refills at one job per second by default. Capacity and rate limits are shared by
all clients of an instance; `/` and `/health` are not subject to them. Multiple
replicas each have their own limits. For per-client fairness or a deployment-wide
quota, also configure limits at a trusted gateway. The application does not trust
`X-Forwarded-For` as client identity. CORS is not an admission or authentication
control. The CLI `grind` command retains its existing behavior.

## Examples

### curl
```bash
curl -X GET 'http://localhost:8080/grind?base=3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd&suffix=omni'
```

### JavaScript
```javascript
const response = await fetch('http://localhost:8080/grind?base=3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd&suffix=omni');
const result = await response.json();
console.log('Address:', result.address);
console.log('Seed:', result.seed);
console.log('Seed bytes:', result.seed_bytes);
```

### Python
```python
import requests
response = requests.get('http://localhost:8080/grind', params={
    'base': '3tJrAXnjofAw8oskbMaSo9oMAYuzdBgVbW3TvQLdMEBd',
    'suffix': 'omni'
})
result = response.json()
print(f"Address: {result['address']}")
print(f"Seed: {result['seed']}")
print(f"Seed bytes: {result['seed_bytes']}")
```

## Deployment

```bash
# Build
cargo build --release --features server

# Run on VPS
./target/release/vanity server --port 8080

# Use process manager (systemd, PM2, Docker)
# Monitor CPU usage - grinding is intensive
```
