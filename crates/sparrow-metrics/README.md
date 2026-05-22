# sparrow-metrics

Optional anonymous telemetry for SparrowDB. Collects aggregate usage events (command invocations, instance starts) and sends them asynchronously to `https://logs.helix-db.com/v2`. All collection is opt-out and no query data or schema contents are ever sent.

## Build

```bash
cargo build -p sparrow-metrics
```

## Test

```bash
cargo test -p sparrow-metrics
```

## Configuration

Telemetry is controlled by the `~/.sparrow/credentials` file:

```
metrics=true     # enable (default)
metrics=false    # disable
```

Manage via the CLI:

```bash
sparrow metrics off      # disable
sparrow metrics on       # enable
sparrow metrics status   # show current setting
```

## Key items

| Item | Description |
|---|---|
| `METRICS_ENABLED` | `LazyLock<bool>` — reads `metrics` from credentials file |
| `SPARROW_USER_ID` | `LazyLock<&str>` — anonymous per-install identifier from credentials file |
| `METRICS_URL` | Endpoint constant (`https://logs.helix-db.com/v2`) |
| `events` module | Event type definitions serialised and sent to the endpoint |
