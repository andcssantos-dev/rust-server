$ErrorActionPreference = "Stop"

$previousHydrologyProof = $env:AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER

try {
    $env:AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER = "1"

    Write-Host "Starting Aurenfall Server in deterministic WATER-bearing development proof mode..."
    Write-Host "Authority remains server-owned; the Rust server selects the WATER-bearing frontier and safe spawn."

    & "$PSScriptRoot\run-server.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    if ($null -eq $previousHydrologyProof) {
        Remove-Item Env:AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER -ErrorAction SilentlyContinue
    }
    else {
        $env:AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER = $previousHydrologyProof
    }
}
