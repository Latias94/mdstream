"""Portable native-binary contracts for Flutter release artifacts."""

from __future__ import annotations

import plistlib
import struct
from dataclasses import dataclass
from pathlib import PurePosixPath
from types import MappingProxyType
from typing import Mapping


ANDROID_MIN_LOAD_ALIGNMENT = 16 * 1024
LINUX_GLIBC_BASELINE = (2, 17)
FRAMEWORK_MODULE_MAP = (
    "framework module MdstreamFFI {\n"
    '  umbrella header "mdstream.h"\n'
    "  export *\n"
    "  module * { export * }\n"
    "}\n"
)
NATIVE_FILE_SUFFIXES = (
    ".a",
    ".dylib",
    ".dll",
    ".dwo",
    ".dwp",
    ".exe",
    ".exp",
    ".ilk",
    ".lib",
    ".node",
    ".o",
    ".obj",
    ".pdb",
    ".rmeta",
    ".rlib",
    ".so",
)
NATIVE_FILE_MAGICS = (
    b"\x7fELF",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
    b"\xbe\xba\xfe\xca",
    b"\xca\xfe\xba\xbf",
    b"\xbf\xba\xfe\xca",
    b"MZ",
    b"!<arch>\n",
    b"!<thin>\n",
    b"BC\xc0\xde",
    b"\xde\xc0\x17\x0b",
    b"Microsoft C/C++ MSF 7.00",
)
COFF_OBJECT_MACHINES = (
    b"\x4c\x01",
    b"\x64\x86",
    b"\xc0\x01",
    b"\xc4\x01",
    b"\x64\xaa",
)
COFF_BIGOBJ_SIGNATURE = b"\x00\x00\xff\xff"
COFF_BIGOBJ_CLASS_ID = bytes.fromhex("c7a1bad1eebaa94baf20faf66aa4dcb8")
NATIVE_MAGIC_PREFIX_BYTES = max(
    max(map(len, NATIVE_FILE_MAGICS)),
    max(map(len, COFF_OBJECT_MACHINES)),
    28,
)


class NativeArtifactError(RuntimeError):
    """Raised when native bytes do not satisfy their distribution contract."""


@dataclass(frozen=True)
class NativeImage:
    format: str
    architectures: frozenset[str]
    exported_symbols: frozenset[str] = frozenset()
    elf_load_alignments: tuple[int, ...] = ()
    macho_platforms: frozenset[int] = frozenset()
    macho_minimum_versions: tuple[tuple[str, tuple[int, int, int]], ...] = ()


@dataclass(frozen=True)
class NativeContract:
    format: str
    architectures: frozenset[str]
    minimum_elf_load_alignment: int | None = None
    apple_platform: str | None = None
    apple_variant: str | None = None
    apple_minimum_version: tuple[int, int, int] | None = None


@dataclass(frozen=True)
class XCFrameworkSlice:
    group: str
    identifier: str
    binary_path: str


NATIVE_CONTRACTS: Mapping[str, NativeContract] = MappingProxyType(
    {
        "android/arm64-v8a": NativeContract(
            "elf",
            frozenset(("arm64",)),
            ANDROID_MIN_LOAD_ALIGNMENT,
        ),
        "android/armeabi-v7a": NativeContract(
            "elf",
            frozenset(("armv7",)),
            ANDROID_MIN_LOAD_ALIGNMENT,
        ),
        "android/x86_64": NativeContract(
            "elf",
            frozenset(("x86_64",)),
            ANDROID_MIN_LOAD_ALIGNMENT,
        ),
        "ios/ios-arm64": NativeContract(
            "macho",
            frozenset(("arm64",)),
            apple_platform="ios",
            apple_minimum_version=(14, 0, 0),
        ),
        "ios/ios-arm64_x86_64-simulator": NativeContract(
            "macho",
            frozenset(("arm64", "x86_64")),
            apple_platform="ios",
            apple_variant="simulator",
            apple_minimum_version=(14, 0, 0),
        ),
        "macos/macos-arm64_x86_64": NativeContract(
            "macho",
            frozenset(("arm64", "x86_64")),
            apple_platform="macos",
            apple_minimum_version=(11, 0, 0),
        ),
        "linux/x86_64": NativeContract("elf", frozenset(("x86_64",))),
        "windows/x64": NativeContract("pe", frozenset(("x86_64",))),
    }
)


def expected_native_groups(platform: str) -> frozenset[str]:
    prefix = f"{platform}/"
    return frozenset(group for group in NATIVE_CONTRACTS if group.startswith(prefix))


