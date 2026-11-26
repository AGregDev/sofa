# Saves a program's largest icon (up to 256 px, as Explorer shows it) as a PNG.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File icon.ps1 -Path <exe> -Out <png>
# Prints the saved width, or nothing when the program has no icon.
param(
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class SofaIcon {
  [StructLayout(LayoutKind.Sequential)] public struct SIZE { public int cx; public int cy; }
  [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IShellItemImageFactory { [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm); }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr pbc,
    [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory ppv);
  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr h);

  public static int Save(string path, string outPng) {
    IShellItemImageFactory f;
    SHCreateItemFromParsingName(path, IntPtr.Zero, new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), out f);
    SIZE s; s.cx = 256; s.cy = 256;
    IntPtr hbm;
    // SIIGBF_BIGGERSIZEOK | SIIGBF_ICONONLY: the real icon, never a stretched copy.
    if (f.GetImage(s, 0x1 | 0x4, out hbm) != 0 || hbm == IntPtr.Zero) return 0;
    try {
      using (Bitmap raw = Image.FromHbitmap(hbm)) {
        Rectangle r = new Rectangle(0, 0, raw.Width, raw.Height);
        BitmapData d = raw.LockBits(r, ImageLockMode.ReadOnly, raw.PixelFormat);
        try {
          using (Bitmap argb = new Bitmap(d.Width, d.Height, d.Stride, PixelFormat.Format32bppArgb, d.Scan0))
          using (Bitmap copy = new Bitmap(argb)) { copy.Save(outPng, ImageFormat.Png); return copy.Width; }
        } finally { raw.UnlockBits(d); }
      }
    } finally { DeleteObject(hbm); }
  }
}
'@
$w = [SofaIcon]::Save($Path, $Out)
if ($w) { $w }
