$ErrorActionPreference = "Stop"
$env:RUST_LOG = if ($env:RUST_LOG) { $env:RUST_LOG } else { "info" }

cargo run -p aurenfall-protocol-client -- 127.0.0.1:7777 config/tls/dev-cert.der localhost