def is_native_like_artifact(path: str, leading: bytes) -> bool:
    """Return whether a package file looks like a host-native artifact."""

    lower = path.lower()
    has_native_path = (
        any(
            part.endswith((".framework", ".xcframework"))
            for part in PurePosixPath(lower).parts
        )
        or lower.endswith(NATIVE_FILE_SUFFIXES)
        or ".so." in lower
    )
    return (
        has_native_path
        or leading.startswith(NATIVE_FILE_MAGICS)
        or leading[:2] in COFF_OBJECT_MACHINES
        or _is_coff_bigobj(leading)
    )


def _is_coff_bigobj(leading: bytes) -> bool:
    """Recognize LLVM/MSVC BigObj headers without trusting the file suffix."""

    return (
        len(leading) >= 28
        and leading[:4] == COFF_BIGOBJ_SIGNATURE
        and int.from_bytes(leading[4:6], "little") >= 2
        and leading[12:28] == COFF_BIGOBJ_CLASS_ID
    )


def is_reserved_flutter_native_path(path: str) -> bool:
    """Return whether a file is inside a platform native inventory root."""

    parts = PurePosixPath(path).parts
    return (
        len(parts) >= 5
        and parts[:4] == ("android", "src", "main", "jniLibs")
    ) or (
        len(parts) >= 3 and parts[:2] in (("linux", "lib"), ("windows", "lib"))
    )


def canonical_flutter_native_binary(path: str) -> tuple[str, str] | None:
    """Return the platform and contract group for a canonical native binary."""

    parts = PurePosixPath(path).parts
    if len(parts) == 6 and parts[:4] == (
        "android",
        "src",
        "main",
        "jniLibs",
    ) and parts[5] == "libmdstream_ffi.so":
        group = f"android/{parts[4]}"
        return ("android", group) if group in NATIVE_CONTRACTS else None
    if (
        len(parts) == 4
        and parts[:2] == ("linux", "lib")
        and parts[3] == "libmdstream_ffi.so"
    ):
        group = f"linux/{parts[2]}"
        return ("linux", group) if group in NATIVE_CONTRACTS else None
    if (
        len(parts) == 4
        and parts[:2] == ("windows", "lib")
        and parts[3] == "mdstream_ffi.dll"
    ):
        group = f"windows/{parts[2]}"
        return ("windows", group) if group in NATIVE_CONTRACTS else None
    if (
        len(parts) == 5
        and parts[0] in {"ios", "macos"}
        and parts[1] == "MdstreamFFI.xcframework"
        and parts[3:] == ("MdstreamFFI.framework", "MdstreamFFI")
    ):
        group = f"{parts[0]}/{parts[2]}"
        return (parts[0], group) if group in NATIVE_CONTRACTS else None
    return None


def is_canonical_flutter_native_path(path: str) -> bool:
    """Return whether a path belongs to the fixed Flutter native inventory."""

    if canonical_flutter_native_binary(path) is not None:
        return True
    parts = PurePosixPath(path).parts
    if (
        len(parts) == 3
        and parts[0] in {"ios", "macos"}
        and parts[1:] == ("MdstreamFFI.xcframework", "Info.plist")
    ):
        return True
    if (
        len(parts) < 5
        or parts[0] not in {"ios", "macos"}
        or parts[1] != "MdstreamFFI.xcframework"
        or parts[3] != "MdstreamFFI.framework"
        or f"{parts[0]}/{parts[2]}" not in NATIVE_CONTRACTS
    ):
        return False
    return parts[4:] in {
        ("Headers", "mdstream.h"),
        ("Modules", "module.modulemap"),
        ("Info.plist",),
    }


