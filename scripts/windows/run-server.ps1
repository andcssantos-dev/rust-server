$ErrorActionPreference = "Stop"
$env:RUST_LOG = if ($env:RUST_LOG) { $env:RUST_LOG } else { "info" }

cargo run -p aurenfall-world-server -- config/server.toml
