#!/usr/bin/env python3
"""Generate LOOM's deterministic, synthetic adversarial PDF corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


GENERATOR_VERSION = "loom-pdf-adversarial-v1"
EXTRACTOR_ID = "loom.pdf"
EXTRACTOR_VERSION = "0.1.0"


def pdf_literal(text: str) -> bytes:
    return b"(" + text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)").encode("ascii") + b")"


def text_stream(lines: list[tuple[int, int, str]], *, tagged: bool = False) -> bytes:
    chunks = [b"BT", b"/F1 12 Tf"]
    for index, (x, y, text) in enumerate(lines):
        chunks.extend([f"{x} {y} Td".encode(), b"/Span" if tagged and index == 0 else b"", b"<</MCID 0>> BDC" if tagged and index == 0 else b"", pdf_literal(text), b"Tj", b"EMC" if tagged and index == 0 else b""])
    chunks.extend([b"ET"])
    return b"\n".join(chunk for chunk in chunks if chunk)


def build_pdf_stable(
    pages: list[dict[str, object]],
    *,
    tagged: bool = False,
    encrypted_marker: bool = False,
) -> bytes:
    """Build a small PDF with explicit object numbers and a deterministic xref."""

    # Object numbers are allocated in their final order: catalog, pages, font,
    # page/content pairs, optional images, then the tagged-PDF structures.
    objects: list[bytes | None] = [None, None, None, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    font_number = 3
    page_numbers: list[int] = []
    content_numbers: list[int] = []
    image_numbers: list[int | None] = []
    for page in pages:
        page_numbers.append(len(objects))
        objects.append(None)
        content_numbers.append(len(objects))
        objects.append(None)
        if page.get("image"):
            image_numbers.append(len(objects))
            objects.append(None)
        else:
            image_numbers.append(None)

    struct_root = struct_elem = parent_tree = None
    if tagged:
        struct_root = len(objects)
        objects.append(None)
        struct_elem = len(objects)
        objects.append(None)
        parent_tree = len(objects)
        objects.append(None)

    for index, page in enumerate(pages):
        image_number = image_numbers[index]
        if image_number is not None:
            image = bytes(page["image"])
            objects[image_number] = (
                b"<< /Type /XObject /Subtype /Image /Width 16 /Height 16 "
                b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Length "
                + str(len(image)).encode()
                + b" >>\nstream\n"
                + image
                + b"\nendstream"
            )
            content = b"q 16 0 0 16 72 720 cm /Im0 Do Q"
            resources = f"/XObject << /Im0 {image_number} 0 R >>".encode()
        else:
            content = text_stream(page["lines"], tagged=tagged and index == 0)
            resources = f"/Font << /F1 {font_number} 0 R >>".encode()
        objects[content_numbers[index]] = (
            b"<< /Length "
            + str(len(content)).encode()
            + b" >>\nstream\n"
            + content
            + b"\nendstream"
        )
        page_dict = (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << "
            + resources
            + b" >> /Contents "
            + str(content_numbers[index]).encode()
            + b" 0 R"
        )
        if page.get("rotate"):
            page_dict += b" /Rotate 90"
        if tagged and index == 0:
            page_dict += b" /StructParents 0"
        objects[page_numbers[index]] = page_dict + b" >>"

    kids = b" ".join(f"{number} 0 R".encode() for number in page_numbers)
    objects[2] = f"<< /Type /Pages /Kids [{kids.decode()}] /Count {len(pages)} >>".encode()
    catalog = b"<< /Type /Catalog /Pages 2 0 R"
    if tagged:
        catalog += f" /MarkInfo << /Marked true >> /StructTreeRoot {struct_root} 0 R".encode()
        objects[struct_root] = (
            f"<< /Type /StructTreeRoot /K [ {struct_elem} 0 R ] /ParentTree {parent_tree} 0 R /ParentTreeNextKey 1 >>".encode()
        )
        objects[struct_elem] = (
            f"<< /Type /StructElem /S /P /P {struct_root} 0 R /K [<< /Type /MCR /Pg {page_numbers[0]} 0 R /MCID 0 >>] >>".encode()
        )
        objects[parent_tree] = f"<< /Nums [0 [ {struct_elem} 0 R ]] >>".encode()
    objects[1] = catalog + b" >>"

    pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n"
    offsets = [0] * len(objects)
    for number in range(1, len(objects)):
        payload = objects[number]
        if payload is None:
            raise ValueError(f"object {number} was not populated")
        offsets[number] = len(pdf)
        pdf += f"{number} 0 obj\n".encode() + payload + b"\nendobj\n"
    xref_offset = len(pdf)
    pdf += f"xref\n0 {len(objects)}\n".encode()
    pdf += b"0000000000 65535 f \n"
    for offset in offsets[1:]:
        pdf += f"{offset:010} 00000 n \n".encode()
    pdf += f"trailer\n<< /Size {len(objects)} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n".encode()
    if encrypted_marker:
        pdf += b"% /Encrypt 7 0 R\n"
    return pdf


def fixture_specs() -> list[dict[str, object]]:
    image = bytes([240, 244, 248] * 256)
    return [
        {
            "id": "tagged-text",
            "class": "tagged_text",
            "filename": "tagged-text.pdf",
            "outcome": "indexed",
            "expected_page_count": 1,
            "expected_pages": [1],
            "expected_warnings": [],
            "expected_contains": "tagged evidence marker",
            "pdf": build_pdf_stable(
                [{"lines": [(72, 720, "tagged evidence marker")]}], tagged=True
            ),
        },
        {
            "id": "multi-column",
            "class": "multi_column",
            "filename": "multi-column.pdf",
            "outcome": "indexed",
            "expected_page_count": 1,
            "expected_pages": [1],
            "expected_warnings": [],
            "expected_contains": "right column marker",
            "pdf": build_pdf_stable(
                [
                    {
                        "lines": [
                            (72, 720, "left column marker and first reading lane"),
                            (320, 720, "right column marker and second reading lane"),
                        ]
                    }
                ]
            ),
        },
        {
            "id": "ligature",
            "class": "ligature",
            "filename": "ligature.pdf",
            "outcome": "indexed",
            "expected_page_count": 1,
            "expected_pages": [1],
            "expected_warnings": [],
            "expected_contains": "office fi ligature marker",
            "pdf": build_pdf_stable(
                [{"lines": [(72, 720, "office fi ligature marker")]}]
            ),
        },
        {
            "id": "rotated-page",
            "class": "rotated_page",
            "filename": "rotated-page.pdf",
            "outcome": "indexed",
            "expected_page_count": 1,
            "expected_pages": [1],
            "expected_warnings": [],
            "expected_contains": "rotated page marker",
            "pdf": build_pdf_stable(
                [{"lines": [(72, 720, "rotated page marker")], "rotate": True}]
            ),
        },
        {
            "id": "encrypted",
            "class": "encrypted",
            "filename": "encrypted.pdf",
            "outcome": "unsupported",
            "expected_page_count": None,
            "expected_pages": [],
            "expected_warnings": [],
            "expected_error_contains": "encrypted PDF",
            "pdf": build_pdf_stable(
                [{"lines": [(72, 720, "encrypted marker must not enter the index")]}],
                encrypted_marker=True,
            ),
        },
        {
            "id": "malformed",
            "class": "malformed",
            "filename": "malformed.pdf",
            "outcome": "unsupported",
            "expected_page_count": None,
            "expected_pages": [],
            "expected_warnings": [],
            "expected_error_contains": "malformed PDF",
            "pdf": b"%PDF-1.7\n% deterministic malformed fixture\n1 0 obj\n<< /Type /Catalog >>\n",
        },
        {
            "id": "image-only",
            "class": "image_only",
            "filename": "image-only.pdf",
            "outcome": "unsupported",
            "expected_page_count": None,
            "expected_pages": [],
            "expected_warnings": [],
            "expected_error_contains": "no extractable text",
            "pdf": build_pdf_stable([{"image": image}]),
        },
    ]


def write_corpus(output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    fixtures = []
    for spec in fixture_specs():
        path = output / str(spec["filename"])
        path.write_bytes(bytes(spec["pdf"]))
        fixture = {key: value for key, value in spec.items() if key not in {"pdf", "filename"}}
        fixture["path"] = path.name
        fixture["byte_hash"] = hashlib.sha256(path.read_bytes()).hexdigest()
        fixture["hash_algorithm"] = "sha256"
        fixture["extractor_id"] = EXTRACTOR_ID
        fixture["extractor_version"] = EXTRACTOR_VERSION
        fixtures.append(fixture)
    manifest = {
        "schema_version": 1,
        "name": "loom-pdf-adversarial-v1",
        "license": "CC0-1.0",
        "generator": GENERATOR_VERSION,
        "extractor": {"id": EXTRACTOR_ID, "version": EXTRACTOR_VERSION},
        "fixtures": fixtures,
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("benchmarks/pdf-adversarial/corpus"),
    )
    args = parser.parse_args()
    write_corpus(args.output)
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
