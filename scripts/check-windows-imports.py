#!/usr/bin/env python3
"""The shipped Windows executables import only system DLLs (DD-10 rules 3, 4 and 5).

Usage:
  python3 -I scripts/check-windows-imports.py EXE...    check (rust.yml cross-build-nightly)
  python3 -I scripts/check-windows-imports.py --self-test

whisper.cpp is C++. A mingw build that links its C++ runtime, OpenMP or winpthreads
dynamically produces a biorouter.exe that starts on the build machine and then fails on a
user's machine with a loader dialog naming a DLL, before any Biorouter code runs. The
release does stage three mingw runtime DLLs beside the executables (WIN_DLL_STAGE in
scripts/cross-env.sh), so the package would carry them; this check holds the stronger
rule that the engine needs none of them: libstdc++ and libgcc are linked statically,
OpenMP is off, and ggml's threads are Windows threads.

It reads each file's PE import directory and delay import directory, prints every DLL
name, and fails on any of FORBIDDEN (case insensitive). Standard library only; no objdump.

A DLL that a build WITHOUT the engine already imports is a fact about the existing build,
not about the engine. Such a DLL is recorded in ALLOWANCES with the run that measured it,
instead of dropping the check. The table is empty: nothing has been measured yet.
"""

import struct
import sys

FORBIDDEN = (
    "libstdc++-6.dll",
    "libgcc_s_seh-1.dll",
    "libwinpthread-1.dll",
    "libgomp-1.dll",
    "vcomp140.dll",
)

# {executable base name (lower case): {dll (lower case): "why, and the run id that measured it"}}
ALLOWANCES = {}

IMAGE_DIRECTORY_ENTRY_IMPORT = 1
IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT = 13


class NotPe(Exception):
    pass


def _u16(data, offset):
    if offset < 0 or offset + 2 > len(data):
        raise NotPe(f"truncated at offset {offset}")
    return struct.unpack_from("<H", data, offset)[0]


def _u32(data, offset):
    if offset < 0 or offset + 4 > len(data):
        raise NotPe(f"truncated at offset {offset}")
    return struct.unpack_from("<I", data, offset)[0]


def _sections(data, pe):
    count = _u16(data, pe + 6)
    optional_size = _u16(data, pe + 20)
    table = pe + 24 + optional_size
    sections = []
    for i in range(count):
        entry = table + 40 * i
        virtual_size = _u32(data, entry + 8)
        virtual_address = _u32(data, entry + 12)
        raw_size = _u32(data, entry + 16)
        raw_pointer = _u32(data, entry + 20)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_pointer, raw_size))
    return sections


def _rva_to_offset(sections, rva):
    for virtual_address, span, raw_pointer, raw_size in sections:
        if virtual_address <= rva < virtual_address + span:
            delta = rva - virtual_address
            if delta >= raw_size:
                raise NotPe(f"RVA {rva:#x} points past the raw data of its section")
            return raw_pointer + delta
    raise NotPe(f"RVA {rva:#x} is in no section")


def _c_string(data, offset):
    end = data.find(b"\0", offset)
    if end < 0 or end - offset > 512:
        raise NotPe(f"unterminated name at offset {offset}")
    return data[offset:end].decode("ascii", "replace")