def inspect_xcframework(data: bytes, platform: str) -> tuple[XCFrameworkSlice, ...]:
    try:
        value = plistlib.loads(data)
    except (plistlib.InvalidFileException, ValueError, TypeError) as error:
        raise NativeArtifactError(f"invalid XCFramework property list: {error}") from error
    if not isinstance(value, dict):
        raise NativeArtifactError("XCFramework property list must contain a dictionary")
    if value.get("CFBundlePackageType") != "XFWK":
        raise NativeArtifactError("XCFramework package type must be XFWK")
    if value.get("XCFrameworkFormatVersion") != "1.0":
        raise NativeArtifactError("XCFramework format version must be 1.0")
    libraries = value.get("AvailableLibraries")
    if not isinstance(libraries, list):
        raise NativeArtifactError("XCFramework AvailableLibraries must be an array")

    slices: list[XCFrameworkSlice] = []
    seen: set[str] = set()
    for library in libraries:
        if not isinstance(library, dict):
            raise NativeArtifactError("XCFramework library metadata must be a dictionary")
        identifier = library.get("LibraryIdentifier")
        if not isinstance(identifier, str) or not identifier:
            raise NativeArtifactError("XCFramework library identifier must be a string")
        group = f"{platform}/{identifier}"
        contract = NATIVE_CONTRACTS.get(group)
        if contract is None or contract.apple_platform is None:
            raise NativeArtifactError(
                f"unexpected {platform} XCFramework identifier {identifier!r}"
            )
        if group in seen:
            raise NativeArtifactError(
                f"XCFramework repeats library identifier {identifier!r}"
            )
        seen.add(group)

        library_path = library.get("LibraryPath")
        if library_path != "MdstreamFFI.framework":
            raise NativeArtifactError(
                f"XCFramework {identifier} has unexpected LibraryPath {library_path!r}"
            )
        binary_path = library.get("BinaryPath")
        expected_binary = "MdstreamFFI.framework/MdstreamFFI"
        if binary_path != expected_binary:
            raise NativeArtifactError(
                f"XCFramework {identifier} has unexpected BinaryPath {binary_path!r}"
            )
        if library.get("SupportedPlatform") != contract.apple_platform:
            raise NativeArtifactError(
                f"XCFramework {identifier} has an unexpected supported platform"
            )
        if library.get("SupportedPlatformVariant") != contract.apple_variant:
            raise NativeArtifactError(
                f"XCFramework {identifier} has an unexpected platform variant"
            )
        architectures = library.get("SupportedArchitectures")
        if (
            not isinstance(architectures, list)
            or not all(isinstance(item, str) for item in architectures)
            or len(set(architectures)) != len(architectures)
            or frozenset(architectures) != contract.architectures
        ):
            raise NativeArtifactError(
                f"XCFramework {identifier} has unexpected supported architectures"
            )
        slices.append(
            XCFrameworkSlice(
                group=group,
                identifier=identifier,
                binary_path=f"{identifier}/{expected_binary}",
            )
        )

    expected = expected_native_groups(platform)
    if seen != expected:
        missing = sorted(expected - seen)
        unexpected = sorted(seen - expected)
        detail = []
        if missing:
            detail.append(f"missing {', '.join(missing)}")
        if unexpected:
            detail.append(f"unexpected {', '.join(unexpected)}")
        raise NativeArtifactError(
            f"{platform} XCFramework slice inventory mismatch: {'; '.join(detail)}"
        )
    return tuple(sorted(slices, key=lambda item: item.group))


def inspect_framework_info(data: bytes, contract: NativeContract) -> None:
    try:
        value = plistlib.loads(data)
    except (plistlib.InvalidFileException, ValueError, TypeError) as error:
        raise NativeArtifactError(f"invalid framework property list: {error}") from error
    if not isinstance(value, dict):
        raise NativeArtifactError("framework property list must contain a dictionary")
    expected_platform = {
        ("macos", None): "MacOSX",
        ("ios", None): "iPhoneOS",
        ("ios", "simulator"): "iPhoneSimulator",
    }.get((contract.apple_platform, contract.apple_variant))
    expected_minimum = contract.apple_minimum_version
    expected_version = (
        f"{expected_minimum[0]}.{expected_minimum[1]}"
        if expected_minimum is not None
        else None
    )
    required = {
        "CFBundleExecutable": "MdstreamFFI",
        "CFBundleIdentifier": "io.mdstream.flutter.MdstreamFFI",
        "CFBundleName": "MdstreamFFI",
        "CFBundlePackageType": "FMWK",
        "CFBundleSupportedPlatforms": [expected_platform],
        "MinimumOSVersion": expected_version,
    }
    mismatched = [
        key for key, expected in required.items() if value.get(key) != expected
    ]
    if mismatched:
        raise NativeArtifactError(
            "framework property list has unexpected " + ", ".join(mismatched)
        )


def inspect_native_image(data: bytes) -> NativeImage:
    if data.startswith(b"\x7fELF"):
        return _inspect_elf(data)
    if data.startswith(b"MZ"):
        return _inspect_pe(data)
    if data[:4] in {
        b"\xfe\xed\xfa\xce",
        b"\xce\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
        b"\xca\xfe\xba\xbf",
        b"\xbf\xba\xfe\xca",
    }:
        return _inspect_macho(data)
    raise NativeArtifactError("unrecognized native binary format")


