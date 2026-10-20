# Builds a release and installs it to C:\dev\tool\pmanager: both executables
# plus the strip profiles beside them.

$ErrorActionPreference = 'Stop'

$Destination = 'C:\dev\tool\pmanager'

cargo build --release --manifest-path "$PSScriptRoot\Cargo.toml"
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed with exit code $LASTEXITCODE; nothing was copied"
}

New-Item -ItemType Directory -Force -Path "$Destination\strip" | Out-Null

foreach ($exe in 'pman.exe', 'pmanager.exe') {
    Copy-Item -Path "$PSScriptRoot\target\release\$exe" -Destination $Destination -Force
}
Copy-Item -Path "$PSScriptRoot\strip\*" -Destination "$Destination\strip" -Force

Write-Host "Installed pman.exe, pmanager.exe and the strip profiles to $Destination"
