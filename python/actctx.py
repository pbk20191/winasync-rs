r"""Load the component's reg-free WinRT manifest into an activation context.

`HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId` is owned by
`NT SERVICE\TrustedInstaller`, so a third-party component cannot register
there even elevated. The documented alternative is a manifest on the
consuming process -- and since `python.exe`'s manifest is not ours to edit,
we load ours at run time instead.
"""
import ctypes
from ctypes import wintypes

k32 = ctypes.WinDLL("kernel32", use_last_error=True)

class ACTCTXW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.ULONG),
        ("dwFlags", wintypes.DWORD),
        ("lpSource", wintypes.LPCWSTR),
        ("wProcessorArchitecture", wintypes.USHORT),
        ("wLangId", wintypes.LANGID),
        ("lpAssemblyDirectory", wintypes.LPCWSTR),
        ("lpResourceName", wintypes.LPCWSTR),
        ("lpApplicationName", wintypes.LPCWSTR),
        ("hModule", wintypes.HMODULE),
    ]

k32.CreateActCtxW.argtypes = [ctypes.POINTER(ACTCTXW)]
k32.CreateActCtxW.restype = wintypes.HANDLE
k32.ActivateActCtx.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t)]
k32.ActivateActCtx.restype = wintypes.BOOL

def activate(manifest_path: str) -> None:
    ctx = ACTCTXW()
    ctx.cbSize = ctypes.sizeof(ACTCTXW)
    ctx.lpSource = manifest_path
    handle = k32.CreateActCtxW(ctypes.byref(ctx))
    if handle == wintypes.HANDLE(-1).value or not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    cookie = ctypes.c_size_t()
    if not k32.ActivateActCtx(handle, ctypes.byref(cookie)):
        raise ctypes.WinError(ctypes.get_last_error())