def validate_native_image(data: bytes, contract: NativeContract) -> NativeImage:
    image = inspect_native_image(data)
    if image.format != contract.format:
        raise NativeArtifactError(
            f"expected {contract.format}, got {image.format}"
        )
    if image.architectures != contract.architectures:
        raise NativeArtifactError(
            "expected architecture(s) "
            f"{sorted(contract.architectures)}, got {sorted(image.architectures)}"
        )
    minimum = contract.minimum_elf_load_alignment
    if minimum is not None and any(
        alignment < minimum for alignment in image.elf_load_alignments
    ):
        raise NativeArtifactError(
            f"ELF LOAD alignment is below {minimum} bytes: "
            f"{image.elf_load_alignments}"
        )
    if contract.apple_platform is not None:
        platform = {
            ("macos", None): 1,
            ("ios", None): 2,
            ("ios", "simulator"): 7,
        }.get((contract.apple_platform, contract.apple_variant))
        if platform is None:
            raise NativeArtifactError("unsupported Apple platform contract")
        if image.macho_platforms != frozenset((platform,)):
            raise NativeArtifactError(
                f"expected Mach-O platform {platform}, got "
                f"{sorted(image.macho_platforms)}"
            )
        expected_minimum = contract.apple_minimum_version
        actual_minimums = dict(image.macho_minimum_versions)
        if expected_minimum is None or any(
            actual_minimums.get(architecture) != expected_minimum
            for architecture in contract.architectures
        ):
            raise NativeArtifactError(
                "expected Mach-O minimum OS version "
                f"{_version_text(expected_minimum)}, got "
                + ", ".join(
                    f"{architecture}={_version_text(version)}"
                    for architecture, version in sorted(actual_minimums.items())
                )
            )
    return image


def _inspect_elf(data: bytes) -> NativeImage:
    if len(data) < 16:
        raise NativeArtifactError("truncated ELF identification header")
    elf_class = data[4]
    data_encoding = data[5]
    if elf_class not in (1, 2) or data_encoding != 1 or data[6] != 1:
        raise NativeArtifactError("ELF must be a supported little-endian image")
    endian = "<"
    header_format = endian + (
        "HHIIIIIHHHHHH" if elf_class == 1 else "HHIQQQIHHHHHH"
    )
    header_size = 16 + struct.calcsize(header_format)
    if len(data) < header_size:
        raise NativeArtifactError("truncated ELF header")
    header = struct.unpack_from(header_format, data, 16)
    image_type = header[0]
    machine = header[1]
    version = header[2]
    program_offset = header[4]
    section_offset = header[5]
    encoded_header_size = header[7]
    program_entry_size = header[8]
    program_count = header[9]
    section_entry_size = header[10]
    section_count = header[11]
    if image_type != 3 or version != 1:
        raise NativeArtifactError("ELF image is not an ET_DYN shared object")
    if encoded_header_size != header_size:
        raise NativeArtifactError("ELF header size does not match its class")
    if program_count == 0 or program_count == 0xFFFF:
        raise NativeArtifactError("ELF has no directly encoded program headers")
    program_format = endian + ("IIIIIIII" if elf_class == 1 else "IIQQQQQQ")
    minimum_entry_size = struct.calcsize(program_format)
    if program_entry_size < minimum_entry_size:
        raise NativeArtifactError("ELF program-header entry is too small")
    end = program_offset + program_entry_size * program_count
    if program_offset < header_size or end > len(data):
        raise NativeArtifactError("truncated ELF program-header table")
    alignments: list[int] = []
    dynamic_segments = 0
    executable_load = False
    for index in range(program_count):
        values = struct.unpack_from(
            program_format,
            data,
            program_offset + index * program_entry_size,
        )
        segment_type = values[0]
        if elf_class == 1:
            offset, virtual_address = values[1], values[2]
            file_size, memory_size = values[4], values[5]
            flags, alignment = values[6], values[7]
        else:
            flags = values[1]
            offset, virtual_address = values[2], values[3]
            file_size, memory_size, alignment = values[5], values[6], values[7]
        if file_size > memory_size or offset > len(data) or file_size > len(data) - offset:
            raise NativeArtifactError("ELF segment extends outside the file")
        if alignment not in (0, 1) and (
            alignment & (alignment - 1) != 0
            or offset % alignment != virtual_address % alignment
        ):
            raise NativeArtifactError("ELF segment has invalid alignment congruence")
        if segment_type == 1:
            alignments.append(alignment)
            executable_load = executable_load or bool(flags & 1)
        elif segment_type == 2:
            dynamic_segments += 1
            _validate_elf_dynamic_segment(
                data,
                offset=offset,
                size=file_size,
                elf_class=elf_class,
                endian=endian,
            )
    if not alignments:
        raise NativeArtifactError("ELF contains no LOAD segments")
    if not executable_load or dynamic_segments != 1:
        raise NativeArtifactError(
            "ELF shared object must contain executable LOAD and one PT_DYNAMIC segment"
        )
    architectures = {
        (1, 40): "armv7",
        (2, 62): "x86_64",
        (2, 183): "arm64",
    }
    architecture = architectures.get((elf_class, machine))
    if architecture is None:
        raise NativeArtifactError(
            f"unsupported ELF class/machine combination {elf_class}/{machine}"
        )
    exported = _elf_dynamic_symbols(
        data,
        elf_class=elf_class,
        endian=endian,
        section_offset=section_offset,
        section_entry_size=section_entry_size,
        section_count=section_count,
    )
    return NativeImage(
        "elf",
        frozenset((architecture,)),
        frozenset(exported),
        tuple(alignments),
    )