def imports(data):
    """Return [(dll name, "import" | "delay")] in the order the directories list them."""
    if data[:2] != b"MZ":
        raise NotPe("no MZ header")
    pe = _u32(data, 0x3C)
    if data[pe:pe + 4] != b"PE\0\0":
        raise NotPe("no PE signature")
    optional = pe + 24
    magic = _u16(data, optional)
    if magic == 0x20B:  # PE32+
        count_offset, directories = optional + 108, optional + 112
    elif magic == 0x10B:  # PE32
        count_offset, directories = optional + 92, optional + 96
    else:
        raise NotPe(f"unknown optional header magic {magic:#x}")
    directory_count = _u32(data, count_offset)
    sections = _sections(data, pe)
    found = []

    def directory(index):
        if index >= directory_count:
            return 0, 0
        return _u32(data, directories + 8 * index), _u32(data, directories + 8 * index + 4)

    rva, size = directory(IMAGE_DIRECTORY_ENTRY_IMPORT)
    if rva and size:
        offset = _rva_to_offset(sections, rva)
        for i in range(100000):
            entry = offset + 20 * i
            fields = struct.unpack_from("<5I", data, entry) if entry + 20 <= len(data) else None
            if fields is None:
                raise NotPe("import directory runs past the end of the file")
            if not any(fields):
                break
            found.append((_c_string(data, _rva_to_offset(sections, fields[3])), "import"))

    rva, size = directory(IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT)
    if rva and size:
        offset = _rva_to_offset(sections, rva)
        for i in range(100000):
            entry = offset + 32 * i
            fields = struct.unpack_from("<8I", data, entry) if entry + 32 <= len(data) else None
            if fields is None:
                raise NotPe("delay import directory runs past the end of the file")
            if not any(fields):
                break
            # Attribute bit 0 set: the fields are RVAs (every linker since Visual C++ 7).
            # Clear: they are virtual addresses, which no toolchain this repository uses emits.
            if not fields[0] & 1:
                raise NotPe("a delay import descriptor uses virtual addresses; read it by hand")
            found.append((_c_string(data, _rva_to_offset(sections, fields[1])), "delay"))
    return found


def check(path, data, out):
    """Print the file's DLLs and return the forbidden ones that no allowance covers."""
    base = path.replace("\\", "/").rsplit("/", 1)[-1].lower()
    allowed = ALLOWANCES.get(base, {})
    bad = []
    out.write(f"{path}:\n")
    for name, kind in imports(data):
        lower = name.lower()
        note = ""
        if lower in FORBIDDEN:
            if lower in allowed:
                note = f"  (allowed: {allowed[lower]})"
            else:
                note = "  <- forbidden"
                bad.append(name)
        out.write(f"  DLL Name: {name}{' (delay load)' if kind == 'delay' else ''}{note}\n")
    return bad


def main(paths):
    failed = False
    for path in paths:
        try:
            with open(path, "rb") as handle:
                data = handle.read()
            bad = check(path, data, sys.stdout)
        except (OSError, NotPe, struct.error) as error:
            print(f"::error::{path}: cannot read its imports: {error}")
            return 2
        if bad:
            failed = True
            print(
                f"::error::{path} imports {', '.join(bad)}. The engine must link the C++ runtime "
                "statically and use no OpenMP or winpthreads DLL (DD-10 rules 3 to 5); a user's "
                "machine without that DLL cannot start Biorouter at all."
            )
    if failed:
        return 1
    print(f"OK: {len(paths)} executable(s) import only system DLLs")
    return 0


# ---------------------------------------------------------------------------------------
# Self test: two minimal PE32+ images built in memory, one importing KERNEL32.dll and one
# importing libstdc++-6.dll; plus a PE32 image, a delay load, and a file that is not a PE.


