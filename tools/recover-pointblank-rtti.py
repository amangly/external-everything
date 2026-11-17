#!/usr/bin/env python3
"""Recover evidence-bound x86 MSVC RTTI/vtable candidates from saved sections.

Usage: python tools/recover-pointblank-rtti.py EVIDENCE.json [--output NEW.json]

This tool reads files only. It never opens a process, performs a hypercall,
executes captured bytes, changes a production profile, or admits a new hash.
Input section offsets and read_gaps offsets are section-file-relative. Failed
reads were zero-filled by the producer; every candidate touching such a gap is
rejected. A gap with no length/size field covers the producer's 4096-byte page.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import struct
import sys
from typing import Any

ADDRESS_LIMIT = 1 << 32
PAGE_SIZE = 4096
MAX_CAPTURE_BYTES = 256 * 1024 * 1024
NAME_PATTERN = re.compile(rb"\.\?A[UV][\x21-\x7e]{1,508}?@@\x00")
WANTED = re.compile(r"framework|camera|avatar|chara|matrix|view|project", re.I)
SHA256 = re.compile(r"[0-9a-fA-F]{64}\Z")


class ValidationError(ValueError):
    pass


class MissingBytes(ValidationError):
    pass


def integer(value: Any, label: str) -> int:
    if isinstance(value, bool):
        raise ValidationError(f"{label}: boolean is not an integer")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value, 16 if value.lower().startswith("0x") else 10)
        except ValueError:
            pass
    raise ValidationError(f"{label}: expected integer or decimal/0x string")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def no_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValidationError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


@dataclass(frozen=True)
class Section:
    name: str
    rva: int
    data: bytes
    path: str
    sha256: str
    gaps: tuple[tuple[int, int, str], ...]

    @property
    def end(self) -> int:
        return self.rva + len(self.data)

    def check(self, offset: int, length: int) -> None:
        if offset < 0 or length < 0 or offset + length > len(self.data):
            raise MissingBytes(f"{self.name}: range outside captured section")
        for start, end, error in self.gaps:
            if offset < end and offset + length > start:
                raise MissingBytes(
                    f"{self.name}+{offset:#x}/{length:#x} intersects missing "
                    f"page {start:#x}..{end:#x}: {error}"
                )


@dataclass
class Image:
    base: int
    size: int
    sections: list[Section]
    uncaptured: list[dict[str, Any]]

    def locate(self, va: int, length: int) -> tuple[Section, int]:
        rva = va - self.base
        if va < 0x10000 or length < 0 or rva < 0 or rva + length > self.size:
            raise MissingBytes(f"VA {va:#x}/{length:#x} outside recorded x86 image")
        for section in self.sections:
            if section.rva <= rva and rva + length <= section.end:
                offset = rva - section.rva
                section.check(offset, length)
                return section, offset
        raise MissingBytes(f"VA {va:#x}/{length:#x} lacks captured section backing")

    def read(self, va: int, length: int) -> bytes:
        section, offset = self.locate(va, length)
        return section.data[offset:offset + length]

    def words(self, va: int, count: int) -> tuple[int, ...]:
        return struct.unpack(f"<{count}I", self.read(va, count * 4))

    def source(self, va: int, length: int) -> dict[str, Any]:
        section, offset = self.locate(va, length)
        return {
            "section": section.name,
            "file": section.path,
            "file_sha256": section.sha256,
            "file_offset": offset,
            "length": length,
            "rva": va - self.base,
            "va": va,
        }


def load_evidence(path: Path) -> tuple[Image, dict[str, Any], str]:
    raw = path.read_bytes()
    if len(raw) > 8 * 1024 * 1024:
        raise ValidationError("evidence JSON exceeds 8 MiB")
    evidence = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=no_duplicate_keys)
    if not isinstance(evidence, dict):
        raise ValidationError("evidence root must be an object")
    for key in ("pid", "eprocess", "cr3", "image_base", "image_size", "path", "disk_sha256", "sections"):
        if key not in evidence:
            raise ValidationError(f"missing evidence field: {key}")
    pid = integer(evidence["pid"], "pid")
    if not 0 < pid < ADDRESS_LIMIT:
        raise ValidationError("pid is outside uint32 range")
    base = integer(evidence["image_base"], "image_base")
    size = integer(evidence["image_size"], "image_size")
    if base < 0x10000 or size <= 0 or base + size > ADDRESS_LIMIT:
        raise ValidationError("image range is outside x86 pointer bounds")
    disk_hash = evidence["disk_sha256"]
    if not isinstance(disk_hash, str) or not SHA256.fullmatch(disk_hash):
        raise ValidationError("disk_sha256 must be exactly 64 hexadecimal digits")
    entries = evidence["sections"]
    if not isinstance(entries, list) or not 1 <= len(entries) <= 96:
        raise ValidationError("sections must contain between 1 and 96 records")
    sections = []
    uncaptured = []
    ranges = []
    total = 0
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise ValidationError(f"section {index} is not an object")
        name = entry.get("name")
        filename = entry.get("file")
        expected_hash = entry.get("sha256")
        if not isinstance(name, str) or not name:
            raise ValidationError(f"section {index}: missing name")
        rva = integer(entry.get("rva"), f"section {name} rva")
        length = integer(entry.get("size"), f"section {name} size")
        if rva < 0 or length <= 0 or rva + length > size:
            raise ValidationError(f"section {name}: invalid range")
        ranges.append((rva, rva + length, name))
        if filename is None and expected_hash is None:
            if entry.get("read_gaps"):
                raise ValidationError(f"section {name}: gaps declared without a captured file")
            uncaptured.append({"name": name, "rva": rva, "size": length,
                               "status": "metadata-only; bytes were not captured"})
            continue
        if not isinstance(filename, str) or not filename:
            raise ValidationError(f"section {name}: missing captured filename")
        if not isinstance(expected_hash, str) or not SHA256.fullmatch(expected_hash):
            raise ValidationError(f"section {name}: invalid SHA-256")
        total += length
        if total > MAX_CAPTURE_BYTES:
            raise ValidationError("captured sections exceed 256 MiB")
        source = Path(filename)
        if not source.is_absolute():
            source = path.parent / source
        source = source.resolve(strict=True)
        if source.stat().st_size != length:
            raise ValidationError(f"section {name}: file length differs from declared size")
        data = source.read_bytes()
        actual_hash = digest(data)
        if actual_hash.lower() != expected_hash.lower():
            raise ValidationError(f"section {name}: SHA-256 mismatch")
        raw_gaps = entry.get("read_gaps", [])
        if not isinstance(raw_gaps, list):
            raise ValidationError(f"section {name}: read_gaps must be an array")
        gaps = []
        for gap in raw_gaps:
            if not isinstance(gap, dict) or "offset" not in gap or "error" not in gap:
                raise ValidationError(f"section {name}: gap requires offset/error")
            offset = integer(gap["offset"], f"section {name} gap offset")
            gap_size = integer(gap.get("length", gap.get("size", PAGE_SIZE)), "gap length")
            if offset < 0 or offset >= length or gap_size <= 0:
                raise ValidationError(f"section {name}: gap outside captured range")
            end = min(length, offset + gap_size)
            if any(data[offset:end]):
                raise ValidationError(f"section {name}: declared zero-filled gap has nonzero bytes")
            gaps.append((offset, end, str(gap["error"])))
        sections.append(Section(name, rva, data, str(source), actual_hash, tuple(sorted(gaps))))
    sections.sort(key=lambda section: section.rva)
    ranges.sort()
    for left, right in zip(ranges, ranges[1:]):
        if left[1] > right[0]:
            raise ValidationError(f"overlapping section ranges: {left[2]}/{right[2]}")
    if not sections:
        raise ValidationError("evidence has zero captured section files")
    return Image(base, size, sections, uncaptured), evidence, digest(raw)


def occurrences(data: bytes, value: bytes):
    offset = 0
    while True:
        offset = data.find(value, offset)
        if offset < 0:
            return
        yield offset
        offset += 1


def category(name: str) -> str:
    lower = name.lower()
    if "chara" in lower and ("manager" in lower or "mgr" in lower):
        return "character_manager"
    if "framework" in lower:
        return "framework"
    if "avatar" in lower:
        return "avatar"
    if "project" in lower:
        return "projection"
    if "view" in lower:
        return "view"
    if "camera" in lower:
        return "camera"
    if "matrix" in lower:
        return "matrix"
    return "character"


def analyze(image: Image) -> dict[str, Any]:
    descriptors = []
    rejected = []
    candidates: dict[int, dict[str, Any]] = {}
    rdata = [section for section in image.sections if section.name.lower().startswith(".rdata")]
    if not rdata:
        raise ValidationError("RTTI cross-reference search requires captured .rdata")

    def reject(stage: str, va: int, name: str, reason: Exception | str) -> None:
        # Bounded detail; counts remain exact if the diagnostic list is truncated.
        nonlocal rejected_count
        rejected_count += 1
        if len(rejected) < 4096:
            rejected.append({"stage": stage, "rva": va - image.base, "name": name, "reason": str(reason)})

    rejected_count = 0
    for section in image.sections:
        for match in NAME_PATTERN.finditer(section.data):
            name = match.group()[:-1].decode("ascii")
            if not WANTED.search(name):
                continue
            name_va = image.base + section.rva + match.start()
            descriptor_va = name_va - 8
            if descriptor_va & 3:
                continue
            try:
                vfptr, spare = image.words(descriptor_va, 2)
                if not 0x10000 <= vfptr < ADDRESS_LIMIT or spare != 0:
                    raise ValidationError("invalid MSVC TypeDescriptor vfptr/spare")
                descriptor_source = image.source(descriptor_va, 8 + len(match.group()))
            except ValidationError as error:
                reject("type_descriptor", descriptor_va, name, error)
                continue
            descriptor = {
                "name": name,
                "simple_name": name[4:-2].split("@")[0],
                "category": category(name),
                "type_descriptor_rva": descriptor_va - image.base,
                "type_info_vfptr": vfptr,
                "source": descriptor_source,
                "complete_object_locators": [],
                "vtable_rvas": [],
            }
            descriptors.append(descriptor)
            wanted_descriptor = struct.pack("<I", descriptor_va)
            for references in rdata:
                for reference in occurrences(references.data, wanted_descriptor):
                    col_va = image.base + references.rva + reference - 12
                    if col_va & 3:
                        continue
                    try:
                        signature, displacement, cd_offset, typedesc, hierarchy = image.words(col_va, 5)
                        if signature != 0 or displacement != 0 or typedesc != descriptor_va:
                            continue
                        if cd_offset > image.size:
                            raise ValidationError("implausible constructor displacement")
                        col_source = image.source(col_va, 20)
                        h_signature, attributes, base_count, base_array = image.words(hierarchy, 4)
                        if h_signature != 0 or attributes & ~7 or not 1 <= base_count <= 1024:
                            raise ValidationError("invalid x86 class hierarchy header")
                        hierarchy_source = image.source(hierarchy, 16)
                        base_array_source = image.source(base_array, base_count * 4)
                        base_descriptor = image.words(base_array, 1)[0]
                        # BaseClassDescriptor starts with a TypeDescriptor pointer.
                        if image.words(base_descriptor, 1)[0] != descriptor_va:
                            raise ValidationError("hierarchy's first base differs from target type")
                        base_descriptor_source = image.source(base_descriptor, 4)
                    except ValidationError as error:
                        reject("complete_object_locator", col_va, name, error)
                        continue
                    descriptor["complete_object_locators"].append(col_va - image.base)
                    wanted_col = struct.pack("<I", col_va)
                    for vtables in rdata:
                        for slot in occurrences(vtables.data, wanted_col):
                            slot_va = image.base + vtables.rva + slot
                            vtable_va = slot_va + 4
                            if slot_va & 3:
                                continue
                            try:
                                slot_source = image.source(slot_va, 8)
                                first_function = image.words(vtable_va, 1)[0]
                                code, code_offset = image.locate(first_function, 16)
                                if not code.name.lower().startswith(".text"):
                                    raise ValidationError("first virtual function lacks captured .text backing")
                                code_bytes = code.data[code_offset:code_offset + 16]
                                if code_bytes == bytes(16) or code_bytes == b"\xff" * 16:
                                    raise ValidationError("first virtual function is a zero/FF placeholder")
                                first_function_source = image.source(first_function, 16)
                            except ValidationError as error:
                                reject("vtable", vtable_va, name, error)
                                continue
                            functions = []
                            for index in range(8):
                                try:
                                    value = image.words(vtable_va + index * 4, 1)[0]
                                    target, _ = image.locate(value, 1)
                                    if not target.name.lower().startswith(".text"):
                                        break
                                    functions.append(value - image.base)
                                except ValidationError:
                                    break
                            candidate = {
                                "name": name,
                                "simple_name": descriptor["simple_name"],
                                "category": descriptor["category"],
                                "vtable_rva": vtable_va - image.base,
                                "vtable_va": vtable_va,
                                "col_rva": col_va - image.base,
                                "type_descriptor_rva": descriptor_va - image.base,
                                "hierarchy_rva": hierarchy - image.base,
                                "base_count": base_count,
                                "constructor_displacement": cd_offset,
                                "first_function_rva": first_function - image.base,
                                "sample_function_rvas": functions,
                                "first_function_bytes": code_bytes.hex(),
                                "status": "RTTI-backed candidate; live object/field semantics unverified",
                                "gap_coverage": "all interpreted ranges have captured, nongap backing",
                                "sources": {
                                    "type_descriptor": descriptor_source,
                                    "complete_object_locator": col_source,
                                    "class_hierarchy": hierarchy_source,
                                    "base_class_array": base_array_source,
                                    "first_base_descriptor": base_descriptor_source,
                                    "col_pointer_and_first_vtable_entry": slot_source,
                                    "first_function": first_function_source,
                                },
                            }
                            prior = candidates.get(candidate["vtable_rva"])
                            if prior and prior["type_descriptor_rva"] != candidate["type_descriptor_rva"]:
                                raise ValidationError("one vtable has conflicting type descriptor identities")
                            candidates[candidate["vtable_rva"]] = candidate
                            descriptor["vtable_rvas"].append(candidate["vtable_rva"])
    for descriptor in descriptors:
        descriptor["complete_object_locators"] = sorted(set(descriptor["complete_object_locators"]))
        descriptor["vtable_rvas"] = sorted(set(descriptor["vtable_rvas"]))
    ordered = sorted(candidates.values(), key=lambda item: (item["category"], item["name"], item["vtable_rva"]))
    by_category: dict[str, list[int]] = {}
    for candidate in ordered:
        by_category.setdefault(candidate["category"], []).append(candidate["vtable_rva"])
    return {
        "candidate_count": len(ordered),
        "candidates": ordered,
        "candidate_rvas_by_category": by_category,
        "type_descriptor_count": len(descriptors),
        "type_descriptors": sorted(descriptors, key=lambda item: (item["category"], item["name"])),
        "rejected_count": rejected_count,
        "rejected_details_truncated": rejected_count > len(rejected),
        "rejected": rejected,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, help="root capture's evidence.json")
    parser.add_argument("--output", type=Path, help="optional NEW JSON file; existing files stay untouched")
    args = parser.parse_args()
    try:
        evidence_path = args.evidence.resolve(strict=True)
        image, evidence, evidence_hash = load_evidence(evidence_path)
        result = {
            "schema_version": 1,
            "kind": "pointblank-x86-msvc-rtti-offline-candidates",
            "admitted_profile": False,
            "evidence_file": str(evidence_path),
            "evidence_sha256": evidence_hash,
            "snapshot": {key: evidence[key] for key in (
                "pid", "eprocess", "cr3", "image_base", "image_size", "path", "disk_sha256"
            )},
            "section_validation": [{
                "name": section.name, "rva": section.rva, "size": len(section.data),
                "file": section.path, "sha256": section.sha256, "digest_verified": True,
                "gap_ranges": [{"offset": start, "length": end - start, "error": error}
                               for start, end, error in section.gaps],
            } for section in image.sections],
            "uncaptured_sections": image.uncaptured,
            "search_boundary": {
                "type_names": "MSVC .?AV/.?AU names matching Framework/Camera/Avatar/Chara(including Character)/Matrix/View/Project",
                "xref_sections": ".rdata* only",
                "complete_object_locator": "x86 signature=0, offset=0; primary vtables only",
                "gap_policy": "reject every candidate touching a missing page or uncaptured required range",
            },
            "limitations": [
                "This analyzes a recorded, potentially non-atomic memory snapshot, rather than a live target.",
                "SHA-256 validates supplied section files; producer/process/disk identity is retained evidence metadata.",
                "RTTI proves structural type/vtable candidates, rather than camera/health/vector field semantics.",
                "First-function .text backing is byte evidence, rather than executed or decoded instruction evidence.",
                "Secondary-base vtables and type names outside the keyword search remain outside this bounded pass.",
                "Offsets and production hash gates remain unchanged by this tool.",
            ],
        }
        result.update(analyze(image))
        encoded = json.dumps(result, indent=2, ensure_ascii=True) + "\n"
        if args.output:
            # Exclusive creation prevents replacement of evidence or another run.
            with args.output.open("x", encoding="utf-8", newline="\n") as output:
                output.write(encoded)
        sys.stdout.write(encoded)
        return 0
    except (OSError, ValueError, KeyError, struct.error) as error:
        sys.stderr.write(json.dumps({"error": type(error).__name__, "message": str(error)}) + "\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
