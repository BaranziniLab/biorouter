"""Synthetic Windows-only diagnostic. No ACL edits, elevated rights, or app calls."""
from __future__ import annotations

import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import sys
import tempfile


def main():
    if os.name != "nt":
        raise SystemExit("This diagnostic must run on actual Windows; no host simulation.")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.ReOpenFile.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD]
    kernel.ReOpenFile.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    native = ctypes.WinDLL("ntdll")

    class UnicodeString(ctypes.Structure):
        _fields_ = [("length", wintypes.USHORT), ("maximum_length", wintypes.USHORT),
                    ("buffer", ctypes.POINTER(wintypes.WCHAR))]

    class ObjectAttributes(ctypes.Structure):
        _fields_ = [("length", wintypes.ULONG), ("root_directory", wintypes.HANDLE),
                    ("object_name", ctypes.POINTER(UnicodeString)), ("attributes", wintypes.ULONG),
                    ("security_descriptor", wintypes.LPVOID), ("security_qos", wintypes.LPVOID)]

    class StatusUnion(ctypes.Union):
        _fields_ = [("status", wintypes.LONG), ("pointer", wintypes.LPVOID)]

    class StatusBlock(ctypes.Structure):
        _fields_ = [("status", StatusUnion), ("information", ctypes.c_size_t)]

    class FileInformation(ctypes.Structure):
        _fields_ = [("attributes", wintypes.DWORD), ("created", wintypes.FILETIME),
                    ("accessed", wintypes.FILETIME), ("written", wintypes.FILETIME),
                    ("volume", wintypes.DWORD), ("size_high", wintypes.DWORD),
                    ("size_low", wintypes.DWORD), ("links", wintypes.DWORD),
                    ("index_high", wintypes.DWORD), ("index_low", wintypes.DWORD)]

    native.NtCreateFile.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.ULONG,
                                   ctypes.POINTER(ObjectAttributes), ctypes.POINTER(StatusBlock),
                                   wintypes.LPVOID, wintypes.ULONG, wintypes.ULONG, wintypes.ULONG,
                                   wintypes.ULONG, wintypes.LPVOID, wintypes.ULONG]
    native.NtCreateFile.restype = wintypes.LONG
    native.RtlNtStatusToDosError.argtypes = [wintypes.LONG]
    native.RtlNtStatusToDosError.restype = wintypes.ULONG
    kernel.GetFileInformationByHandle.argtypes = [wintypes.HANDLE, ctypes.POINTER(FileInformation)]
    kernel.GetFileInformationByHandle.restype = wintypes.BOOL
    invalid = ctypes.c_void_p(-1).value
    flags = {"backup": 0x02000000, "backup_nofollow": 0x02200000}
    access = {"file_generic_read": 0x00120089, "generic_read": 0x80000000,
              "read_attributes": 0x80, "none": 0}
    rows = []

    def identity(handle):
        info = FileInformation()
        if not kernel.GetFileInformationByHandle(handle, ctypes.byref(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        if not info.attributes & 0x10 or info.attributes & 0x400:
            raise RuntimeError("Expected an actual non-reparse directory.")
        return info.volume, info.index_high, info.index_low, info.created.dwHighDateTime, info.created.dwLowDateTime

    def native_reopened(handle, share, stage, target, original_flags):
        buffer = ctypes.create_unicode_buffer(1)
        name = UnicodeString(0, ctypes.sizeof(wintypes.WCHAR), buffer)
        attributes = ObjectAttributes(ctypes.sizeof(ObjectAttributes), handle,
                                      ctypes.pointer(name), 0, None, None)
        block, result = StatusBlock(), wintypes.HANDLE()
        block.status.status = 0x103
        status = native.NtCreateFile(ctypes.byref(result), access["file_generic_read"],
                                     ctypes.byref(attributes), ctypes.byref(block), None,
                                     0, share, 1, 0x00200021, None, 0)
        error = native.RtlNtStatusToDosError(status) if status != 0 else 0
        try:
            same_object = status == 0 and identity(result) == identity(handle)
        except BaseException:
            if status == 0:
                kernel.CloseHandle(result)
            raise
        rows.append(dict(target=target, stage=stage, original_flags=original_flags,
                         api="NtCreateFile", share_mode=share, ntstatus=hex(status & 0xFFFFFFFF),
                         success=status == 0, same_object=same_object, windows_error=error))
        if status == 0 and not same_object:
            kernel.CloseHandle(result)
            raise RuntimeError("Native directory reopen changed its captured identity.")
        return None if status != 0 else result

    def reopened(handle, desired, share, mode, stage, target, original_flags):
        ctypes.set_last_error(0)
        result = kernel.ReOpenFile(handle, desired, share, mode)
        error = ctypes.get_last_error() if result == invalid else 0
        rows.append(dict(target=target, stage=stage, original_flags=original_flags,
                         desired_access=hex(desired), share_mode=share, flags=hex(mode),
                         success=result != invalid, windows_error=error))
        return None if result == invalid else result

    with tempfile.TemporaryDirectory(prefix="biorouter-lease-probe-") as temporary:
        directory = Path(temporary) / "workspace"
        directory.mkdir()
        (directory / "file.txt").write_bytes(b"synthetic-approved\n")
        targets = {"volume_root": Path(temporary).anchor, "temporary_directory": str(directory)}
        for target, path in targets.items():
            for original_name, original_flags in flags.items():
                ctypes.set_last_error(0)
                original = kernel.CreateFileW(path, access["generic_read"], 3, None, 3,
                                              original_flags, None)
                rows.append(dict(target=target, stage="create_original", original_flags=original_name,
                                 success=original != invalid,
                                 windows_error=ctypes.get_last_error() if original == invalid else 0))
                if original == invalid:
                    continue
                captured = native_capture = None
                try:
                    for desired in access.values():
                        for mode in flags.values():
                            captured = reopened(original, desired, 7, mode, "capture", target, original_name)
                            if captured is not None:
                                try:
                                    leased = reopened(captured, access["file_generic_read"], 3,
                                                      flags["backup_nofollow"], "lease_original_live",
                                                      target, original_name)
                                    if leased is not None:
                                        kernel.CloseHandle(leased)
                                finally:
                                    kernel.CloseHandle(captured)
                    # Legacy ReOpenFile comparison drops the original protective handle after capture.
                    captured = reopened(original, access["file_generic_read"], 7,
                                        flags["backup_nofollow"], "legacy_reopen_capture_comparison", target, original_name)
                    native_capture = native_reopened(original, 7, "native_capture", target, original_name)
                finally:
                    kernel.CloseHandle(original)
                if captured is not None:
                    try:
                        leased = reopened(captured, access["file_generic_read"], 3,
                                          flags["backup_nofollow"], "legacy_reopen_lease_original_dropped_comparison",
                                          target, original_name)
                        if leased is not None:
                            kernel.CloseHandle(leased)
                    finally:
                        kernel.CloseHandle(captured)
                if native_capture is not None:
                    try:
                        leased = native_reopened(native_capture, 3, "native_lease_original_dropped",
                                                 target, original_name)
                        if leased is not None:
                            kernel.CloseHandle(leased)
                    finally:
                        kernel.CloseHandle(native_capture)
        # Positive outside content is only our own temporary synthetic file, never user storage.
        if (directory / "file.txt").read_bytes() != b"synthetic-approved\n":
            raise RuntimeError("Synthetic canary changed unexpectedly.")
    native_rows = [row for row in rows if row.get("api") == "NtCreateFile"]
    native_success = (len(native_rows) == 8
                      and all(row["success"] and row["same_object"] for row in native_rows))
    print(json.dumps(dict(kind="windows-directory-lease-probe", platform=sys.platform,
                          python=sys.version.split()[0], app_or_tool_calls=0, acl_changes=0,
                          canary_unchanged=True, native_directory_reopens_verified=native_success,
                          rows=rows), indent=2))
    if not native_success:
        raise SystemExit("The native directory reopen control failed; inspect the actual Windows rows.")


if __name__ == "__main__":
    main()
