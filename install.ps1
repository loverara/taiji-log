$ErrorActionPreference = "Stop"

$Repo = if ($env:TAIJI_LOG_REPO) { $env:TAIJI_LOG_REPO } else { "loverara/taiji-log" }
$Version = if ($env:TAIJI_LOG_VERSION) { $env:TAIJI_LOG_VERSION } else { "" }
$InstallDir = if ($env:TAIJI_LOG_INSTALL_DIR) { $env:TAIJI_LOG_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\\taiji-log\\bin" }

for ($i = 0; $i -lt $args.Length; $i++) {
  switch ($args[$i]) {
    "--repo" { $Repo = $args[$i + 1]; $i++ }
    "--version" { $Version = $args[$i + 1]; $i++ }
    "--to" { $InstallDir = $args[$i + 1]; $i++ }
    default { throw "Unknown argument: $($args[$i])" }
  }
}

$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -eq "AMD64") { $ArchId = "x86_64" }
elseif ($Arch -eq "ARM64") { $ArchId = "aarch64" }
else { throw "Unsupported arch: $Arch" }

$Target = "$ArchId-pc-windows-msvc"
$Asset = "taiji-log-$Target.zip"

if ($Version) { $BaseUrl = "https://github.com/$Repo/releases/download/$Version" }
else { $BaseUrl = "https://github.com/$Repo/releases/latest/download" }

$TempDir = Join-Path $env:TEMP ("taiji-log-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

$ArchivePath = Join-Path $TempDir $Asset
$SumsPath = Join-Path $TempDir "sha256sums.txt"

Invoke-WebRequest -Uri "$BaseUrl/$Asset" -OutFile $ArchivePath -UseBasicParsing
Invoke-WebRequest -Uri "$BaseUrl/sha256sums.txt" -OutFile $SumsPath -UseBasicParsing

$Expected = (Select-String -Path $SumsPath -Pattern ("  " + [Regex]::Escape($Asset) + "$") | Select-Object -First 1).Line.Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)[0]
if (-not $Expected) { throw "Checksum for $Asset not found in sha256sums.txt" }

$Actual = (Get-FileHash -Algorithm SHA256 -Path $ArchivePath).Hash.ToLowerInvariant()
if ($Expected.ToLowerInvariant() -ne $Actual) {
  throw "Checksum mismatch for $Asset`nexpected: $Expected`nactual:   $Actual"
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Expand-Archive -Path $ArchivePath -DestinationPath $TempDir -Force

$ExeSrc = Join-Path $TempDir "taiji-log.exe"
$ExeDst = Join-Path $InstallDir "taiji-log.exe"
Move-Item -Force -Path $ExeSrc -Destination $ExeDst

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
  [Environment]::SetEnvironmentVariable("Path", $UserPath + ";" + $InstallDir, "User")
  Write-Output "Installed to $ExeDst"
  Write-Output "Restart your terminal to use: taiji-log"
} else {
  Write-Output "Installed: taiji-log"
}
