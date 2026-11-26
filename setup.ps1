# Builds Sofa and installs it for the current Windows user (no admin needed).
#
#   powershell -ExecutionPolicy Bypass -File setup.ps1             build + install
#   powershell -ExecutionPolicy Bypass -File setup.ps1 -BuildOnly  build the installer only
#
# The installer adds Start menu and desktop shortcuts and starts Sofa. On first
# launch Sofa walks through its own setup: controller check, library scan,
# start-with-Windows, and links to the Windows settings worth changing.
param([switch]$BuildOnly)
$ErrorActionPreference = 'Stop'

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
  throw 'Node.js 20 or newer is required. Install it from https://nodejs.org and run this again.'
}

# The NSIS compiler cannot open files past 260 characters, so the build runs in
# a short folder under %TEMP%, mirrored from this one.
$work = Join-Path $env:TEMP 'sofa-build'
Write-Host "Copying the project to $work"
robocopy $PSScriptRoot $work /MIR /XD (Join-Path $PSScriptRoot 'dist') (Join-Path $PSScriptRoot 'shots') (Join-Path $work 'dist') /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Copy failed (robocopy $LASTEXITCODE)" }

Push-Location $work
try {
  Write-Host 'Installing dependencies'
  npm install --no-fund --no-audit
  if ($LASTEXITCODE) { throw 'npm install failed' }

  Write-Host 'Building the installer'
  npm run dist
  if ($LASTEXITCODE) { throw 'Build failed' }
} finally {
  Pop-Location
}

$installer = Get-ChildItem (Join-Path $work 'dist') -Filter 'Sofa Setup *.exe' |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { throw 'Installer not found' }
$dist = Join-Path $PSScriptRoot 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item $installer.FullName $dist -Force
$out = Join-Path $dist $installer.Name
Write-Host "Built $out"
if ($BuildOnly) { return }

Write-Host "Running $($installer.Name)"
Start-Process $out -Wait
Write-Host 'Sofa is installed. Find it in the Start menu, or press the PS button once it is running.'
