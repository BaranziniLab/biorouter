#!/usr/bin/env python3
"""Compute M-21 precision from owner labels without printing candidate terms."""
import argparse
import csv
import json


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("candidates_file")
    args = parser.parse_args()
    with open(args.candidates_file, newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    if any(row.get("owner_label") not in {"yes", "no"} for row in rows):
        print(json.dumps({"candidates": len(rows), "labeled": sum(row.get("owner_label") in {"yes", "no"} for row in rows), "gate": "pending_labels"}))
        return
    yes = sum(row["owner_label"] == "yes" for row in rows)
    precision = yes / len(rows) if rows else None
    print(json.dumps({"candidates": len(rows), "true_positive": yes, "precision": precision, "gate": "pass" if precision is not None and precision >= 0.8 else "fail"}))


if __name__ == "__main__":
    main()
