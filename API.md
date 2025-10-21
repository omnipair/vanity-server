# Vanity Server API

A blazingly fast REST API for grinding Solana vanity addresses synchronously.

## Quick Start

```bash
# Build and start server
cargo build --release --features server
cargo run --release --features server -- server

# Configure via .env file
cp env.example .env
# Edit .env with your settings

# Grind addresses
curl -X GET http://localhost:8080/grind
```

## Configuration

All parameters are configured via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `VANITY_PORT` | Server port | `8080` |
| `VANITY_DEFAULT_BASE` | Base pubkey (required) | - |
| `VANITY_DEFAULT_OWNER` | Owner pubkey (required) | - |
| `VANITY_DEFAULT_PREFIX` | Target prefix | - |
| `VANITY_DEFAULT_SUFFIX` | Target suffix | - |
| `VANITY_DEFAULT_CPUS` | CPU threads (0=auto) | `0` |
| `VANITY_DEFAULT_CASE_INSENSITIVE` | Case insensitive | `false` |

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | API documentation |
| `/health` | GET | Health check |
| `/grind` | GET | Grind vanity addresses (synchronous) |

### Grind Vanity Addresses
**GET** `/grind`

Returns vanity address result immediately using environment variable configuration.

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

## Examples

### curl
```bash
curl -X GET http://localhost:8080/grind
```

### JavaScript
```javascript
const response = await fetch('http://localhost:8080/grind');
const result = await response.json();
console.log('Address:', result.address);
console.log('Seed:', result.seed);
console.log('Seed bytes:', result.seed_bytes);
```

### Python
```python
import requests
response = requests.get('http://localhost:8080/grind')
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
