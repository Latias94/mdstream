"""Deterministic native images shared by packaging contract tests."""

from __future__ import annotations

import struct


REQUIRED_SYMBOL_TEXT = (
    b"mdstream_abi_version|mdstream_package_version|"
    b"mdstream_engine_new|mdstream_reducer_new"
)


def required_symbols() -> tuple[str, ...]:
    return (
        "mdstream_abi_version",
        "mdstream_package_version",
        "mdstream_engine_new",
        "mdstream_reducer_new",
    )


def elf_image(
    architecture: str,
    *,
    alignment: int = 16 * 1024,
    export_section_index: int = 1,
) -> bytes:
    """Build a minimal valid shared ELF image with the mdstream FFI exports."""

    machine = {"armv7": 40, "x86_64": 62, "arm64": 183}[architecture]
    elf_class = 1 if architecture == "armv7" else 2
    header_size, program_size, symbol_size, section_size = (
        (52, 32, 16, 40) if elf_class == 1 else (64, 56, 24, 64)
    )
    program_offset = header_size
    dynamic_offset = program_offset + 2 * program_size
    dynamic = struct.pack("<iI" if elf_class == 1 else "<qQ", 0, 0)
    text_offset = dynamic_offset + len(dynamic)
    strings = b"\0" + b"\0".join(
        symbol.encode("ascii") for symbol in required_symbols()
    ) + b"\0"
    string_offset = text_offset + 1
    symbol_offset = (string_offset + len(strings) + 7) & ~7
    string_positions = {
        symbol: strings.index(symbol.encode("ascii"))
        for symbol in required_symbols()
    }
    symbols = bytearray(b"\0" * symbol_size)
    for symbol in required_symbols():
        if elf_class == 1:
            symbols.extend(
                struct.pack(
                    "<IIIBBH",
                    string_positions[symbol],
                    0,
                    1,
                    0x12,
                    0,
                    export_section_index,
                )
            )
        else:
            symbols.extend(
                struct.pack(
                    "<IBBHQQ",
                    string_positions[symbol],
                    0x12,
                    0,
                    export_section_index,
                    0,
                    1,
                )
            )
    section_offset = (symbol_offset + len(symbols) + 7) & ~7
    total_size = section_offset + 4 * section_size
    image = bytearray(b"\0" * total_size)
    image[:16] = b"\x7fELF" + bytes((elf_class, 1, 1, 0)) + b"\0" * 8
    if elf_class == 1:
        struct.pack_into(
            "<HHIIIIIHHHHHH",
            image,
            16,
            3,
            machine,
            1,
            0,
            program_offset,
            section_offset,
            0,
            header_size,
            program_size,
            2,
            section_size,
            4,
            0,
        )
        load = struct.pack(
            "<IIIIIIII", 1, 0, 0, 0, total_size, total_size, 5, alignment
        )
        dynamic_program = struct.pack(
            "<IIIIIIII",
            2,
            dynamic_offset,
            dynamic_offset,
            dynamic_offset,
            len(dynamic),
            len(dynamic),
            4,
            4,
        )
        section_format = "<IIIIIIIIII"
    else:
        struct.pack_into(
            "<HHIQQQIHHHHHH",
            image,
            16,
            3,
            machine,
            1,
            0,
            program_offset,
            section_offset,
            0,
            header_size,
            program_size,
            2,
            section_size,
            4,
            0,
        )
        load = struct.pack(
            "<IIQQQQQQ", 1, 5, 0, 0, 0, total_size, total_size, alignment
        )
        dynamic_program = struct.pack(
            "<IIQQQQQQ",
            2,
            4,
            dynamic_offset,
            dynamic_offset,
            dynamic_offset,
            len(dynamic),
            len(dynamic),
            8,
        )
        section_format = "<IIQQQQIIQQ"
    image[program_offset : program_offset + program_size] = load
    image[program_offset + program_size : program_offset + 2 * program_size] = (
        dynamic_program
    )
    image[dynamic_offset : dynamic_offset + len(dynamic)] = dynamic
    image[text_offset] = 0xC3
    image[string_offset : string_offset + len(strings)] = strings
    image[symbol_offset : symbol_offset + len(symbols)] = symbols
    sections = (
        (0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
        (0, 1, 6, text_offset, text_offset, 1, 0, 0, 1, 0),
        (0, 3, 2, string_offset, string_offset, len(strings), 0, 0, 1, 0),
        (
            0,
            11,
            2,
            symbol_offset,
            symbol_offset,
            len(symbols),
            2,
            1,
            8 if elf_class == 2 else 4,
            symbol_size,
        ),
    )
    for index, section in enumerate(sections):
        struct.pack_into(
            section_format,
            image,
            section_offset + index * section_size,
            *section,
        )
    return bytes(image)
