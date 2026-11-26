# Lists playback devices or sets the Windows default one, for Sofa's sound switcher.
#   audio.ps1 -List          -> JSON [{ id, name, default }]
#   audio.ps1 -Set <id>      -> makes that device the default for games, media and calls
#   audio.ps1 -Mic get|toggle -> "muted" / "unmuted" / "none" for the default microphone
param([switch]$List, [string]$Set, [string]$Mic)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class SofaAudio {
  [StructLayout(LayoutKind.Sequential)] public struct PROPERTYKEY { public Guid fmtid; public int pid; }
  [StructLayout(LayoutKind.Sequential)] public struct PROPVARIANT { public ushort vt; public ushort r1, r2, r3; public IntPtr p; public IntPtr p2; }

  [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator { }

  [Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDeviceEnumerator {
    int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
    int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice endpoint);
  }
  [Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDeviceCollection {
    int GetCount(out int count);
    int Item(int index, out IMMDevice device);
  }
  [Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDevice {
    int Activate(ref Guid iid, int ctx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object o);
    int OpenPropertyStore(int access, out IPropertyStore props);
    int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  }
  [Guid("886d8eeb-8cf2-4446-8d02-cdba1dbdcf99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IPropertyStore {
    int GetCount(out int count);
    int GetAt(int i, out PROPERTYKEY key);
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT value);
  }
  // Undocumented but long-stable interface Windows itself uses to change the default device.
  [ComImport, Guid("870af99c-171d-4f9e-af0d-e63df40c2bc9")] class CPolicyConfigClient { }
  [Guid("f8679f50-850a-41cf-9c72-430f290290c8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IPolicyConfig {
    int GetMixFormat(); int GetDeviceFormat(); int ResetDeviceFormat(); int SetDeviceFormat();
    int GetProcessingPeriod(); int SetProcessingPeriod(); int GetShareMode(); int SetShareMode();
    int GetPropertyValue(); int SetPropertyValue();
    int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string id, int role);
  }

  [Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IAudioEndpointVolume {
    int RegisterControlChangeNotify(); int UnregisterControlChangeNotify(); int GetChannelCount();
    int SetMasterVolumeLevel(); int SetMasterVolumeLevelScalar(); int GetMasterVolumeLevel();
    int GetMasterVolumeLevelScalar(); int SetChannelVolumeLevel(); int SetChannelVolumeLevelScalar();
    int GetChannelVolumeLevel(); int GetChannelVolumeLevelScalar();
    int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid ctx);
    int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
  }

  // The default microphone's mute switch: "get" or "toggle". Returns "muted" / "unmuted" / "none".
  public static string Mic(string mode) {
    var e = (IMMDeviceEnumerator)new MMDeviceEnumerator();
    IMMDevice mic;
    if (e.GetDefaultAudioEndpoint(1, 0, out mic) != 0 || mic == null) return "none";
    var iid = new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");
    object o;
    mic.Activate(ref iid, 23, IntPtr.Zero, out o);
    var vol = (IAudioEndpointVolume)o;
    bool muted;
    vol.GetMute(out muted);
    if (mode == "toggle") { var ctx = Guid.Empty; vol.SetMute(!muted, ref ctx); muted = !muted; }
    return muted ? "muted" : "unmuted";
  }

  static string Esc(string s) { return (s ?? "").Replace("\\", "\\\\").Replace("\"", "\\\""); }

  public static string List() {
    var e = (IMMDeviceEnumerator)new MMDeviceEnumerator();
    IMMDevice def; string defId = "";
    if (e.GetDefaultAudioEndpoint(0, 1, out def) == 0) def.GetId(out defId);
    IMMDeviceCollection col;
    e.EnumAudioEndpoints(0, 1, out col); // render devices that are active
    int n; col.GetCount(out n);
    var key = new PROPERTYKEY { fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"), pid = 14 };
    var items = new List<string>();
    for (int i = 0; i < n; i++) {
      IMMDevice d; col.Item(i, out d);
      string id; d.GetId(out id);
      IPropertyStore ps; d.OpenPropertyStore(0, out ps);
      PROPVARIANT v; ps.GetValue(ref key, out v);
      string name = v.vt == 31 ? Marshal.PtrToStringUni(v.p) : id;
      items.Add("{\"id\":\"" + Esc(id) + "\",\"name\":\"" + Esc(name) + "\",\"default\":" + (id == defId ? "true" : "false") + "}");
    }
    return "[" + string.Join(",", items.ToArray()) + "]";
  }

  public static void Set(string id) {
    var p = (IPolicyConfig)new CPolicyConfigClient();
    for (int role = 0; role < 3; role++) p.SetDefaultEndpoint(id, role);
  }
}
'@

[Console]::OutputEncoding = [Text.Encoding]::UTF8
if ($Mic) { [SofaAudio]::Mic($Mic) }
elseif ($Set) { [SofaAudio]::Set($Set); 'ok' }
else { [SofaAudio]::List() }
