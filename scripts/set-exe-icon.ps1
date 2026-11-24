# Writes an .ico file into a Windows executable's icon resources: as group icon 1, which GPUI loads
# for the window and taskbar, and over every group icon the executable already has, which Explorer
# shows. scripts/package-windows.sh uses it on tarn.exe, CEF's launcher, which carries CEF's icon.
param([string]$Exe, [string]$Icon)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

public static class TarnIcon {
    const int RtIcon = 3, RtGroupIcon = 14, FirstIconId = 30001;
    const uint LoadAsDatafile = 2;
    delegate bool EnumName(IntPtr module, IntPtr type, IntPtr name, IntPtr param);
    delegate bool EnumLanguage(IntPtr module, IntPtr type, IntPtr name, ushort language, IntPtr param);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr BeginUpdateResource(string file, bool deleteExisting);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool UpdateResource(IntPtr update, IntPtr type, IntPtr name, ushort language, byte[] data, uint size);
    [DllImport("kernel32", SetLastError = true)] static extern bool EndUpdateResource(IntPtr update, bool discard);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr LoadLibraryEx(string file, IntPtr reserved, uint flags);
    [DllImport("kernel32", SetLastError = true)] static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool EnumResourceNames(IntPtr module, IntPtr type, EnumName callback, IntPtr param);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool EnumResourceLanguages(IntPtr module, IntPtr type, IntPtr name, EnumLanguage callback, IntPtr param);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr FindResourceEx(IntPtr module, IntPtr type, IntPtr name, ushort language);
    [DllImport("kernel32", SetLastError = true)] static extern IntPtr LoadResource(IntPtr module, IntPtr info);
    [DllImport("kernel32", SetLastError = true)] static extern IntPtr LockResource(IntPtr data);
    [DllImport("kernel32", SetLastError = true)] static extern uint SizeofResource(IntPtr module, IntPtr info);

    public sealed class Group {
        public object Name;
        public ushort Language;
    }

    // The images of an .ico file and the RT_GROUP_ICON directory that names them FirstIconId onwards.
    public static List<byte[]> Images(byte[] ico, out byte[] group) {
        if (ico.Length < 6 || BitConverter.ToUInt16(ico, 0) != 0 || BitConverter.ToUInt16(ico, 2) != 1) throw new InvalidDataException("not an .ico file");
        int count = BitConverter.ToUInt16(ico, 4);
        if (count == 0 || ico.Length < 6 + 16 * count) throw new InvalidDataException("the .ico file lists no images");
        if (count > 256) throw new InvalidDataException("the .ico file lists too many images");
        var images = new List<byte[]>();
        group = new byte[6 + 14 * count];
        Array.Copy(ico, group, 6);
        for (int i = 0; i < count; i++) {
            int entry = 6 + 16 * i;
            long size = BitConverter.ToUInt32(ico, entry + 8), offset = BitConverter.ToUInt32(ico, entry + 12);
            if (size == 0 || offset + size > ico.Length) throw new InvalidDataException("an image lies outside the .ico file");
            var image = new byte[size];
            Array.Copy(ico, offset, image, 0, size);
            images.Add(image);
            Array.Copy(ico, entry, group, 6 + 14 * i, 12);
            BitConverter.GetBytes((ushort)(FirstIconId + i)).CopyTo(group, 6 + 14 * i + 12);
        }
        return images;
    }

    public static List<Group> Groups(string exe) {
        var groups = new List<Group>();
        IntPtr module = LoadLibraryEx(exe, IntPtr.Zero, LoadAsDatafile);
        if (module == IntPtr.Zero) throw new Win32Exception();
        try {
            EnumLanguage language = (m, t, name, lang, p) => { groups.Add(new Group { Name = Copy(name), Language = lang }); return true; };
            EnumName names = (m, t, name, p) => { EnumResourceLanguages(m, t, name, language, IntPtr.Zero); return true; };
            EnumResourceNames(module, (IntPtr)RtGroupIcon, names, IntPtr.Zero);
        } finally {
            FreeLibrary(module);
        }
        return groups;
    }

    public static byte[] Read(string exe, int type, Group group) {
        IntPtr module = LoadLibraryEx(exe, IntPtr.Zero, LoadAsDatafile);
        if (module == IntPtr.Zero) throw new Win32Exception();
        IntPtr name = Pointer(group.Name);
        try {
            IntPtr info = FindResourceEx(module, (IntPtr)type, name, group.Language);
            if (info == IntPtr.Zero) throw new Win32Exception();
            var bytes = new byte[SizeofResource(module, info)];
            Marshal.Copy(LockResource(LoadResource(module, info)), bytes, 0, bytes.Length);
            return bytes;
        } finally {
            Free(name);
            FreeLibrary(module);
        }
    }

    public static void Set(string exe, string ico) {
        byte[] group;
        var images = Images(File.ReadAllBytes(ico), out group);
        var targets = Groups(exe);
        if (!targets.Exists(g => g.Name is int && (int)g.Name == 1)) targets.Add(new Group { Name = 1, Language = 0 });
        IntPtr update = BeginUpdateResource(exe, false);
        if (update == IntPtr.Zero) throw new Win32Exception();
        bool written = false;
        try {
            for (int i = 0; i < images.Count; i++) {
                if (!UpdateResource(update, (IntPtr)RtIcon, (IntPtr)(FirstIconId + i), 0, images[i], (uint)images[i].Length)) throw new Win32Exception();
            }
            foreach (var target in targets) {
                IntPtr name = Pointer(target.Name);
                try {
                    if (!UpdateResource(update, (IntPtr)RtGroupIcon, name, target.Language, group, (uint)group.Length)) throw new Win32Exception();
                } finally {
                    Free(name);
                }
            }
            written = true;
        } finally {
            if (!EndUpdateResource(update, !written) && written) throw new Win32Exception();
        }
    }

    static object Copy(IntPtr name) {
        return ((long)name >> 16) == 0 ? (object)(int)name : Marshal.PtrToStringUni(name);
    }

    static IntPtr Pointer(object name) {
        return name is int ? (IntPtr)(int)name : Marshal.StringToHGlobalUni((string)name);
    }

    static void Free(IntPtr name) {
        if (((long)name >> 16) != 0) Marshal.FreeHGlobal(name);
    }
}
'@

if ($Exe) {
    if (-not $Icon) { throw 'usage: set-exe-icon.ps1 -Exe <file.exe> -Icon <file.ico>' }
    [TarnIcon]::Set((Resolve-Path -LiteralPath $Exe).ProviderPath, (Resolve-Path -LiteralPath $Icon).ProviderPath)
}
