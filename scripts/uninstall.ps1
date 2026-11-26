# Finds and runs the right uninstaller for a Start-menu app, for Sofa.
#   uninstall.ps1 -Name <app name> -AppId <AppsFolder id> -Check   -> "appx <name>" | "program <name>" | "system" | "notfound"
#   uninstall.ps1 -Name <app name> -AppId <AppsFolder id>          -> "removed" | "started" | "system" | "notfound" | "error <message>"
# Store apps are removed directly (Sofa confirms first). Desktop programs open their
# own uninstaller, which asks again in its own window.
param([Parameter(Mandatory = $true)][string]$Name, [string]$AppId = '', [switch]$Check)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

function Norm([string]$s) { ($s -replace '[^A-Za-z0-9]', '').ToLowerInvariant() }

# Store (packaged) apps: AppId looks like Family_publisher!App
if ($AppId -match '^([^_!\\]+)_([a-z0-9]+)!') {
  $pkgName = $Matches[1]
  $pkg = Get-AppxPackage -Name $pkgName -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $pkg) { 'notfound'; return }
  if ($pkg.NonRemovable -or $pkg.SignatureKind -eq 'System') { 'system'; return }
  if ($Check) { "appx $($pkg.Name)"; return }
  try { Remove-AppxPackage -Package $pkg.PackageFullName; 'removed' } catch { "error $($_.Exception.Message)" }
  return
}

# Desktop programs: match the app to an entry in Windows' installed programs list.
$roots = @(
  'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
  'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
)
$entries = foreach ($r in $roots) {
  Get-ChildItem $r -ErrorAction SilentlyContinue | ForEach-Object {
    $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
    if ($p.DisplayName -and $p.UninstallString -and -not $p.SystemComponent) { $p }
  }
}
$want = Norm $Name
$exeDir = if ($AppId -match '^[A-Za-z]:\\') { (Split-Path $AppId -Parent).ToLowerInvariant() } else { $null }
$best = $entries | ForEach-Object {
  $n = Norm $_.DisplayName
  $score = 0
  if ($n -eq $want) { $score = 100 }
  elseif ($want.Length -ge 4 -and ($n.StartsWith($want) -or $want.StartsWith($n))) { $score = 70 }
  if ($exeDir -and $_.InstallLocation -and $exeDir.StartsWith(($_.InstallLocation.TrimEnd('\')).ToLowerInvariant())) { $score += 50 }
  if ($score -gt 0) { [pscustomobject]@{ Score = $score; Entry = $_ } }
} | Sort-Object Score -Descending | Select-Object -First 1

if (-not $best) { 'notfound'; return }
$e = $best.Entry
if ($Check) { "program $($e.DisplayName)"; return }

$cmd = $e.UninstallString.Trim()
try {
  if ($cmd -match '(?i)msiexec(\.exe)?\s.*(\{[0-9A-F-]+\})') {
    Start-Process msiexec.exe -ArgumentList "/x $($Matches[2])"
  } elseif ($cmd -match '^"([^"]+)"\s*(.*)$') {
    if ($Matches[2]) { Start-Process -FilePath $Matches[1] -ArgumentList $Matches[2] } else { Start-Process -FilePath $Matches[1] }
  } elseif ($cmd -match '^(\S+\.exe)\s*(.*)$') {
    if ($Matches[2]) { Start-Process -FilePath $Matches[1] -ArgumentList $Matches[2] } else { Start-Process -FilePath $Matches[1] }
  } else {
    Start-Process cmd.exe -ArgumentList '/c', $cmd -WindowStyle Hidden
  }
  'started'
} catch { "error $($_.Exception.Message)" }
