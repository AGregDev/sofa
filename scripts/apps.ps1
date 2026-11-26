# Enumerates every app in the Windows Start menu (shell:AppsFolder) and
# extracts a 256px icon for each one into the icon cache folder.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File apps.ps1 -OutFile apps.json -IconDir icons
param(
  [Parameter(Mandatory = $true)][string]$OutFile,
  [Parameter(Mandatory = $true)][string]$IconDir
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class SofaIcons {
  [StructLayout(LayoutKind.Sequential)]
  public struct SIZE { public int cx; public int cy; }

  [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IShellItemImageFactory {
    [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
  }

  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr pbc,
    [MarshalAs(UnmanagedType.LPStruct)] Guid riid,
    [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory ppv);

  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr h);

  // SIIGBF_BIGGERSIZEOK | SIIGBF_ICONONLY
  const int FLAGS = 0x1 | 0x4;

  public static bool Save(string parsingName, string outPng, int px) {
    IShellItemImageFactory f;
    SHCreateItemFromParsingName(parsingName, IntPtr.Zero,
      new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), out f);
    SIZE s; s.cx = px; s.cy = px;
    IntPtr hbm;
    if (f.GetImage(s, FLAGS, out hbm) != 0 || hbm == IntPtr.Zero) return false;
    try {
      using (Bitmap raw = Image.FromHbitmap(hbm)) {
        // FromHbitmap drops alpha; re-wrap the pixel bits as ARGB to keep transparency.
        Rectangle r = new Rectangle(0, 0, raw.Width, raw.Height);
        BitmapData d = raw.LockBits(r, ImageLockMode.ReadOnly, raw.PixelFormat);
        try {
          using (Bitmap argb = new Bitmap(d.Width, d.Height, d.Stride, PixelFormat.Format32bppArgb, d.Scan0))
          using (Bitmap copy = new Bitmap(argb)) {
            copy.Save(outPng, ImageFormat.Png);
          }
        } finally { raw.UnlockBits(d); }
      }
      return true;
    } finally { DeleteObject(hbm); }
  }
}
'@

New-Item -ItemType Directory -Force -Path $IconDir | Out-Null
$md5 = [System.Security.Cryptography.MD5]::Create()

$apps = foreach ($a in Get-StartApps) {
  $hash = [BitConverter]::ToString($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($a.AppID))).Replace('-', '').ToLower()
  $png = Join-Path $IconDir "$hash.png"
  $icon = $null
  if (Test-Path $png) { $icon = $png }
  else {
    try { if ([SofaIcons]::Save("shell:AppsFolder\" + $a.AppID, $png, 256)) { $icon = $png } } catch { }
  }
  [pscustomobject]@{ name = $a.Name; appId = $a.AppID; icon = $icon }
}

$json = ConvertTo-Json -InputObject @($apps) -Depth 3
[IO.File]::WriteAllText($OutFile, $json, (New-Object Text.UTF8Encoding $false))
"ok $(@($apps).Count)"
