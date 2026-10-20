# PostgreSQL local de desarrollo (binarios portables, sin servicio de Windows).
#   powershell -File scripts/db.ps1 start|stop|status
param([ValidateSet("start", "stop", "status")] [string]$Action = "status")

$pg = Join-Path $env:LOCALAPPDATA "Programs\PostgreSQL18"
$bin = Join-Path $pg "pgsql\bin"
$data = Join-Path $pg "data"

if (-not (Test-Path (Join-Path $bin "pg_ctl.exe"))) {
    Write-Error "No se encontró PostgreSQL en $bin"
    exit 1
}

switch ($Action) {
    "start" {
        & (Join-Path $bin "pg_ctl.exe") -D $data -l (Join-Path $data "postgres.log") -o "-p 5432 -c listen_addresses=localhost" -w start
    }
    "stop" {
        & (Join-Path $bin "pg_ctl.exe") -D $data -m fast -w stop
    }
    "status" {
        & (Join-Path $bin "pg_ctl.exe") -D $data status
    }
}
exit $LASTEXITCODE
