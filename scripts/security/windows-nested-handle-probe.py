"""Windows-only, bounded synthetic directory-rename diagnostic.

No app calls, network, ACL edits, subprocesses, elevated rights, retries, or sleeps.
Primary references:
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/133840e4-778e-44ca-9b41-da2323615075
"""
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
        raise SystemExit("Run only on actual Windows; host simulation is not evidence.")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    native = ctypes.WinDLL("ntdll")
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL

    class UnicodeString(ctypes.Structure):
        _fields_ = [("length", wintypes.USHORT), ("maximum_length", wintypes.USHORT),
                    ("buffer", ctypes.POINTER(wintypes.WCHAR))]

    class ObjectAttributes(ctypes.Structure):
        _fields_ = [("length", wintypes.ULONG), ("root_directory", wintypes.HANDLE),
                    ("object_name", ctypes.POINTER(UnicodeString)),
                    ("attributes", wintypes.ULONG),
                    ("security_descriptor", wintypes.LPVOID),
                    ("security_qos", wintypes.LPVOID)]

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
                                   ctypes.POINTER(ObjectAttributes),
                                   ctypes.POINTER(StatusBlock), wintypes.LPVOID,
                                   wintypes.ULONG, wintypes.ULONG, wintypes.ULONG,
                                   wintypes.ULONG, wintypes.LPVOID, wintypes.ULONG]
    native.NtCreateFile.restype = wintypes.LONG
    native.RtlNtStatusToDosError.argtypes = [wintypes.LONG]
    native.RtlNtStatusToDosError.restype = wintypes.ULONG
    kernel.GetFileInformationByHandle.argtypes = [wintypes.HANDLE,
                                                 ctypes.POINTER(FileInformation)]
    kernel.GetFileInformationByHandle.restype = wintypes.BOOL
    invalid = ctypes.c_void_p(-1).value
    rows, errors = [], []
    content = b"synthetic-approved\n"

    def identity(handle):
        information = FileInformation()
        if not kernel.GetFileInformationByHandle(handle, ctypes.byref(information)):
            raise ctypes.WinError(ctypes.get_last_error())
        if not information.attributes & 0x10 or information.attributes & 0x400:
            raise RuntimeError("Expected an actual non-reparse directory.")
        return (information.volume, information.index_high, information.index_low,
                information.created.dwHighDateTime, information.created.dwLowDateTime)

    def run_case(base, case, operation):
        parent = base / case / "parent"
        nested = parent / "new" / "nested"
        nested.mkdir(parents=True)
        (nested / "file.txt").write_bytes(content)
        handles = []
        canary = nested / "file.txt"

        def close(handle):
            if handle not in handles:
                raise RuntimeError("Attempted to close an unowned diagnostic handle.")
            if not kernel.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())
            handles.remove(handle)

        def reopen(handle, share, stage):
            empty = ctypes.create_unicode_buffer(1)
            name = UnicodeString(0, ctypes.sizeof(wintypes.WCHAR), empty)
            attributes = ObjectAttributes(ctypes.sizeof(ObjectAttributes), handle,
                                          ctypes.pointer(name), 0, None, None)
            block, result = StatusBlock(), wintypes.HANDLE()
            block.status.status = 0x103
            status = native.NtCreateFile(ctypes.byref(result), 0x00120089,
                                         ctypes.byref(attributes), ctypes.byref(block),
                                         None, 0, share, 1, 0x00200021, None, 0)
            row = dict(case=case, stage=stage, api="NtCreateFile", share_mode=share,
                       ntstatus=hex(status & 0xFFFFFFFF), success=status == 0,
                       windows_error=int(native.RtlNtStatusToDosError(status)) if status else 0)
            rows.append(row)
            if status != 0:
                raise RuntimeError(f"{stage}: NtCreateFile returned {row['ntstatus']}")
            captured = result.value
            handles.append(captured)
            row["same_object"] = identity(captured) == identity(handle)
            if not row["same_object"]:
                raise RuntimeError("Native reopen substituted the selected object.")
            return captured

        def pin(path, stage):
            original = kernel.CreateFileW(str(path), 0x80000000, 3, None, 3,
                                          0x02200000, None)
            if original == invalid:
                raise ctypes.WinError(ctypes.get_last_error())
            handles.append(original)
            try:
                return reopen(original, 7, stage)
            finally:
                close(original)

        def rename(source, destination, expected, stage, expected_error=None):
            nonlocal canary
            before = canary
            try:
                os.rename(source, destination)
                succeeded, error = True, 0
                canary = destination / before.relative_to(source)
            except OSError as problem:
                succeeded, error = False, problem.winerror
            rows.append(dict(case=case, stage=stage, api="os.rename/MoveFileExW",
                             success=succeeded, windows_error=error,
                             expected_success=expected, expected_error=expected_error))
            if expected_error is not None and error != expected_error:
                raise RuntimeError(f"{stage}: expected Windows error {expected_error}, got {error}")
            if succeeded != expected:
                raise RuntimeError(f"{stage}: expected success={expected}, got {succeeded}, error={error}")

        def same_identity(handle, expected, stage):
            matched = identity(handle) == expected
            rows.append(dict(case=case, stage=stage, same_object=matched))
            if not matched:
                raise RuntimeError("Rename changed the captured object identity.")

        try:
            operation(parent, nested, pin, reopen, close, rename, same_identity)
        except Exception as problem:
            errors.append(dict(case=case, error=str(problem)))
        finally:
            for handle in list(reversed(handles)):
                try:
                    close(handle)
                except Exception as problem:
                    errors.append(dict(case=case, cleanup_error=str(problem)))
        try:
            unchanged = canary.read_bytes() == content
        except OSError as problem:
            unchanged = False
            errors.append(dict(case=case, canary_error=str(problem)))
        rows.append(dict(case=case, stage="all_handles_closed_canary_check",
                         remaining_handles=len(handles), canary_unchanged=unchanged))
        if handles or not unchanged:
            errors.append(dict(case=case, error="Handle cleanup or synthetic canary check failed."))

    def exact_parent(parent, nested, pin, reopen, close, rename, same):
        captured = pin(parent, "capture_parent_share7_original_dropped")
        expected = identity(captured)
        rename(parent, parent.with_name("moved-parent"), True, "exact_parent_idle_move")
        same(captured, expected, "exact_parent_identity_after_move")

    def descendant_kept_dropped(parent, nested, pin, reopen, close, rename, same):
        captured = pin(parent, "capture_parent_share7_original_dropped")
        descendant = pin(nested, "capture_nested_share7_original_dropped")
        expected_parent, expected_nested = identity(captured), identity(descendant)
        moved = parent.with_name("moved-parent")
        rename(parent, moved, False, "ancestor_move_nested_share7_held", expected_error=5)
        same(descendant, expected_nested, "nested_identity_after_denial")
        close(descendant)
        rename(parent, moved, True, "ancestor_move_nested_dropped_parent_still_held")
        same(captured, expected_parent, "parent_identity_after_retry")

    def exact_nested(parent, nested, pin, reopen, close, rename, same):
        captured = pin(nested, "capture_nested_share7_original_dropped")
        expected = identity(captured)
        rename(nested, nested.with_name("moved-nested"), True, "exact_nested_idle_move")
        same(captured, expected, "exact_nested_identity_after_move")

    def active_lease(parent, nested, pin, reopen, close, rename, same):
        captured = pin(nested, "capture_nested_share7_original_dropped")
        expected = identity(captured)
        lease = reopen(captured, 3, "active_nested_share3_lease")
        moved = nested.with_name("moved-nested")
        rename(nested, moved, False, "exact_nested_move_active_lease_held")
        close(lease)
        rename(nested, moved, True, "exact_nested_move_lease_released")
        same(captured, expected, "nested_identity_after_lease_retry")

    with tempfile.TemporaryDirectory(prefix="biorouter-nested-handle-probe-") as temporary:
        base = Path(temporary)
        for case, operation in [("exact_parent", exact_parent),
                                ("descendant_kept_dropped", descendant_kept_dropped),
                                ("exact_nested", exact_nested),
                                ("active_lease", active_lease)]:
            run_case(base, case, operation)
    print(json.dumps(dict(kind="windows-nested-handle-probe", platform=sys.platform,
                          python=sys.version.split()[0], cases=4, app_or_tool_calls=0,
                          acl_changes=0, all_controls_met=not errors,
                          evidence="actual Windows execution only", rows=rows, errors=errors), indent=2))
    if errors:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
