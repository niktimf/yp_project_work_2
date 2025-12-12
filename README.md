# Quote Streaming System

A client-server system for streaming stock quotes over TCP/UDP.

## Project Structure

```
.
├── server/     # Quote Server (generator)
├── client/     # Quote Client
└── common/     # Shared types and protocol
```

## Build

```bash
cargo build --release
```

## Running

### Server

```bash
cargo run --release -p server
```

The server starts:
- TCP server on port 5000 (for commands)
- UDP listener on port 5001 (for ping)

### Client

```bash
cargo run --release -p client -- [OPTIONS]
```

Options:
- `-s, --server-addr <ADDR>` — TCP server address (default: `127.0.0.1:5000`)
- `-p, --ping-port <PORT>` — ping destination port (default: `5001`)
- `-u, --udp-port <PORT>` — local UDP port for receiving data (default: `34254`)
- `-c, --client-ip <IP>` — client IP for receiving data (default: `127.0.0.1`)
- `-t, --tickers-file <FILE>` — path to tickers file (default: `tickers.txt`)

Example:
```bash
cargo run --release -p client -- -s 127.0.0.1:5000 -t tickers.txt
```

## Docker

### Build

```bash
docker build -t quote-server .
```

### Run

```bash
docker run -p 5000:5000/tcp -p 5001:5001/udp quote-server
```

### Connect client to Docker server

When server runs in Docker, use host's Docker bridge IP for receiving data:

```bash
# Find Docker bridge IP (usually 172.17.0.1)
ip addr show docker0

# Run client with host IP
cargo run --release -p client -- -s 127.0.0.1:5000 -c 172.17.0.1 -t tickers.txt
```

Or use `--network host` for the container:

```bash
docker run --network host quote-server
cargo run --release -p client -- -t tickers.txt
```

## Protocol

### STREAM Command

```
STREAM udp://<ip>:<port> <TICKER1,TICKER2,...>
```

Example: `STREAM udp://127.0.0.1:34254 AAPL,TSLA,GOOGL`

### Server Responses

- `OK` — command accepted
- `ERR <message>` — error

### Quote Format (JSON)

```json
{"ticker":"AAPL","price":"285.50","volume":3500,"timestamp":1702300000000}
```

### Keep-Alive (Ping/Pong)

- Client sends `PING` every 2 seconds to server's UDP port
- Server responds with `PONG`
- Server stops streaming if no ping received for 5 seconds

## Tickers File

Format of `tickers.txt`:
```
AAPL
GOOGL
TSLA
```

Empty lines and lines starting with `#` are ignored.

## Testing

```bash
cargo test --all
```

## Architecture

### Server

1. **Quote Generator** — separate thread, generates data for all tickers
2. **TCP Server** — accepts commands from clients
3. **UDP Ping Listener** — handles ping from clients
4. **Cleanup Thread** — removes inactive clients
5. **Streamers** — separate thread per client for UDP data delivery

### Client

1. **TCP Connection** — sends STREAM command
2. **UDP Receiver** — receives quotes
3. **Ping Thread** — sends keep-alive messages

## Graceful Shutdown

- Server and client handle Ctrl+C properly
- All threads terminate, resources are released
