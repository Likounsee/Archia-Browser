param(
    [Parameter(Position = 0)]
    [string]$Url
)

$ErrorActionPreference = "Stop"
$RepositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepositoryRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Rust/Cargo est introuvable. Installe Rust depuis https://rustup.rs puis relance ce script."
    exit 1
}

Write-Host "Demarrage d'Archia Browser depuis $RepositoryRoot" -ForegroundColor Cyan
if ([string]::IsNullOrWhiteSpace($Url)) {
    cargo run --locked -p archia-browser
}
else {
    cargo run --locked -p archia-browser -- $Url
}

exit $LASTEXITCODE
