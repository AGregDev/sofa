<#
.SYNOPSIS
  Sofa's automated QA run. Starts the native app in QA mode (--qa: its own data folder in
  %TEMP%\sofa-qa, nothing launched, nothing outside that folder touched), drives every screen
  with posted keys, checks what the app reports in its log, saves screenshots to shots\, and
  measures start-up time, memory and idle CPU. Writes shots\qa.txt. Exit code 0 = all pass.
.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File qa\qa.ps1
#>
param(
  [string]$Exe = "$env:LOCALAPPDATA\sofa-build\target\release\sofa.exe",
  [string]$Out = (Join-Path $PSScriptRoot '..\shots')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class QaWin {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, IntPtr title);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@

$qaDir = Join-Path $env:TEMP 'sofa-qa'
$log = Join-Path $qaDir 'qa.log'
$json = Join-Path $qaDir 'sofa.json'
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Get-ChildItem $Out -Filter '*.png' -ErrorAction SilentlyContinue | Remove-Item -Force
$results = New-Object System.Collections.Generic.List[string]
$script:fails = 0

function Check([string]$name, [bool]$ok, [string]$detail = '') {
  $mark = if ($ok) { 'PASS' } else { 'FAIL' }
  $line = if ($detail) { "$mark  $name  ($detail)" } else { "$mark  $name" }
  $results.Add($line)
  Write-Host $line
  if (-not $ok) { $script:fails++ }
}

# What a QA run must never touch: the real settings, start-with-Windows, Steam's setting,
# the Start menu shortcut and the Installed apps entry.
function Fingerprint {
  $parts = @()
  $real = Join-Path $env:APPDATA 'sofa\sofa.json'
  $parts += if (Test-Path $real) { 'settings=' + (Get-FileHash $real).Hash } else { 'settings=none' }
  $run = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
  if ($run) { $parts += 'run=' + (($run.PSObject.Properties | Where-Object { $_.Name -notlike 'PS*' } | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ';') }
  $steam = (Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction SilentlyContinue).SteamPath
  if ($steam) {
    $vals = Get-ChildItem (Join-Path $steam 'userdata\*\config\localconfig.vdf') -ErrorAction SilentlyContinue | ForEach-Object {
      $m = Select-String -Path $_.FullName -Pattern '"Controller_CheckGuideButton"\s+"(\d+)"' | Select-Object -First 1
      if ($m) { $m.Matches[0].Groups[1].Value } else { '-' }
    }
    $parts += 'steam=' + ($vals -join ',')
  }
  $parts += 'link=' + (Test-Path (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Sofa.lnk'))
  $parts += 'uninstall=' + (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sofa')
  $parts -join '|'
}

$VK = @{ Left = 0x25; Up = 0x26; Right = 0x27; Down = 0x28; Enter = 0x0D; Esc = 0x1B; Back = 0x08; Home = 0x24;
         E = 0x45; Q = 0x51; O = 0x4F; M = 0x4D; F = 0x46 }

function Key([string]$k, [int]$times = 1, [int]$wait = 260) {
  for ($i = 0; $i -lt $times; $i++) {
    [QaWin]::PostMessage($script:h, 0x0100, [IntPtr]$VK[$k], [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds $wait
  }
}

function Type-Text([string]$s) {
  foreach ($c in $s.ToCharArray()) {
    [QaWin]::PostMessage($script:h, 0x0102, [IntPtr][int]$c, [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds 60
  }
  Start-Sleep -Milliseconds 250
}

function State {
  $line = Get-Content $log -ErrorAction SilentlyContinue | Where-Object { $_ -match 'state\|' } | Select-Object -Last 1
  $s = @{}
  if ($line) {
    foreach ($p in ($line -replace '^.*?state\|', '').Split('|')) {
      $kv = $p.Split('=', 2)
      if ($kv.Count -eq 2) { $s[$kv[0]] = $kv[1] }
    }
  }
  $s
}

function Settings { Get-Content $json -Raw | ConvertFrom-Json }

function Shot([string]$name, [int]$settle = 900) {
  Start-Sleep -Milliseconds $settle
  $r = New-Object QaWin+RECT
  [QaWin]::GetClientRect($script:h, [ref]$r) | Out-Null
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc()
  [QaWin]::PrintWindow($script:h, $hdc, 2) | Out-Null
  $g.ReleaseHdc($hdc); $g.Dispose()
  $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

function Start-Sofa {
  Remove-Item $log -ErrorAction SilentlyContinue
  $p = Start-Process $Exe -ArgumentList '--qa', '--size', '1600x900' -PassThru
  $script:h = [IntPtr]::Zero
  for ($i = 0; $i -lt 60; $i++) {
    if ((Test-Path $log) -and (Select-String -Path $log -Pattern 'ready in' -Quiet)) { break }
    Start-Sleep -Milliseconds 250
  }
  $script:h = [QaWin]::FindWindow('SofaNativeQA', [IntPtr]::Zero)
  $p
}

if (-not (Test-Path $Exe)) { Write-Host "FAIL  build first: $Exe not found"; exit 1 }
$before = Fingerprint
Get-Process sofa -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe } | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $qaDir) { Remove-Item -Recurse -Force $qaDir }

# ---------- First run ----------
$p = Start-Sofa
Check 'S1 starts and opens its window' ($script:h -ne [IntPtr]::Zero)
$ready = Select-String -Path $log -Pattern 'ready in (\d+) ms' | Select-Object -First 1
$ms = if ($ready) { [int]$ready.Matches[0].Groups[1].Value } else { 99999 }
Check 'S2 starts in under 1.5 s' ($ms -lt 1500) "$ms ms to the first frame"
Start-Sleep -Milliseconds 1200
Key Home
$s = State
Check 'W1 first run shows the welcome screen' ($s.layer -eq 'welcome:3') $s.layer
Shot '01-welcome' 600
Key Up
Key Enter
Check 'W2 welcome: button layout cycles' ((Settings).settings.controllerLayout -eq 'ps') ((Settings).settings.controllerLayout)
Key Down
Key Enter
$s = State
Check 'W3 welcome: Continue opens Home' ($s.layer -eq 'none' -and $s.toast -eq 'Sofa is ready') "$($s.layer), toast '$($s.toast)'"
Check 'W4 welcome: setup is saved' ((Settings).setupDone -eq $true)

# ---------- Home ----------
Start-Sleep -Milliseconds 1500
$menu = Join-Path $qaDir 'cache\startmenu.json'
for ($i = 0; $i -lt 40 -and -not (Test-Path $menu); $i++) { Start-Sleep -Milliseconds 250 }
Check 'L1 The Start menu scan runs in the background' (Test-Path $menu) $(if (Test-Path $menu) { '{0} entries' -f ((Get-Content $menu -Raw | ConvertFrom-Json).Count) } else { 'no startmenu.json' })
Start-Sleep -Milliseconds 800
Check 'L2 The scan helper exits when done' (-not (Get-Process sofa -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $p.Id -and $_.Path -eq $Exe }))
$s = State
Check 'H1 Home has rows and the library' ([int]$s.rows -ge 1 -and [int]$s.items -ge 12) "rows $($s.rows), items $($s.items)"
Shot '02-home' 1500
Key Right
$s = State
Check 'H2 Right moves along the row' ($s.focus -eq '0,1') $s.focus
Key Down
$s = State
$rowsHome = [int]$s.rows
Check 'H3 Down moves to the next row' ($rowsHome -lt 2 -or $s.focus.StartsWith('1,')) $s.focus
Shot '03-home-row2'
Key Home
$s = State
Check 'H4 Home goes back to the first tile' ($s.focus -eq '0,0' -and $s.tab -eq 'home') "$($s.tab) $($s.focus)"

# ---------- Tabs ----------
$tabs = @()
for ($i = 0; $i -lt 6; $i++) { Key E 1 450; $tabs += (State).tab; if ((State).tab -eq 'games') { Shot '04-games' } ; if ((State).tab -eq 'social') { Shot '05-social' } ; if ((State).tab -eq 'system') { Shot '05-system' } }
Check 'T1 R1 walks the tabs and wraps' (($tabs -join ',') -eq 'games,media,apps,social,system,home') ($tabs -join ',')
Key Q
Check 'T2 L1 goes back a tab' ((State).tab -eq 'system') (State).tab
Key Esc
Check 'T3 Back returns to Home' ((State).tab -eq 'home') (State).tab

# ---------- Settings ----------
Key O
$s = State
Check 'C1 Options opens Settings' ($s.layer -eq 'sheet:Settings:0') $s.layer
Shot '06-settings' 500
Key Down 7
Check 'C2 Settings: focus moves down the list' ((State).layer -eq 'sheet:Settings:7') (State).layer
Key Enter
Check 'C3 Settings: Reduce motion switches on' ((Settings).settings.motion -eq 'reduced') ((Settings).settings.motion)
Key Enter
Check 'C4 Settings: and off again' ((Settings).settings.motion -eq 'full') ((Settings).settings.motion)
Key Down
Key Enter
Check 'C5 Settings: Show hidden apps' ((Settings).settings.showHidden -eq $true)
Key Enter
Key Up 6
Key Enter
Check 'C6 Settings: Home button choice' ((Settings).settings.homeButton -eq 'combo') ((Settings).settings.homeButton)
Key Enter
Check 'C7 Settings: back to the PS button' ((Settings).settings.homeButton -eq 'ps') ((Settings).settings.homeButton)
Key Esc
Check 'C8 Back closes Settings' ((State).layer -eq 'none') (State).layer

# ---------- Search ----------
Key F
$s = State
Check 'K1 Square opens search' ($s.layer -like 'keyboard:Search:*') $s.layer
Shot '07-search-empty' 400
Type-Text 'a'
$s = State
$hits = [int](($s.layer -split ':')[3])
Check 'K2 Typing finds games and apps' ($hits -gt 0) $s.layer
Shot '08-search' 700
Key Back
Check 'K3 Backspace deletes' ((State).layer -like 'keyboard:Search::*') (State).layer
Key Down
Key Enter
Check 'K4 On-screen keys type' ((State).layer -like 'keyboard:Search:z:*') (State).layer
Key Esc
Check 'K5 Back closes the keyboard' ((State).layer -eq 'none') (State).layer

# ---------- Item options ----------
Key Home
$tile = (State).tile
if ($tile -like 'nav:*' -or $tile -like 'system:*' -or -not $tile) { Key Right; $tile = (State).tile }
Key M
$s = State
Check 'I1 Triangle opens the item menu' ($s.layer -like 'sheet:*:0' -and $s.layer -ne 'sheet:Settings:0') $s.layer
Shot '09-item-menu' 500
Key Enter
$fav = (Settings).overrides.$tile.favorite
Check 'I2 Add to favorites' ($fav -eq $true) $tile
Shot '10-favorites' 1200
Key M
Key Enter
Check 'I3 Remove from favorites' ($null -eq (Settings).overrides.$tile) $tile
Key M
Key Down 3
Key Enter
$s = State
Check 'I4 Rename opens the keyboard with the name' ($s.layer -like 'keyboard:Rename*') $s.layer
Key Back 60 25
Type-Text 'QA Name'
Shot '11-rename' 300
Key Enter
Check 'I5 Rename saves the name' ((Settings).overrides.$tile.name -eq 'QA Name') ((Settings).overrides.$tile.name)
Key M
Key Down 3
Key Enter
Key Back 60 25
Key Enter
Start-Sleep -Milliseconds 400
Check 'I6 An empty name brings the original back' ($null -eq (Settings).overrides.$tile) $tile

# ---------- Launch ----------
Key Home
$tile = (State).tile
Key Enter 1 500
$s = State
Check 'G1 Select shows the launch screen (nothing starts in QA)' ($s.layer -like 'launch:*') $s.layer
Shot '12-launch' 400
Key Esc
Check 'G2 Back leaves the launch screen' ((State).layer -eq 'none') (State).layer
Check 'G3 Launching records Jump back in' ($null -ne (Settings).recent.$tile) $tile

# ---------- Running card ----------
$stand = Start-Process powershell -ArgumentList '-NoProfile', '-WindowStyle', 'Normal', '-Command', "Add-Type -AssemblyName System.Windows.Forms; `$f = New-Object Windows.Forms.Form; `$f.Text = 'Sofa QA window'; `$f.ShowDialog()" -PassThru
$win = [IntPtr]::Zero
for ($i = 0; $i -lt 40 -and $win -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 250; $stand.Refresh(); $win = $stand.MainWindowHandle }
[QaWin]::PostMessage($script:h, 0x8000 + 10, $win, [IntPtr]1) | Out-Null
Start-Sleep -Milliseconds 500
$s = State
Check 'R1 PS from an app adds a Running card first' ($s.running -eq '1' -and $s.tile -eq "run:$([int64]$win)" -and $s.focus -eq '0,0') "$($s.tile) running $($s.running)"
Shot '13-running' 900
Key M
$s = State
Check 'R2 Triangle on Running opens its menu' ($s.layer -like 'sheet:*:0' -and $s.layer -ne 'sheet:Settings:0') $s.layer
Shot '14-running-menu' 400
Key Esc
Stop-Process -Id $stand.Id -Force
Start-Sleep -Milliseconds 400
Key Home
Check 'R3 A closed app loses its card' ((State).running -eq '0') "running $((State).running)"

# ---------- One Sofa at a time ----------
$second = Start-Process $Exe -ArgumentList '--qa', '--size', '1600x900' -PassThru
$gone = $second.WaitForExit(4000)
Check 'O1 A second start hands over to the first' ($gone -and -not $p.HasExited)

# ---------- Memory and CPU ----------
Key Home
Start-Sleep -Seconds 3
$p.Refresh()
$ws = [math]::Round($p.WorkingSet64 / 1MB, 1); $priv = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
Check 'P1 Memory on Home under 150 MB' ($ws -lt 150) "working set $ws MB, private $priv MB"
$cpu0 = $p.TotalProcessorTime.TotalMilliseconds
Start-Sleep -Seconds 5
$p.Refresh()
$cpu = [math]::Round(($p.TotalProcessorTime.TotalMilliseconds - $cpu0) / 5000 * 100, 2)
Check 'P2 Idle CPU under 1% of a core' ($cpu -lt 1.0) "$cpu %"
Check 'P3 Threads stay few' ($p.Threads.Count -le 24) "$($p.Threads.Count) threads"
[QaWin]::PostMessage($script:h, 0x8000 + 9, [IntPtr]0, [IntPtr]0) | Out-Null
Start-Sleep -Seconds 2
$p.Refresh()
$wsTrim = [math]::Round($p.WorkingSet64 / 1MB, 1)
Check 'P4 Memory is given back in the background' ($wsTrim -lt $ws) "working set $ws MB -> $wsTrim MB"
Key Right
Key Left
Shot '15-after-trim' 1500
Check 'P5 Draws again after giving memory back' (-not $p.HasExited -and (State).focus -eq '0,0')

# ---------- Quit ----------
Key O
Key Down 11
Check 'Q1 Quit Sofa is the last setting' ((State).layer -eq 'sheet:Settings:11') (State).layer
Key Enter
$quit = $p.WaitForExit(4000)
Check 'Q2 Quit Sofa closes it' $quit

# ---------- Second start ----------
$p = Start-Sofa
Start-Sleep -Milliseconds 800
Key Home
$s = State
Check 'Q3 No welcome once set up' ($s.layer -eq 'none') $s.layer
Check 'Q4 Settings were kept' ((Settings).settings.controllerLayout -eq 'ps')
[QaWin]::PostMessage($script:h, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null
Check 'Q5 Closing the window quits' ($p.WaitForExit(4000))

# ---------- The real world ----------
$after = Fingerprint
Check 'X1 QA touched nothing outside its folder' ($before -eq $after) $(if ($before -ne $after) { "$before -> $after" } else { '' })
$errors = Select-String -Path $log -Pattern 'panic|graphics failed' -SimpleMatch:$false
Check 'X2 No errors in the log' (-not $errors) ($errors | Select-Object -First 1)

$summary = "{0} checks, {1} failed" -f $results.Count, $script:fails
$results.Add($summary)
$results | Set-Content -Encoding utf8 (Join-Path $Out 'qa.txt')
Write-Host $summary
if ($script:fails -gt 0) { exit 1 } else { exit 0 }