def _validate_elf_dynamic_segment(
    data: bytes,
    *,
    offset: int,
    size: int,
    elf_class: int,
    endian: str,
) -> None:
    entry_format = endian + ("iI" if elf_class == 1 else "qQ")
    entry_size = struct.calcsize(entry_format)
    if size < entry_size or size % entry_size != 0:
        raise NativeArtifactError("ELF PT_DYNAMIC has an invalid size")
    terminated = False
    for cursor in range(offset, offset + size, entry_size):
        tag, _ = struct.unpack_from(entry_format, data, cursor)
        if tag == 0:
            terminated = True
            break
    if not terminated:
        raise NativeArtifactError("ELF PT_DYNAMIC has no DT_NULL terminator")


def _elf_dynamic_symbols(
    data: bytes,
    *,
    elf_class: int,
    endian: str,
    section_offset: int,
    section_entry_size: int,
    section_count: int,
) -> set[str]:
    section_format = endian + (
        "IIIIIIIIII" if elf_class == 1 else "IIQQQQIIQQ"
    )
    minimum_section_size = struct.calcsize(section_format)
    if (
        section_count == 0
        or section_count == 0xFFFF
        or section_entry_size < minimum_section_size
        or section_offset == 0
        or section_offset + section_entry_size * section_count > len(data)
    ):
        raise NativeArtifactError("ELF section table is missing or truncated")
    sections: list[tuple[int, ...]] = []
    for index in range(section_count):
        section = struct.unpack_from(
            section_format,
            data,
            section_offset + index * section_entry_size,
        )
        section_type = section[1]
        file_offset, size = section[4], section[5]
        if section_type != 8 and (
            file_offset > len(data) or size > len(data) - file_offset
        ):
            raise NativeArtifactError("ELF section extends outside the file")
        sections.append(section)

    dynamic_tables = [section for section in sections if section[1] == 11]
    if len(dynamic_tables) != 1:
        raise NativeArtifactError("ELF must contain exactly one SHT_DYNSYM table")
    symbols = dynamic_tables[0]
    symbol_offset, symbol_size = symbols[4], symbols[5]
    string_index, symbol_entry_size = symbols[6], symbols[9]
    minimum_symbol_size = 16 if elf_class == 1 else 24
    if (
        string_index >= len(sections)
        or sections[string_index][1] != 3
        or symbol_entry_size < minimum_symbol_size
        or symbol_size == 0
        or symbol_size % symbol_entry_size != 0
    ):
        raise NativeArtifactError("ELF dynamic symbol table is malformed")
    strings = sections[string_index]
    string_offset, string_size = strings[4], strings[5]
    string_data = data[string_offset : string_offset + string_size]
    symbol_format = endian + ("IIIBBH" if elf_class == 1 else "IBBHQQ")
    exported: set[str] = set()
    for cursor in range(symbol_offset, symbol_offset + symbol_size, symbol_entry_size):
        symbol = struct.unpack_from(symbol_format, data, cursor)
        if elf_class == 1:
            name_offset, info, other, section_index = (
                symbol[0],
                symbol[3],
                symbol[4],
                symbol[5],
            )
        else:
            name_offset, info, other, section_index = symbol[:4]
        if (
            name_offset == 0
            or name_offset >= len(string_data)
            or info >> 4 not in (1, 2)
            or other & 0x03 not in (0, 3)
            or section_index == 0
            or section_index >= len(sections)
        ):
            continue
        exported.add(_native_symbol_name(string_data, name_offset, "ELF"))
    if not exported:
        raise NativeArtifactError("ELF dynamic symbol table has no exports")
    return exported


def _native_symbol_name(data: bytes, offset: int, label: str) -> str:
    end = data.find(b"\0", offset)
    if end < 0 or end - offset > 4096:
        raise NativeArtifactError(f"{label} symbol name is not terminated")
    try:
        name = data[offset:end].decode("ascii")
    except UnicodeDecodeError as error:
        raise NativeArtifactError(f"{label} symbol name is not ASCII") from error
    if not name:
        raise NativeArtifactError(f"{label} symbol name is empty")
    return name


