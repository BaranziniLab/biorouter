#!/usr/bin/env python3
"""Summarize a Crew CSV without assuming fixture values."""
from __future__ import annotations

import csv
import sys
from pathlib import Path


def summarize(input_csv: Path, output_csv: Path) -> tuple[int, int]:
    with input_csv.open(newline="", encoding="utf-8") as source:
        reader = csv.DictReader(source)
        fields = reader.fieldnames or []
        required = {"sample_id", "value"}
        missing = required.difference(fields)
        if missing:
            raise ValueError(f"missing required columns: {sorted(missing)}")
        row_count = 0
        total = 0
        for row_number, row in enumerate(reader, start=2):
            if None in row:
                raise ValueError(f"row {row_number} has extra fields")
            raw_value = row.get("value")
            if raw_value is None or not raw_value.strip():
                raise ValueError(f"row {row_number} has an empty value")
            try:
                value = int(raw_value.strip())
            except ValueError as error:
                raise ValueError(f"row {row_number} value is not an integer") from error
            row_count += 1
            total += value

    with output_csv.open("w", newline="", encoding="utf-8") as destination:
        writer = csv.writer(destination, lineterminator="\n")
        writer.writerow(("row_count", "total"))
        writer.writerow((row_count, total))
    return row_count, total


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} INPUT_CSV OUTPUT_CSV")
    count, total = summarize(Path(sys.argv[1]), Path(sys.argv[2]))
    print(f"row_count={count} total={total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
