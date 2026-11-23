#!/usr/bin/env python3
"""Check the packaged JNI libraries and their 16 KiB ELF and APK alignment."""

import argparse
from pathlib import Path
import struct
import zipfile


PAGE = 16_384
MACHINES = {"arm64-v8a": 183, "x86_64": 62}
REQUIRED = {"libimpasto.so"}
ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
PROGRAM_HEADER = struct.Struct("<IIQQQQQQ")
ZIP_HEADER = struct.Struct("<IHHHHHIIIHH")


def alignment(archive, entry, machine):
    with archive.open(entry) as elf:
        ident, _, actual, _, _, offset, _, _, _, size, count, *_ = ELF_HEADER.unpack(
            elf.read(ELF_HEADER.size)
        )
        if not ident.startswith(b"\x7fELF\x02\x01") or actual != machine:
            raise ValueError("ELF class, byte order or machine does not match the ABI")
        if size < PROGRAM_HEADER.size or offset + size * count > entry.file_size:
            raise ValueError("invalid ELF program header table")
        alignments = []
        for index in range(count):
            elf.seek(offset + size * index)
            kind, _, file_offset, vaddr, *_, boundary = PROGRAM_HEADER.unpack(
                elf.read(PROGRAM_HEADER.size)
            )
            if kind == 1:  # PT_LOAD
                if boundary < PAGE or (vaddr - file_offset) % PAGE:
                    raise ValueError("ELF load segment is not 16 KiB aligned")
                alignments.append(boundary)
        if not alignments:
            raise ValueError("ELF has no load segments")
        return min(alignments)


def check(package, abis):
    prefix = {".apk": "lib/", ".aab": "base/lib/", ".aar": "jni/"}[package.suffix]
    found = {abi: set() for abi in abis}
    with zipfile.ZipFile(package) as archive, package.open("rb") as container:
        for entry in archive.infolist():
            if not entry.filename.startswith(prefix) or not entry.filename.endswith(".so"):
                continue
            abi, name = entry.filename.removeprefix(prefix).split("/", 1)
            if abi not in found:
                raise ValueError(f"unexpected native ABI {abi}")
            if name in found[abi]:
                raise ValueError(f"duplicate library {entry.filename}")
            found[abi].add(name)
            try:
                boundary = alignment(archive, entry, MACHINES[abi])
                if package.suffix == ".apk" and entry.compress_type == zipfile.ZIP_STORED:
                    container.seek(entry.header_offset)
                    signature, *_, name_len, extra_len = ZIP_HEADER.unpack(container.read(ZIP_HEADER.size))
                    offset = entry.header_offset + ZIP_HEADER.size + name_len + extra_len
                    if signature != 0x04034B50 or offset % PAGE:
                        raise ValueError("uncompressed APK entry is not 16 KiB aligned")
            except (ValueError, struct.error) as error:
                raise ValueError(f"{entry.filename}: {error}") from error
            print(f"{entry.filename}: ELF load segment alignment >= {boundary:,} bytes")
    for abi, libraries in found.items():
        if missing := REQUIRED - libraries:
            raise ValueError(f"{abi}: missing {', '.join(sorted(missing))}")
        if extra := libraries - REQUIRED:
            raise ValueError(f"{abi}: unexpected libraries {', '.join(sorted(extra))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--abi", action="append", choices=MACHINES)
    args = parser.parse_args()
    if args.package.suffix not in {".apk", ".aab", ".aar"}:
        parser.error("expected an APK, AAB or AAR")
    try:
        check(args.package, args.abi or MACHINES)
    except (ValueError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