def _inspect_pe(data: bytes) -> NativeImage:
    if len(data) < 0x40:
        raise NativeArtifactError("truncated PE DOS header")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_offset < 0x40 or pe_offset + 24 > len(data) or data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise NativeArtifactError("invalid PE signature")
    (
        machine,
        section_count,
        _,
        _,
        _,
        optional_size,
        characteristics,
    ) = struct.unpack_from("<HHIIIHH", data, pe_offset + 4)
    architecture = {0x8664: "x86_64", 0xAA64: "arm64"}.get(machine)
    if architecture is None:
        raise NativeArtifactError(f"unsupported PE machine 0x{machine:04x}")
    if section_count == 0 or section_count > 96:
        raise NativeArtifactError("PE image has an invalid section count")
    if (characteristics & 0x2002) != 0x2002:
        raise NativeArtifactError("PE image is not an executable DLL")
    optional_offset = pe_offset + 24
    optional_end = optional_offset + optional_size
    if optional_size < 120 or optional_end > len(data):
        raise NativeArtifactError("PE optional header is truncated")
    if struct.unpack_from("<H", data, optional_offset)[0] != 0x20B:
        raise NativeArtifactError("PE DLL must use a PE32+ optional header")
    section_alignment, file_alignment = struct.unpack_from(
        "<II", data, optional_offset + 32
    )
    image_size, headers_size = struct.unpack_from("<II", data, optional_offset + 56)
    directory_count = struct.unpack_from("<I", data, optional_offset + 108)[0]
    if (
        section_alignment == 0
        or file_alignment == 0
        or image_size == 0
        or headers_size > len(data)
        or directory_count == 0
    ):
        raise NativeArtifactError("PE optional header has invalid image geometry")
    export_rva, export_size = struct.unpack_from("<II", data, optional_offset + 112)
    if export_rva == 0 or export_size < 40:
        raise NativeArtifactError("PE DLL has no export directory")

    section_table = optional_end
    section_table_end = section_table + section_count * 40
    if section_table_end > len(data) or headers_size < section_table_end:
        raise NativeArtifactError("PE section table is truncated")
    sections: list[tuple[int, int, int, int]] = []
    for index in range(section_count):
        offset = section_table + index * 40
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from(
            "<IIII", data, offset + 8
        )
        if raw_offset > len(data) or raw_size > len(data) - raw_offset:
            raise NativeArtifactError("PE section extends outside the file")
        sections.append((virtual_address, virtual_size, raw_offset, raw_size))
    exported = _pe_exported_symbols(
        data,
        export_rva=export_rva,
        export_size=export_size,
        sections=sections,
        headers_size=headers_size,
    )
    return NativeImage(
        "pe",
        frozenset((architecture,)),
        frozenset(exported),
    )


def _pe_file_range(
    rva: int,
    size: int,
    *,
    data_size: int,
    sections: list[tuple[int, int, int, int]],
    headers_size: int,
) -> tuple[int, int]:
    if rva < headers_size:
        if size > headers_size - rva or rva + size > data_size:
            raise NativeArtifactError("PE RVA extends beyond its headers")
        return rva, rva + size
    for virtual_address, virtual_size, raw_offset, raw_size in sections:
        span = max(virtual_size, raw_size)
        if virtual_address <= rva < virtual_address + span:
            relative = rva - virtual_address
            if relative > raw_size or size > raw_size - relative:
                raise NativeArtifactError("PE RVA extends beyond section data")
            start = raw_offset + relative
            if start > data_size or size > data_size - start:
                raise NativeArtifactError("PE RVA extends outside the file")
            return start, start + size
    raise NativeArtifactError("PE RVA is not mapped by any section")


def _pe_exported_symbols(
    data: bytes,
    *,
    export_rva: int,
    export_size: int,
    sections: list[tuple[int, int, int, int]],
    headers_size: int,
) -> set[str]:
    export_offset, _ = _pe_file_range(
        export_rva,
        export_size,
        data_size=len(data),
        sections=sections,
        headers_size=headers_size,
    )
    (
        _,
        _,
        _,
        _,
        _,
        _,
        function_count,
        name_count,
        function_rva,
        name_rva,
        ordinal_rva,
    ) = struct.unpack_from("<IIHHIIIIIII", data, export_offset)
    if (
        function_count == 0
        or name_count == 0
        or name_count > function_count
        or function_count > 1_000_000
    ):
        raise NativeArtifactError("PE export directory has invalid counts")
    function_offset, _ = _pe_file_range(
        function_rva,
        function_count * 4,
        data_size=len(data),
        sections=sections,
        headers_size=headers_size,
    )
    name_offset, _ = _pe_file_range(
        name_rva,
        name_count * 4,
        data_size=len(data),
        sections=sections,
        headers_size=headers_size,
    )
    ordinal_offset, _ = _pe_file_range(
        ordinal_rva,
        name_count * 2,
        data_size=len(data),
        sections=sections,
        headers_size=headers_size,
    )
    exported: set[str] = set()
    for index in range(name_count):
        ordinal = struct.unpack_from("<H", data, ordinal_offset + index * 2)[0]
        if ordinal >= function_count:
            raise NativeArtifactError("PE export ordinal is outside the function table")
        target = struct.unpack_from("<I", data, function_offset + ordinal * 4)[0]
        if target == 0:
            raise NativeArtifactError("PE named export has no function RVA")
        if export_rva <= target < export_rva + export_size:
            raise NativeArtifactError("PE forwarded exports are not supported")
        _pe_file_range(
            target,
            1,
            data_size=len(data),
            sections=sections,
            headers_size=headers_size,
        )
        symbol_rva = struct.unpack_from("<I", data, name_offset + index * 4)[0]
        symbol_offset, _ = _pe_file_range(
            symbol_rva,
            1,
            data_size=len(data),
            sections=sections,
            headers_size=headers_size,
        )
        exported.add(_native_symbol_name(data, symbol_offset, "PE"))
    if not exported:
        raise NativeArtifactError("PE export directory has no named exports")
    return exported


