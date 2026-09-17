$ErrorActionPreference = "Stop"

function Assert-ExternalCommandSucceeded {
    param([Parameter(Mandatory = $true)][string]$Step)

    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed with exit code $LASTEXITCODE"
    }
}

$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $RepositoryRoot

try {
    Write-Host "Aurenfall Server 2 :: Windows bootstrap"

    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Write-Host "rustup was not found. Install Rust from https://rustup.rs and re-run this script."
        exit 1
    }

    rustup toolchain install 1.98.0
    Assert-ExternalCommandSucceeded "Rust toolchain install"
    rustup component add --toolchain 1.98.0 clippy rustfmt
    Assert-ExternalCommandSucceeded "Rust component installation"

    if (-not (Test-Path "config/tls/dev-cert.der") -or -not (Test-Path "config/tls/dev-key.der")) {
        Write-Host "Generating persistent local TLS identity..."
        cargo run -p aurenfall-dev-certgen -- config/tls
        Assert-ExternalCommandSucceeded "Development TLS identity generation"
    }

    & (Join-Path $PSScriptRoot "preflight.ps1")

    Write-Host "Validating GameData..."
    cargo run -p aurenfall-gamedata-compiler -- gamedata/manifest.yaml
    Assert-ExternalCommandSucceeded "GameData validation"

    Write-Host "Aurenfall Server 2 bootstrap complete."
}
finally {
    Pop-Location
}
