# Background helper for Sofa. Runs for as long as Sofa is open.
#
# - Watches every connected controller through the Windows joystick API, which
#   keeps working while a game has focus. The PS button (or Back+Start on other
#   controllers) brings Sofa forward and prints
#   "home <hwnd>\t<title>\t<pid>\t<exe>" for the window that was in front.
# - Keeps Sofa in front like a console home screen (guard on): when the desktop or
#   taskbar takes focus, or a full-screen app that was in front closes, Sofa comes
#   back and "ended <hwnd>" is printed for a closed app.
# - When Steam also answers the PS button and grabs focus, minimizes it again and
#   prints "steamgrab" so Sofa can offer the one-time Steam settings fix.
# - Reads commands on stdin:
#     focus <hwnd> | close <hwnd> | kill <hwnd> | alive <hwnd> ... |
#     guard on|off | allow <ms> | home | chord ps|combo | vol up|down|mute
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File padwatch.ps1 -Sofa <hwnd>
param([Parameter(Mandatory = $true)][long]$Sofa)

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class SofaPad {
  [StructLayout(LayoutKind.Sequential)]
  struct JOYINFOEX {
    public int dwSize, dwFlags, dwXpos, dwYpos, dwZpos, dwRpos, dwUpos, dwVpos,
               dwButtons, dwButtonNumber, dwPOV, dwReserved1, dwReserved2;
  }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  struct JOYCAPS {
    public ushort wMid, wPid;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szPname;
    public int wXmin, wXmax, wYmin, wYmax, wZmin, wZmax, wNumButtons, wPeriodMin, wPeriodMax,
               wRmin, wRmax, wUmin, wUmax, wVmin, wVmax, wCaps, wMaxAxes, wNumAxes, wMaxButtons;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szRegKey;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string szOEMVxD;
  }
  [StructLayout(LayoutKind.Sequential)] struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] struct MONITORINFO { public int cbSize; public RECT rcMonitor, rcWork; public int dwFlags; }

  [DllImport("winmm.dll")] static extern int joyGetPosEx(int id, ref JOYINFOEX info);
  [DllImport("winmm.dll", CharSet = CharSet.Unicode)] static extern int joyGetDevCapsW(IntPtr id, ref JOYCAPS caps, int size);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
  [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO mi);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, int flags, IntPtr extra);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool QueryFullProcessImageName(IntPtr h, int flags, StringBuilder s, ref int size);

  static readonly Stopwatch clock = Stopwatch.StartNew();
  const int JOY_RETURNBUTTONS = 0x80;
  const ushort SONY = 0x054C;
  static IntPtr sofa;
  static uint sofaPid;
  public static volatile bool Guard = true;
  static readonly object outLock = new object();
  static readonly HashSet<string> SHELL = new HashSet<string> {
    "Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "NotifyIconOverflowWindow"
  };

  static void Emit(string line) {
    lock (outLock) { Console.Out.WriteLine(line); Console.Out.Flush(); }
  }

  static string Cls(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  static string Title(IntPtr h) { var sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }

  static string Exe(uint pid) {
    IntPtr p = OpenProcess(0x1000, false, pid); // PROCESS_QUERY_LIMITED_INFORMATION
    if (p == IntPtr.Zero) return "";
    try { var sb = new StringBuilder(1024); int n = 1024; return QueryFullProcessImageName(p, 0, sb, ref n) ? sb.ToString() : ""; }
    finally { CloseHandle(p); }
  }

  static bool IsFullscreen(IntPtr h) {
    RECT r;
    if (!GetWindowRect(h, out r)) return false;
    var mi = new MONITORINFO { cbSize = Marshal.SizeOf(typeof(MONITORINFO)) };
    if (!GetMonitorInfo(MonitorFromWindow(h, 2), ref mi)) return false;
    return r.L <= mi.rcMonitor.L && r.T <= mi.rcMonitor.T && r.R >= mi.rcMonitor.R && r.B >= mi.rcMonitor.B;
  }

  // The desktop, taskbar, Start menu and Windows Search: the parts of Windows that
  // Sofa stands in for while it is running.
  static bool IsShell(IntPtr h) {
    if (h == IntPtr.Zero) return true;
    if (SHELL.Contains(Cls(h))) return true;
    uint pid;
    GetWindowThreadProcessId(h, out pid);
    string exe = System.IO.Path.GetFileName(Exe(pid)).ToLowerInvariant();
    return exe == "searchhost.exe" || exe == "startmenuexperiencehost.exe" || exe == "shellexperiencehost.exe";
  }

  public static void Focus(IntPtr h) {
    if (h == IntPtr.Zero || !IsWindow(h)) return;
    if (IsIconic(h)) ShowWindow(h, 9);
    IntPtr fg = GetForegroundWindow();
    uint ignore;
    uint fgThread = GetWindowThreadProcessId(fg, out ignore);
    uint me = GetCurrentThreadId();
    // Tapping Alt lets a background process take the foreground.
    keybd_event(0x12, 0, 0, IntPtr.Zero);
    keybd_event(0x12, 0, 2, IntPtr.Zero);
    AttachThreadInput(me, fgThread, true);
    SetForegroundWindow(h);
    BringWindowToTop(h);
    AttachThreadInput(me, fgThread, false);
  }

  public static void Close(IntPtr h) {
    if (h != IntPtr.Zero && h != sofa && IsWindow(h)) PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); // WM_CLOSE
  }

  public static void Kill(IntPtr h) {
    if (h == IntPtr.Zero || h == sofa || !IsWindow(h)) return;
    uint pid;
    GetWindowThreadProcessId(h, out pid);
    if (pid == 0 || pid == sofaPid) return;
    string exe = Exe(pid).ToLowerInvariant();
    // Never kill shared Windows hosts; closing their window is the only safe option.
    if (exe.EndsWith("\\explorer.exe") || exe.EndsWith("\\applicationframehost.exe") || exe.EndsWith("\\dwm.exe")) return;
    try { Process.GetProcessById((int)pid).Kill(); } catch { }
  }

  public static void Alive(string line) {
    var live = new List<string>();
    foreach (var s in line.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries)) {
      long v;
      if (long.TryParse(s, out v) && IsWindow((IntPtr)v) && IsWindowVisible((IntPtr)v)) live.Add(s);
    }
    Emit("alive " + string.Join(",", live.ToArray()));
  }

  public static void Key(byte vk) {
    keybd_event(vk, 0, 0, IntPtr.Zero);
    keybd_event(vk, 0, 2, IntPtr.Zero);
  }

  // "ps": the PS button. "combo": Create + Options, which Steam leaves alone.
  public static volatile string Chord = "ps";

  static bool IsHomeChord(ushort mid, int buttons) {
    if (mid == SONY) {
      if (Chord == "combo") return (buttons & 0x300) == 0x300; // Create + Options
      return (buttons & (1 << 12)) != 0;                       // PS button
    }
    return (buttons & 0xC0) == 0xC0;                           // Back + Start on XInput pads
  }

  static bool IsSteam(IntPtr h) {
    if (h == IntPtr.Zero) return false;
    uint pid;
    GetWindowThreadProcessId(h, out pid);
    string exe = Exe(pid).ToLowerInvariant();
    return exe.EndsWith("\\steam.exe") || exe.EndsWith("\\steamwebhelper.exe");
  }

  // Steam also answers the PS button by jumping to the front. For a moment after
  // Sofa takes over, push any Steam window that does that back down.
  static long homeAt = -1;
  // Sofa just launched something; Steam may come forward legitimately until then.
  public static long allowUntil = 0;
  public static void Allow(int ms) { allowUntil = clock.ElapsedMilliseconds + ms; }
  static IntPtr cameFrom = IntPtr.Zero;
  static IntPtr lastLogged = IntPtr.Zero;

  // Small diagnostic log in %TEMP%\sofa-padwatch.log (kept under ~200 KB).
  static readonly string logPath = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "sofa-padwatch.log");
  static void Log(string s) {
    try {
      var fi = new System.IO.FileInfo(logPath);
      if (fi.Exists && fi.Length > 200000) fi.Delete();
      System.IO.File.AppendAllText(logPath, DateTime.Now.ToString("HH:mm:ss.fff") + "  " + s + Environment.NewLine);
    } catch { }
  }
  static string Describe(IntPtr h) {
    if (h == IntPtr.Zero) return "(none)";
    uint pid;
    GetWindowThreadProcessId(h, out pid);
    return (h == sofa ? "[sofa] " : "") + "class=" + Cls(h) + " title=\"" + Title(h) + "\" exe=" + System.IO.Path.GetFileName(Exe(pid)) + " pid=" + pid;
  }

  // Sofa reads a DualSense over HID itself and forwards its Home button here.
  public static void HomeFromSofa() { GoHome(); }

  static void GoHome() {
    IntPtr fg = GetForegroundWindow();
    if (fg == sofa) {
      // Sofa handles the button itself while in front, but Steam still reacts to it.
      cameFrom = sofa;
      homeAt = clock.ElapsedMilliseconds;
      lastLogged = IntPtr.Zero;
      Log("home pressed while Sofa was in front");
      return;
    }
    uint pid = 0;
    string exe = "";
    string title = "";
    if (!IsShell(fg)) {
      GetWindowThreadProcessId(fg, out pid);
      exe = Exe(pid);
      title = Title(fg).Replace("\t", " ");
    } else fg = IntPtr.Zero;
    cameFrom = fg;
    homeAt = clock.ElapsedMilliseconds;
    lastLogged = IntPtr.Zero;
    Log("home pressed, came from " + Describe(fg));
    Focus(sofa);
    Emit("home " + fg.ToInt64() + "\t" + title + "\t" + pid + "\t" + exe);
  }

  static void Poll() {
    var connected = new bool[16];
    var mids = new ushort[16];
    var wasHome = new bool[16];
    int scanTick = 0;
    int guardTick = 0;
    IntPtr session = IntPtr.Zero;      // the full-screen app currently in front
    long sessionSince = 0;
    long shellSince = 0;
    IntPtr prevFg = IntPtr.Zero;

    while (true) {
      // Steam taking focus straight from Sofa, with nothing launched just now, is
      // Steam answering the PS button: send it back under Sofa.
      IntPtr fgNow = GetForegroundWindow();
      if (fgNow != prevFg) {
        if (Guard && prevFg == sofa && fgNow != sofa && clock.ElapsedMilliseconds > allowUntil && IsSteam(fgNow)) {
          Log("steam grabbed focus from Sofa, pushing it down: " + Describe(fgNow));
          ShowWindow(fgNow, 6); // minimize
          Focus(sofa);
          Emit("steamgrab");
          fgNow = sofa;
        }
        prevFg = fgNow;
      }
      if (homeAt >= 0) {
        long since = clock.ElapsedMilliseconds - homeAt;
        if (since > 10000) homeAt = -1; // Big Picture can take several seconds to appear
        else {
          IntPtr fg = GetForegroundWindow();
          if (fg != lastLogged) { lastLogged = fg; Log("fg +" + since + "ms " + Describe(fg)); }
          if (fg != sofa && fg != cameFrom && IsSteam(fg)) {
            Log("steam took focus, pushing it down: " + Describe(fg));
            ShowWindow(fg, 6); // minimize
            Focus(sofa);
            Emit("steamgrab");
          }
        }
      }
      if (scanTick-- <= 0) {
        // Probing disconnected slots is slow, so only rescan every few seconds.
        for (int i = 0; i < 16; i++) {
          var j = new JOYINFOEX { dwSize = Marshal.SizeOf(typeof(JOYINFOEX)), dwFlags = JOY_RETURNBUTTONS };
          connected[i] = joyGetPosEx(i, ref j) == 0;
          if (connected[i]) {
            var caps = new JOYCAPS();
            if (joyGetDevCapsW((IntPtr)i, ref caps, Marshal.SizeOf(typeof(JOYCAPS))) == 0) mids[i] = caps.wMid;
          }
        }
        scanTick = 100;
      }
      for (int i = 0; i < 16; i++) {
        if (!connected[i]) continue;
        var j = new JOYINFOEX { dwSize = Marshal.SizeOf(typeof(JOYINFOEX)), dwFlags = JOY_RETURNBUTTONS };
        if (joyGetPosEx(i, ref j) != 0) { connected[i] = false; continue; }
        bool home = IsHomeChord(mids[i], j.dwButtons);
        if (home && !wasHome[i]) GoHome();
        wasHome[i] = home;
      }

      // Launcher guard, checked about four times a second.
      if (guardTick-- <= 0) {
        guardTick = 8;
        IntPtr fg = GetForegroundWindow();
        long now = clock.ElapsedMilliseconds;
        if (fg != sofa && !IsShell(fg) && fg != session && IsFullscreen(fg)) { session = fg; sessionSince = now; }
        if (session != IntPtr.Zero && !IsWindow(session)) {
          // The full-screen app that was in front has closed: back to Sofa.
          Emit("ended " + session.ToInt64());
          bool wasLong = now - sessionSince > 2000;
          session = IntPtr.Zero;
          if (Guard && wasLong) Focus(sofa);
        }
        if (Guard && fg != sofa && IsShell(fg)) {
          if (shellSince == 0) shellSince = now;
          else if (now - shellSince > 600) { Focus(sofa); shellSince = 0; }
        } else shellSince = 0;
      }
      Thread.Sleep(30);
    }
  }

  public static void Start(long hwnd) {
    sofa = (IntPtr)hwnd;
    GetWindowThreadProcessId(sofa, out sofaPid);
    var t = new Thread(Poll) { IsBackground = true };
    t.Start();
    Emit("ready");
  }
}
'@

[Console]::OutputEncoding = [Text.Encoding]::UTF8
[SofaPad]::Start($Sofa)

while ($null -ne ($line = [Console]::In.ReadLine())) {
  $parts = $line.Trim() -split '\s+'
  switch ($parts[0]) {
    'focus' { [SofaPad]::Focus([IntPtr][long]$parts[1]) }
    'close' { [SofaPad]::Close([IntPtr][long]$parts[1]) }
    'kill' { [SofaPad]::Kill([IntPtr][long]$parts[1]) }
    'alive' { [SofaPad]::Alive($line.Trim().Substring(5)) }
    'guard' { [SofaPad]::Guard = ($parts[1] -eq 'on') }
    'allow' { [SofaPad]::Allow([int]$parts[1]) }
    'home' { [SofaPad]::HomeFromSofa() }
    'chord' { if ($parts[1] -in 'ps', 'combo') { [SofaPad]::Chord = $parts[1] } }
    'vol' {
      switch ($parts[1]) { 'up' { [SofaPad]::Key(0xAF) } 'down' { [SofaPad]::Key(0xAE) } 'mute' { [SofaPad]::Key(0xAD) } }
    }
  }
}