def _inspect_macho(data: bytes) -> NativeImage:
    magic = data[:4]
    thin_endian = {
        b"\xce\xfa\xed\xfe": "<",
        b"\xcf\xfa\xed\xfe": "<",
        b"\xfe\xed\xfa\xce": ">",
        b"\xfe\xed\xfa\xcf": ">",
    }.get(magic)
    if thin_endian is not None:
        architecture, platform, minimum, exported = _inspect_macho_slice(
            data, thin_endian
        )
        return NativeImage(
            "macho",
            frozenset((architecture,)),
            exported_symbols=frozenset(exported),
            macho_platforms=frozenset((platform,)),
            macho_minimum_versions=((architecture, minimum),),
        )

    fat_layout = {
        b"\xca\xfe\xba\xbe": (">", 20),
        b"\xbe\xba\xfe\xca": ("<", 20),
        b"\xca\xfe\xba\xbf": (">", 32),
        b"\xbf\xba\xfe\xca": ("<", 32),
    }.get(magic)
    if fat_layout is None or len(data) < 8:
        raise NativeArtifactError("invalid Mach-O fat header")
    endian, entry_size = fat_layout
    count = struct.unpack_from(endian + "I", data, 4)[0]
    if count == 0 or count > 32 or 8 + count * entry_size > len(data):
        raise NativeArtifactError("invalid Mach-O fat architecture table")
    architectures: set[str] = set()
    platforms: set[int] = set()
    minimum_versions: dict[str, tuple[int, int, int]] = {}
    common_exports: set[str] | None = None
    ranges: list[tuple[int, int]] = []
    table_end = 8 + count * entry_size
    for index in range(count):
        entry_offset = 8 + index * entry_size
        if entry_size == 20:
            cpu_type, _, offset, size, alignment = struct.unpack_from(
                endian + "IIIII", data, entry_offset
            )
        else:
            cpu_type, _, offset, size, alignment, _ = struct.unpack_from(
                endian + "IIQQII", data, entry_offset
            )
        end = offset + size
        if (
            size == 0
            or offset < table_end
            or end > len(data)
            or alignment > 63
            or offset % (1 << alignment) != 0
            or any(offset < previous_end and start < end for start, previous_end in ranges)
        ):
            raise NativeArtifactError("invalid Mach-O fat slice range")
        ranges.append((offset, end))
        slice_data = data[offset:end]
        slice_endian = {
            b"\xce\xfa\xed\xfe": "<",
            b"\xcf\xfa\xed\xfe": "<",
            b"\xfe\xed\xfa\xce": ">",
            b"\xfe\xed\xfa\xcf": ">",
        }.get(slice_data[:4])
        if slice_endian is None:
            raise NativeArtifactError("fat Mach-O contains a non-Mach-O slice")
        architecture, platform, minimum, exported = _inspect_macho_slice(
            slice_data, slice_endian
        )
        if architecture != _macho_architecture(cpu_type):
            raise NativeArtifactError("fat Mach-O slice CPU disagrees with its table")
        architectures.add(architecture)
        platforms.add(platform)
        minimum_versions[architecture] = minimum
        common_exports = (
            set(exported)
            if common_exports is None
            else common_exports & exported
        )
    if len(architectures) != count:
        raise NativeArtifactError("Mach-O fat binary repeats an architecture")
    return NativeImage(
        "macho",
        frozenset(architectures),
        exported_symbols=frozenset(common_exports or set()),
        macho_platforms=frozenset(platforms),
        macho_minimum_versions=tuple(sorted(minimum_versions.items())),
    )