def synthesize(dlls, delay=(), pe32=False):
    file_alignment = 0x200
    section_rva = 0x1000
    body = bytearray(file_alignment)
    descriptors = 20 * (len(dlls) + 1)
    delay_descriptors = 32 * (len(delay) + 1) if delay else 0
    names_at = descriptors + delay_descriptors
    name_offsets = []
    cursor = names_at
    for name in list(dlls) + list(delay):
        encoded = name.encode("ascii") + b"\0"
        body[cursor:cursor + len(encoded)] = encoded
        name_offsets.append(cursor)
        cursor += len(encoded)
    for i in range(len(dlls)):
        struct.pack_into("<5I", body, 20 * i, 0, 0, 0, section_rva + name_offsets[i], 0)
    for i in range(len(delay)):
        struct.pack_into(
            "<8I", body, descriptors + 32 * i, 1, section_rva + name_offsets[len(dlls) + i],
            0, 0, 0, 0, 0, 0,
        )

    optional_size = 224 if pe32 else 240
    headers = bytearray(file_alignment)
    headers[0:2] = b"MZ"
    struct.pack_into("<I", headers, 0x3C, 0x40)
    headers[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", headers, 0x44, 0x14C if pe32 else 0x8664, 1, 0, 0, 0, optional_size, 0x22)
    optional = 0x58
    struct.pack_into("<H", headers, optional, 0x10B if pe32 else 0x20B)
    count_offset, directories = (optional + 92, optional + 96) if pe32 else (optional + 108, optional + 112)
    struct.pack_into("<I", headers, count_offset, 16)
    struct.pack_into("<II", headers, directories + 8 * IMAGE_DIRECTORY_ENTRY_IMPORT, section_rva, descriptors)
    if delay:
        struct.pack_into(
            "<II", headers, directories + 8 * IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT,
            section_rva + descriptors, delay_descriptors,
        )
    section = optional + optional_size
    headers[section:section + 8] = b".idata\0\0"
    struct.pack_into("<IIII", headers, section + 8, len(body), section_rva, len(body), file_alignment)
    return bytes(headers + body)


def self_test():
    class Sink:
        def __init__(self):
            self.text = ""

        def write(self, text):
            self.text += text

    cases = [
        ("a PE32+ image importing KERNEL32.dll passes", synthesize(["KERNEL32.dll"]), []),
        (
            "a PE32+ image importing libstdc++-6.dll fails",
            synthesize(["KERNEL32.dll", "libstdc++-6.dll"]),
            ["libstdc++-6.dll"],
        ),
        (
            "the deny list ignores case",
            synthesize(["LIBWINPTHREAD-1.DLL", "msvcrt.dll"]),
            ["LIBWINPTHREAD-1.DLL"],
        ),
        ("a delay loaded vcomp140.dll fails", synthesize(["KERNEL32.dll"], delay=["vcomp140.dll"]), ["vcomp140.dll"]),
        ("a PE32 image importing libgomp-1.dll fails", synthesize(["libgomp-1.dll"], pe32=True), ["libgomp-1.dll"]),
        (
            "every one of the five is refused",
            synthesize(list(FORBIDDEN) + ["ADVAPI32.dll"]),
            list(FORBIDDEN),
        ),
    ]
    failures = 0
    for name, image, want in cases:
        sink = Sink()
        try:
            got = check("fixture.exe", image, sink)
        except NotPe as error:
            got = [f"<not a PE: {error}>"]
        listed = all(f"DLL Name: {dll}" in sink.text for dll in want)
        if got == want and listed:
            print(f"ok   {name}")
        else:
            failures += 1
            print(f"FAIL {name}: got {got}, want {want}")
            print(sink.text, end="")
    try:
        imports(b"MZ" + b"\0" * 100)
        failures += 1
        print("FAIL a file that is not a PE is refused: it was read")
    except NotPe:
        print("ok   a file that is not a PE is refused")
    ALLOWANCES["fixture.exe"] = {"libwinpthread-1.dll": "self test allowance"}
    try:
        got = check("dir/Fixture.exe", synthesize(["libwinpthread-1.dll"]), Sink())
    finally:
        ALLOWANCES.pop("fixture.exe")
    if got == []:
        print("ok   a named allowance covers exactly its executable and DLL")
    else:
        failures += 1
        print(f"FAIL a named allowance covers exactly its executable and DLL: got {got}")
    if failures:
        print(f"::error::{failures} check-windows-imports self test case(s) did not behave as expected")
        return 1
    print(f"check-windows-imports self test: {len(cases) + 2} cases behaved as expected")
    return 0


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if arguments == ["--self-test"]:
        sys.exit(self_test())
    if not arguments or any(a.startswith("-") for a in arguments):
        print("usage: check-windows-imports.py EXE... | --self-test", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(arguments))
