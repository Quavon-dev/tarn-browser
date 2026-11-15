/// Chromium's sandboxed children load this DLL too, where some system libraries must not load at
/// all (user32 and gdi32 under win32k lockdown). They load on first use instead, which a child
/// never reaches: CEF's list for client DLLs (CEF_DELAYLOAD_FLAGS in cmake/cef_variables.cmake)
/// plus two of GPUI's that pull in user32.
const DELAYLOAD: &[&str] = &[
    "d3dcompiler_47.dll",
    "api-ms-win-shcore-scaling-l1-1-1.dll",
    "api-ms-win-core-synch-l1-2-0.dll",
    "api-ms-win-core-winrt-error-l1-1-0.dll",
    "api-ms-win-core-winrt-l1-1-0.dll",
    "api-ms-win-core-winrt-string-l1-1-0.dll",
    "advapi32.dll",
    "bcryptprimitives.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "credui.dll",
    "cryptui.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3d9.dll",
    "dbghelp.dll",
    "dcomp.dll",
    "dwmapi.dll",
    "dxgi.dll",
    "dxva2.dll",
    "esent.dll",
    "fontsub.dll",
    "gdi32.dll",
    "hid.dll",
    "imagehlp.dll",
    "imm32.dll",
    "mmdevapi.dll",
    "msi.dll",
    "netapi32.dll",
    "ncrypt.dll",
    "ole32.dll",
    "oleacc.dll",
    "oleaut32.dll",
    "pdh.dll",
    "propsys.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "rstrtmgr.dll",
    "setupapi.dll",
    "shell32.dll",
    "shlwapi.dll",
    "uiautomationcore.dll",
    "urlmon.dll",
    "user32.dll",
    "usp10.dll",
    "uxtheme.dll",
    "wer.dll",
    "wevtapi.dll",
    "wininet.dll",
    "winusb.dll",
    "wsock32.dll",
    "wtsapi32.dll",
    "crypt32.dll",
    "dhcpcsvc.dll",
    "dwrite.dll",
    "iphlpapi.dll",
    "secur32.dll",
    "userenv.dll",
    "winhttp.dll",
    "winmm.dll",
    "winspool.drv",
    "wintrust.dll",
    "ws2_32.dll",
];

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo::rustc-link-arg-cdylib=/IGNORE:4199");
        for dll in DELAYLOAD {
            println!("cargo::rustc-link-arg-cdylib=/DELAYLOAD:{dll}");
        }
    }
}
