# Rust Security Rules

## Unsafe Code

Minimize `unsafe` blocks. Every `unsafe` block MUST have a `// SAFETY:` comment that explains exactly which invariants uphold the safety requirement and why those invariants hold at that call site.

```rust
// SAFETY: `ptr` was obtained from `Box::into_raw` on line 42 and has not been aliased.
let val = unsafe { Box::from_raw(ptr) };
```

Never use `unsafe` to work around a borrow-checker error — that is always a design smell indicating the ownership model needs rethinking, not silencing. `cargo clippy -- -D warnings` catches many unsafe misuses; trust it and fix the root cause.

## Secrets and Credentials

Never hardcode API keys, passwords, database credentials, or tokens in source code or configuration files checked into the repository. Read secrets from environment variables at startup:

```rust
let api_key = std::env::var("SPARROW_API_KEY")
    .expect("SPARROW_API_KEY must be set");  // fail fast at startup, not mid-request
```

Validate that all required secrets are present at process startup so the service fails fast with a clear message rather than failing later with an obscure error. `.env` files used for local development must be listed in `.gitignore`.

## Input Validation — Parse, Don't Validate

Convert untyped external input into strongly typed values at the system boundary. A type that can only hold valid values is safer and clearer than a validated `String` that could be passed around un-checked.

```rust
// Prefer: parse into a type that enforces validity
let port: u16 = raw_port.parse().map_err(|_| ConfigError::InvalidPort(raw_port))?;

// Avoid: validate and then carry around the raw string
if raw_port.parse::<u16>().is_err() { return Err(...) }
use_port(&raw_port);  // still a String; nothing enforces it was validated
```

## Dependency Hygiene

Before each release:

- `cargo audit` — check all dependencies against the RustSec advisory database for known CVEs.
- `cargo deny check` — enforce license compliance and ban known problematic crates (configured in `deny.toml`).
- `cargo tree --duplicates` — identify diamond dependency conflicts that may introduce stale or vulnerable versions.

## Error Messages Exposed to Clients

Never expose internal error details, stack traces, or storage internals in HTTP API responses. Log the full context server-side using `tracing` (with `tracing::error!` or `tracing::warn!`), then return a generic, non-revealing message to the client.

```rust
tracing::error!(error = %e, node_id = %id, "lmdb read failed");
return Err(ApiError::InternalServerError);  // client sees only a status code
```

## Command Injection

Never build shell commands by concatenating or interpolating user-supplied input into a command string. Always use `tokio::process::Command` with explicit `.arg()` calls:

```rust
// WRONG — user_input could be "foo; rm -rf /"
Command::new("/bin/sh").args(&["-c", user_input]).spawn()?;

// CORRECT — each argument is passed as a separate token, never interpreted by a shell
tokio::process::Command::new("sparrow-tool")
    .arg("--input")
    .arg(user_input)  // never joined into a shell string
    .spawn().await?;
```

## Path Traversal

Canonicalize file paths before using them and verify the resolved path is within the intended root directory. Reject paths that escape the root:

```rust
let resolved = path.canonicalize()?;
if !resolved.starts_with(&allowed_root) {
    return Err(AccessError::PathTraversal(path.to_owned()));
}
```
