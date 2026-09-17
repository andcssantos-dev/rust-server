param(
    [switch]$CheckOnly
)

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
    Write-Host "Aurenfall Server 2 :: Windows preflight"

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host "cargo was not found. Install the pinned Rust toolchain before running preflight."
        exit 1
    }

    if (-not $CheckOnly) {
        Write-Host "Formatting Rust sources..."
        cargo fmt --all
        Assert-ExternalCommandSucceeded "cargo fmt"
    }

    Write-Host "Verifying format..."
    cargo fmt --all -- --check
    Assert-ExternalCommandSucceeded "cargo fmt --check"

    Write-Host "Checking workspace..."
    cargo check --workspace
    Assert-ExternalCommandSucceeded "cargo check"

    Write-Host "Running Clippy..."
    cargo clippy --workspace --all-targets -- -D warnings
    Assert-ExternalCommandSucceeded "cargo clippy"

    Write-Host "Running tests..."
    cargo test --workspace
    Assert-ExternalCommandSucceeded "cargo test"

    Write-Host "Aurenfall Server 2 preflight complete."
}
finally {
    Pop-Location
}
