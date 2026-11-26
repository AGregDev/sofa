# QA for the background helper (padwatch.ps1) on the real Windows desktop.
#
# Opens two small test windows, one standing in for Sofa and one for a game, then
# drives the helper the way Sofa does and checks what it reports and does:
#   W1  the helper starts and says "ready"
#   W2  "home" from a game brings the Sofa window to the front and reports the game
#   W3  "alive" reports the game window while it is open
#   W4  "close" closes the game window, and "alive" stops reporting it
#   W5  "focus" brings a window back to the front (Resume)
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\qa-helper.ps1
# Close Sofa first: its own helper would also react to the test windows.
$ErrorActionPreference = 'Stop'
$results = New-Object System.Collections.Generic.List[string]
function Check($id, $name, $ok, $detail = '') {
  $line = '{0}  {1}  {2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $id, $name
  if ($detail) { $line += "  ($detail)" }
  $results.Add($line)
}
function Skip($id, $name, $why) { $results.Add("SKIP  $id  $name  ($why)") }

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class QaWin {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
}
'@

function Start-TestWindow($title) {
  $code = "Add-Type -AssemblyName System.Windows.Forms; `$f = New-Object Windows.Forms.Form; `$f.Text = '$title'; `$f.Width = 640; `$f.Height = 360; [Windows.Forms.Application]::Run(`$f)"
  $p = Start-Process powershell.exe -ArgumentList '-NoProfile', '-Command', $code -PassThru -WindowStyle Normal
  for ($i = 0; $i -lt 100 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
  return $p
}

function Read-Line($proc, $timeoutMs = 4000) {
  $task = $proc.StandardOutput.ReadLineAsync()
  if ($task.Wait($timeoutMs)) { return $task.Result }
  return $null
}

$sofa = Start-TestWindow 'QA Sofa'
$game = $null
$helper = $null
try {
  $psi = New-Object Diagnostics.ProcessStartInfo 'powershell.exe'
  $psi.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSScriptRoot\padwatch.ps1`" -Sofa $($sofa.MainWindowHandle.ToInt64())"
  $psi.UseShellExecute = $false
  $psi.RedirectStandardInput = $true
  $psi.RedirectStandardOutput = $true
  $psi.CreateNoWindow = $true
  $helper = [Diagnostics.Process]::Start($psi)
  $first = Read-Line $helper 15000
  Check 'W1' 'Helper starts and says ready' ($first -eq 'ready') $first
  $helper.StandardInput.WriteLine('guard off')

  $game = Start-TestWindow 'QA Game'
  Start-Sleep -Milliseconds 800
  $gameHwnd = $game.MainWindowHandle.ToInt64()
  $helper.StandardInput.WriteLine("focus $gameHwnd")
  Start-Sleep -Milliseconds 600
  # With the screen off or the session inactive, Windows gives no window the focus at
  # all. The focus checks can't mean anything then, so they are skipped, not passed.
  $active = [QaWin]::GetForegroundWindow().ToInt64() -ne 0
  if ($active) {
    $helper.StandardInput.WriteLine('home')
    $line = Read-Line $helper
    Start-Sleep -Milliseconds 400
    $fg = [QaWin]::GetForegroundWindow().ToInt64()
    $reported = $line -and $line.StartsWith("home $gameHwnd`t") -and $line.Contains('QA Game')
    Check 'W2' 'Home from a game brings Sofa forward and reports the game' ($reported -and $fg -eq $sofa.MainWindowHandle.ToInt64()) "reported=$reported foreground-is-sofa=$($fg -eq $sofa.MainWindowHandle.ToInt64())"
  } else { Skip 'W2' 'Home from a game brings Sofa forward and reports the game' 'desktop inactive: no window can take focus' }

  $helper.StandardInput.WriteLine("alive $gameHwnd")
  $line = Read-Line $helper
  Check 'W3' 'Alive reports the open game window' ($line -eq "alive $gameHwnd") $line

  if ($active) {
    $helper.StandardInput.WriteLine("focus $gameHwnd")
    Start-Sleep -Milliseconds 600
    $fg = [QaWin]::GetForegroundWindow().ToInt64()
    Check 'W5' 'Focus brings the game back to the front (Resume)' ($fg -eq $gameHwnd)
  } else { Skip 'W5' 'Focus brings the game back to the front (Resume)' 'desktop inactive: no window can take focus' }

  $helper.StandardInput.WriteLine("close $gameHwnd")
  Start-Sleep -Milliseconds 1200
  $helper.StandardInput.WriteLine("alive $gameHwnd")
  $line = Read-Line $helper
  Check 'W4' 'Close closes the game and alive stops reporting it' ((-not [QaWin]::IsWindow([IntPtr]$gameHwnd)) -and $line -eq 'alive ') $line
} catch {
  $results.Add("FAIL  X  Helper QA crashed: $_")
} finally {
  if ($helper -and -not $helper.HasExited) { $helper.Kill() }
  foreach ($p in @($sofa, $game)) { if ($p -and -not $p.HasExited) { $p.Kill() } }
}

$failed = @($results | Where-Object { $_.StartsWith('FAIL') }).Count
$skipped = @($results | Where-Object { $_.StartsWith('SKIP') }).Count
$passed = @($results | Where-Object { $_.StartsWith('PASS') }).Count
$results.Add('')
$results.Add("$passed passed, $failed failed, $skipped skipped")
$results | ForEach-Object { Write-Output $_ }
if ($failed) { exit 1 }
