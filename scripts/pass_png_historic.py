#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run pass-png over many days' worth of spectrograms, selecting the best TLE per group.

Recognises two filename schemes produced by the strf toolkit:

  rffft output  — YYYY-MM-DDTHH:MM:SS_NNNNNN.bin
      All files sharing the same datetime prefix (i.e. from the same rffft
      recording session) are grouped together and processed as one pass-png run.

  rsmedfilt output — mf_YYYY-MM-DDTHH:MM:SS_NNNNNN.bin
      Each file is treated as its own group (no grouping needed).

For each group the TLE whose epoch most closely matches the start time of the
first spectrogram is selected from the historic TLE archive, written to a
temporary file, and passed to pass-png.

Usage example:
  scripts/pass_png_historic.py \\
      --tle 58340.tle -i 58340 -o out/pass \\
      ~/GDrive/RadioDecoding/spectrograms/*.bin \\
      -- -f 437450000 --zmin 0
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

HEADER_SIZE = 256
# rffft output: YYYY-MM-DDTHH:MM:SS_NNNNNN.bin — group by the datetime prefix.
RFFFT_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})_\d+\.bin$")


# ---------------------------------------------------------------------------
# .bin header parsing
# ---------------------------------------------------------------------------


def read_first_start_time(path: Path) -> datetime:
    """Return UTC_START from the first spectrum header in a .bin file."""
    with path.open("rb") as f:
        header_bytes = f.read(HEADER_SIZE)
    header = header_bytes.decode("ascii", errors="replace")
    m = re.search(r"UTC_START\s+(\S+)", header)
    if not m:
        raise ValueError(f"No UTC_START in first header of {path}")
    ts = m.group(1).rstrip("Z")
    for fmt in ("%Y-%m-%dT%H:%M:%S.%f", "%Y-%m-%dT%H:%M:%S"):
        try:
            return datetime.strptime(ts, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            pass
    raise ValueError(f"Cannot parse UTC_START {ts!r} in {path}")


# ---------------------------------------------------------------------------
# TLE parsing
# ---------------------------------------------------------------------------


def parse_tle_epoch(line1: str) -> datetime:
    """Convert the epoch field from TLE line 1 to a UTC datetime."""
    epoch_str = line1[18:32].strip()
    year2 = int(epoch_str[:2])
    year = (2000 + year2) if year2 < 57 else (1900 + year2)
    day_frac = float(epoch_str[2:])
    day = int(day_frac)
    frac = day_frac - day
    return datetime(year, 1, 1, tzinfo=timezone.utc) + timedelta(days=day - 1 + frac)


def parse_tles(tle_file: Path) -> list[tuple[str, str, str]]:
    """Parse a file containing multiple 2LE/3LE TLEs.

    Returns a list of (title, line1, line2) tuples.
    """
    tles: list[tuple[str, str, str]] = []
    raw_lines = tle_file.read_text().splitlines()
    lines = [l.rstrip() for l in raw_lines]
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if not line:
            i += 1
            continue
        if line.startswith("1 ") or line.startswith("2 "):
            # Orphaned TLE line — skip
            i += 1
            continue
        # Title line (with or without leading "0 ")
        if line.startswith("0 "):
            title = line[2:].strip()
        else:
            title = line
        if i + 2 >= len(lines):
            break
        l1 = lines[i + 1].strip()
        l2 = lines[i + 2].strip()
        if l1.startswith("1 ") and l2.startswith("2 "):
            tles.append((title, l1, l2))
            i += 3
        else:
            i += 1
    return tles


def best_tle_for(
    tles: list[tuple[str, str, str]], target: datetime
) -> tuple[str, str, str]:
    """Return the TLE whose epoch is closest in time to *target*."""

    def delta(tle: tuple[str, str, str]) -> float:
        return abs((parse_tle_epoch(tle[1]) - target).total_seconds())

    return min(tles, key=delta)


# ---------------------------------------------------------------------------
# Grouping
# ---------------------------------------------------------------------------


def group_files(bin_files: list[Path]) -> dict[str, list[Path]]:
    """Group rffft files by shared datetime prefix; everything else is its own group."""
    groups: dict[str, list[Path]] = {}
    for path in bin_files:
        m = RFFFT_RE.match(path.name)
        if m:
            key = m.group(1)  # e.g. "2026-05-20T09:46:37"
        else:
            # mf_* files and anything else: each file is its own group.
            key = f"\x00{path.stem}"  # leading NUL sorts these after rffft groups
        groups.setdefault(key, []).append(path)
    return groups


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    # Split on '--' before argparse so everything after it is forwarded verbatim.
    argv = sys.argv[1:]
    try:
        sep = argv.index("--")
        our_argv, passthrough = argv[:sep], argv[sep + 1 :]
    except ValueError:
        our_argv, passthrough = argv, []

    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "spectrograms",
        nargs="+",
        type=Path,
        help=".bin spectrogram files",
    )
    parser.add_argument(
        "--tle",
        required=True,
        type=Path,
        metavar="TLE_FILE",
        help="Historic TLE archive file (one or more TLEs)",
    )
    parser.add_argument(
        "-i",
        "--norad-id",
        required=True,
        help="NORAD catalog number (forwarded to pass-png)",
    )
    parser.add_argument(
        "-o",
        "--output",
        required=True,
        help="Output path prefix; group key is appended (e.g. out/pass → out/pass_20240101)",
    )
    parser.add_argument(
        "--rstrf",
        default="rstrf",
        metavar="PATH",
        help="Path to rstrf binary [default: rstrf]",
    )
    parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Print commands without executing them",
    )

    args = parser.parse_args(our_argv)

    tles = parse_tles(args.tle)
    if not tles:
        print(f"ERROR: No TLEs found in {args.tle}", file=sys.stderr)
        sys.exit(1)
    print(f"Loaded {len(tles)} TLE(s) from {args.tle}")

    groups = group_files(args.spectrograms)
    print(f"Found {len(groups)} group(s) from {len(args.spectrograms)} file(s)")

    tmpdir = tempfile.mkdtemp(prefix="pass_png_historic_")
    try:
        exit_code = 0
        for group_key, files in sorted(groups.items()):
            files_sorted = sorted(files)
            first_file = files_sorted[0]

            try:
                start_time = read_first_start_time(first_file)
            except Exception as exc:
                print(
                    f"\nWARNING: Skipping group {group_key!r}: {exc}", file=sys.stderr
                )
                continue

            tle = best_tle_for(tles, start_time)
            tle_epoch = parse_tle_epoch(tle[1])
            delta_h = (tle_epoch - start_time).total_seconds() / 3600

            # Build a safe suffix for the output prefix and temp-TLE filename.
            safe_key = re.sub(r"[^\w\-]", "_", group_key.lstrip("\x00"))
            output_prefix = f"{args.output}_{safe_key}"

            print(
                f"\nGroup {group_key.lstrip(chr(0))!r}: {len(files)} file(s), "
                f"start={start_time.isoformat()}"
            )
            print(f"  TLE epoch : {tle_epoch.isoformat()}  (Δ={delta_h:+.1f} h)")
            print(f"  Output    : {output_prefix}_NNN.png")

            tle_path = os.path.join(tmpdir, f"{safe_key}.tle")
            with open(tle_path, "w") as tf:
                tf.write(f"0 {tle[0]}\n{tle[1]}\n{tle[2]}\n")

            cmd = (
                [
                    args.rstrf,
                    "pass-png",
                    "--catalog",
                    tle_path,
                    "--norad-id",
                    args.norad_id,
                    "--output",
                    output_prefix,
                ]
                + passthrough
                + [str(p) for p in files_sorted]
            )

            print(f"  Command   : {' '.join(cmd)}")

            if not args.dry_run:
                result = subprocess.run(cmd)
                if result.returncode != 0:
                    print(
                        f"  ERROR: pass-png exited with code {result.returncode} "
                        f"for group {group_key.lstrip(chr(0))!r}",
                        file=sys.stderr,
                    )
                    exit_code = 1

    finally:
        import shutil

        shutil.rmtree(tmpdir, ignore_errors=True)

    sys.exit(exit_code)


if __name__ == "__main__":
    main()
