param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\casciit\bin")
)

$ErrorActionPreference = "Stop"
$BinaryName = "casciit.exe"
$Source = Join-Path $PSScriptRoot "target\release\cascii-terminal-decorator.exe"
$Destination = Join-Path $InstallDir $BinaryName

Write-Host "Building cascii-terminal-decorator (release)..."
cargo build --release

if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
    throw "Build artifact not found at $Source"
}

Write-Host "Installing $BinaryName to $InstallDir..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -LiteralPath $Source -Destination $Destination -Force

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$PathEntries = @($UserPath -split ";" | Where-Object { $_ })
if (-not ($PathEntries | Where-Object { $_.TrimEnd("\") -ieq $InstallDir.TrimEnd("\") })) {
    $UpdatedPath = (@($PathEntries) + $InstallDir) -join ";"
    [Environment]::SetEnvironmentVariable("Path", $UpdatedPath, "User")
    $env:Path = "$InstallDir;$env:Path"
    Write-Host "Added $InstallDir to your user PATH."
}

& $Destination config init

Write-Host "Done. Open a new terminal and run: casciit C:\path\to\frames"