def _inspect_macho_slice(
    data: bytes, endian: str
) -> tuple[str, int, tuple[int, int, int], set[str]]:
    is_64_bit = data[:4] in {b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf"}
    header_format = endian + ("IIIIIII" if is_64_bit else "IIIIII")
    header_size = 4 + struct.calcsize(header_format)
    if len(data) < header_size:
        raise NativeArtifactError("truncated Mach-O header")
    header = struct.unpack_from(header_format, data, 4)
    cpu_type = header[0]
    file_type = header[2]
    command_count = header[3]
    command_bytes = header[4]
    if file_type != 6:
        raise NativeArtifactError("Mach-O image is not an MH_DYLIB")
    if command_count == 0 or command_count > 65535:
        raise NativeArtifactError("Mach-O has an invalid load-command count")
    command_end = header_size + command_bytes
    if command_end > len(data):
        raise NativeArtifactError("truncated Mach-O load-command table")

    offset = header_size
    build_versions: list[tuple[int, tuple[int, int, int]]] = []
    symbol_tables: list[tuple[int, int, int, int]] = []
    dylib_ids = 0
    for _ in range(command_count):
        if offset + 8 > command_end:
            raise NativeArtifactError("truncated Mach-O load command")
        command, command_size = struct.unpack_from(endian + "II", data, offset)
        if command_size < 8 or offset + command_size > command_end:
            raise NativeArtifactError("invalid Mach-O load-command size")
        if command == 0x32:
            if command_size < 24:
                raise NativeArtifactError("truncated LC_BUILD_VERSION command")
            platform, minimum = struct.unpack_from(endian + "II", data, offset + 8)
            build_versions.append((platform, _decode_packed_version(minimum)))
        elif command == 0x2:
            if command_size < 24:
                raise NativeArtifactError("truncated LC_SYMTAB command")
            symbol_tables.append(
                struct.unpack_from(endian + "IIII", data, offset + 8)
            )
        elif command == 0xD:
            if command_size < 24:
                raise NativeArtifactError("truncated LC_ID_DYLIB command")
            dylib_ids += 1
        offset += command_size
    if offset != command_end:
        raise NativeArtifactError("Mach-O load-command sizes do not match the header")
    if len(build_versions) != 1:
        raise NativeArtifactError("Mach-O must contain exactly one LC_BUILD_VERSION")
    if len(symbol_tables) != 1 or dylib_ids != 1:
        raise NativeArtifactError(
            "Mach-O dylib must contain exactly one LC_SYMTAB and LC_ID_DYLIB"
        )
    platform, minimum = build_versions[0]
    exported = _macho_exported_symbols(
        data,
        endian=endian,
        is_64_bit=is_64_bit,
        symbol_table=symbol_tables[0],
    )
    return _macho_architecture(cpu_type), platform, minimum, exported


def _macho_exported_symbols(
    data: bytes,
    *,
    endian: str,
    is_64_bit: bool,
    symbol_table: tuple[int, int, int, int],
) -> set[str]:
    symbol_offset, symbol_count, string_offset, string_size = symbol_table
    symbol_format = endian + ("IBBHQ" if is_64_bit else "IBBHI")
    symbol_size = struct.calcsize(symbol_format)
    if (
        symbol_count == 0
        or symbol_count > 1_000_000
        or symbol_offset > len(data)
        or symbol_count * symbol_size > len(data) - symbol_offset
        or string_size == 0
        or string_offset > len(data)
        or string_size > len(data) - string_offset
    ):
        raise NativeArtifactError("Mach-O symbol table is truncated")
    strings = data[string_offset : string_offset + string_size]
    exported: set[str] = set()
    for index in range(symbol_count):
        name_offset, symbol_type, _, _, _ = struct.unpack_from(
            symbol_format,
            data,
            symbol_offset + index * symbol_size,
        )
        if (
            name_offset == 0
            or name_offset >= len(strings)
            or symbol_type & 0xE0
            or symbol_type & 0x01 == 0
            or symbol_type & 0x0E == 0
        ):
            continue
        name = _native_symbol_name(strings, name_offset, "Mach-O")
        exported.add(name[1:] if name.startswith("_") else name)
    if not exported:
        raise NativeArtifactError("Mach-O symbol table has no exports")
    return exported


def _decode_packed_version(value: int) -> tuple[int, int, int]:
    return value >> 16, (value >> 8) & 0xFF, value & 0xFF


def _version_text(version: tuple[int, int, int] | None) -> str:
    if version is None:
        return "undefined"
    return ".".join(str(part) for part in version)


def _macho_architecture(cpu_type: int) -> str:
    architecture = {
        0x01000007: "x86_64",
        0x0100000C: "arm64",
    }.get(cpu_type)
    if architecture is None:
        raise NativeArtifactError(f"unsupported Mach-O CPU type 0x{cpu_type:08x}")
    return architecture
