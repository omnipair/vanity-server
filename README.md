# `vanity`

A *bLaZinGlY fAsT* tool for grinding vanity addresses on Solana.

## 1) What

Typically, solana developers wishing to obtain a vanity address for their program or token grind out ed25519 keypairs and sign off on a `SystemInstruction::CreateAccount` instruction. However, by using `SystemInstruction::CreateAccountWithSeed`, developers can bypass ed25519 and get extreme speedups on address searches. Although not as generic, this method covers many use cases.

## 2) H

By default, vanity compiles for cpu. Install via

```bash
cargo install vanity
```

To compile for gpu, install via

```bash
cargo install vanity --features=gpu
```
If you don't have a GPU, consider using [vast.ai](https://cloud.vast.ai/?ref_id=126830). Pls use this referral link so that I can keep using GPUs.


Refer to the help via `vanity --help` for information on usage.

```bash

Usage: vanity [OPTIONS] --base <BASE> --owner <OWNER> --target <TARGET>

Options:
      --base <BASE>          The pubkey that will be the signer for the CreateAccountWithSeed instruction
      --owner <OWNER>        The account owner, e.g. BPFLoaderUpgradeab1e11111111111111111111111 or TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
      --target <TARGET>      The target prefix for the pubkey
      --case-insensitive     Whether user cares about the case of the pubkey
      --logfile <LOGFILE>    Optional log file
      --num-cpus <NUM_CPUS>  Number of cpu threads to use for mining [default: 0]
  -h, --help                 Print help
```

To actually make use of the resulting seed, refer to the `solana_program` docs:

```rust
pub fn create_account_with_seed(
    from_pubkey: &Pubkey,
    // this is the resulting address, obtained via Pubkey::create_with_seed
    to_pubkey: &Pubkey, 
    base: &Pubkey,
    seed: &str,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
) -> Instruction
```

## HTTP Server Mode

The vanity tool can also run as an HTTP server to provide a REST API for grinding vanity addresses.

### Build and Run Server

```bash
# Build with server feature
cargo build --release --features server

# Start server on default port 8080
cargo run --release --features server -- server

# Start server on custom port
cargo run --release --features server -- server --port 3000
```

### API Usage

**Get API documentation:**
```bash
curl -X GET http://localhost:8080/
```

**Grind vanity addresses (synchronous):**
```bash
curl -X GET http://localhost:8080/grind
```

**Health check:**
```bash
curl -X GET http://localhost:8080/health
```

See [API.md](API.md) for complete API documentation with examples in multiple programming languages.

### Environment Configuration

The server supports configuration via environment variables. Copy `.env.example` to `.env` and modify as needed:

```bash
cp .env.example .env
# Edit .env file with your settings
```

Available environment variables:
- `VANITY_PORT` - Server port (default: 8080)
- Additional variables documented in `.env.example`

## Contributions

yes

## Performance

**Update@06/27/25:** Ran a 14x RTX 4090 setup and it did 1.2-1.3 billion searches per second per GPU

## Acknowledgements, External Libraries

- The sha2 implementation used in this library is taken from [here](https://github.com/mochimodev/cuda-hashing-algos), which is in the public domain.
- The base58 encoding implementation is taken from firedancer with heavy modifications for use in cuda & case insensitive encodings, licensed under APACHE-2.0
