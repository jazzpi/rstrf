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

Before invoking pass-png, the selected TLE is propagated (via skyfield) to check
whether the satellite rises above --min-elevation at the observer site during the
group's recording window. Groups with no pass are skipped, avoiding the expensive
spectrogram load. The observer site is read from rstrf's config.json (overridable
with --lat/--lon/--alt); pass --no-pass-filter to disable this and process every
group.

Usage example:
  scripts/pass_png_historic.py \\
      --tle 58340.tle -i 58340 -o out/pass \\
      ~/GDrive/RadioDecoding/spectrograms/*.bin \\
      -- -f 437450000 --zmin 0
"""

import argparse
import json
import math
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
# Sampling step (seconds) when scanning a recording window for a pass. Passes
# last minutes, so this is fine-grained enough to never miss one.
PASS_STEP_S = 30


# ---------------------------------------------------------------------------
# .bin header parsing
# ---------------------------------------------------------------------------


def _parse_utc(ts: str, path: Path) -> datetime:
    ts = ts.rstrip("Z")
    for fmt in ("%Y-%m-%dT%H:%M:%S.%f", "%Y-%m-%dT%H:%M:%S"):
        try:
            return datetime.strptime(ts, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            pass
    raise ValueError(f"Cannot parse UTC_START {ts!r} in {path}")


def read_header(path: Path) -> dict:
    """Parse the first 256-byte header of a .bin file.

    Returns a dict with keys ``utc_start`` (datetime), ``length`` (float seconds
    per sub-integration or None) and ``nsub`` (int or None).
    """
    with path.open("rb") as f:
        header_bytes = f.read(HEADER_SIZE)
    header = header_bytes.decode("ascii", errors="replace")
    m = re.search(r"UTC_START\s+(\S+)", header)
    if not m:
        raise ValueError(f"No UTC_START in first header of {path}")
    length = re.search(r"LENGTH\s+([0-9.]+)", header)
    nsub = re.search(r"NSUB\s+(\d+)", header)
    return {
        "utc_start": _parse_utc(m.group(1), path),
        "length": float(length.group(1)) if length else None,
        "nsub": int(nsub.group(1)) if nsub else None,
    }


def group_time_window(files_sorted: list[Path]) -> tuple[datetime, datetime]:
    """Return (start, end) UTC covering the group's recording.

    ``start`` is the first file's UTC_START; ``end`` is the last file's UTC_START
    plus its NSUB * LENGTH duration when those fields are present.
    """
    start = read_header(files_sorted[0])["utc_start"]
    last = read_header(files_sorted[-1])
    end = last["utc_start"]
    if last["nsub"] is not None and last["length"] is not None:
        end = end + timedelta(seconds=last["nsub"] * last["length"])
    return start, max(start, end)


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
# Observer site & pass prediction
# ---------------------------------------------------------------------------


def default_config_path() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return Path(base) / "rstrf" / "config.json"


def resolve_site(args) -> dict:
    """Resolve observer lat/lon (degrees) and altitude (metres).

    Reads rstrf's config.json (site stored as radians / km) and applies any
    --lat/--lon/--alt overrides (degrees / metres).
    """
    lat = lon = alt = None
    config_path = args.config or default_config_path()
    try:
        cfg = json.loads(Path(config_path).read_text())
        site = cfg.get("site")
        if site:
            lat = math.degrees(site["latitude"])
            lon = math.degrees(site["longitude"])
            alt = site["altitude"] * 1000.0  # km -> m
    except FileNotFoundError:
        if args.config is not None:
            raise
    except (KeyError, ValueError, TypeError) as exc:
        print(
            f"WARNING: Could not read site from {config_path}: {exc}", file=sys.stderr
        )

    if args.lat is not None:
        lat = args.lat
    if args.lon is not None:
        lon = args.lon
    if args.alt is not None:
        alt = args.alt

    if lat is None or lon is None:
        raise SystemExit(
            "ERROR: No observer site available. Provide --lat/--lon (and --alt), "
            f"or set 'site' in {config_path}."
        )
    return {"lat": lat, "lon": lon, "alt": alt if alt is not None else 0.0}


def has_pass(
    tle: tuple[str, str, str],
    site: dict,
    start: datetime,
    end: datetime,
    min_elevation: float,
) -> tuple[bool, float]:
    """Return (has_pass, max_elevation_deg) over [start, end] using skyfield.

    Raises ImportError if skyfield is not installed.
    """
    from skyfield.api import EarthSatellite, load, wgs84

    ts = load.timescale()
    sat = EarthSatellite(tle[1], tle[2], tle[0], ts)
    observer = wgs84.latlon(site["lat"], site["lon"], elevation_m=site["alt"])

    n = int((end - start).total_seconds() // PASS_STEP_S) + 1
    samples = [start + timedelta(seconds=i * PASS_STEP_S) for i in range(n)]
    if samples[-1] < end:
        samples.append(end)

    t = ts.from_datetimes(samples)
    alt, _az, _dist = (sat - observer).at(t).altaz()
    max_elev = float(alt.degrees.max())
    return max_elev >= min_elevation, max_elev


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
    parser.add_argument(
        "--min-elevation",
        type=float,
        default=0.0,
        metavar="DEG",
        help="Minimum elevation (deg) for a pass to count [default: 0]",
    )
    parser.add_argument(
        "--no-pass-filter",
        action="store_true",
        help="Disable pass prediction; process every group",
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=None,
        metavar="PATH",
        help="rstrf config.json to read site from "
        "[default: $XDG_CONFIG_HOME/rstrf/config.json]",
    )
    parser.add_argument(
        "--lat", type=float, default=None, help="Observer latitude override (deg)"
    )
    parser.add_argument(
        "--lon", type=float, default=None, help="Observer longitude override (deg)"
    )
    parser.add_argument(
        "--alt", type=float, default=None, help="Observer altitude override (m)"
    )

    args = parser.parse_args(our_argv)

    tles = parse_tles(args.tle)
    unfiltered_tles = len(tles)

    def norad_id(tle: tuple[str, str, str]) -> int:
        field = tle[1].split(None, 3)[1].strip()
        return int(field.rstrip("UCS"))

    tles = list(filter(lambda tle: norad_id(tle) == int(args.norad_id), tles))
    print(f"Loaded {len(tles)}/{unfiltered_tles} TLE(s) from {args.tle}")
    if not tles:
        print(
            f"ERROR: No TLEs found for NORAD ID {args.norad_id} in {args.tle}",
            file=sys.stderr,
        )
        sys.exit(1)

    site = None
    if not args.no_pass_filter:
        site = resolve_site(args)
        print(
            f"Observer site: lat={site['lat']:.4f}° lon={site['lon']:.4f}° "
            f"alt={site['alt']:.0f} m  (min elevation {args.min_elevation:.1f}°)"
        )

    groups = group_files(args.spectrograms)
    print(f"Found {len(groups)} group(s) from {len(args.spectrograms)} file(s)")

    tmpdir = tempfile.mkdtemp(prefix="pass_png_historic_")
    try:
        exit_code = 0
        processed = 0
        skipped = 0
        for group_key, files in sorted(groups.items()):
            files_sorted = sorted(files)

            try:
                start_time, end_time = group_time_window(files_sorted)
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
                f"{start_time.isoformat()} → {end_time.isoformat()}"
            )
            print(f"  TLE epoch : {tle_epoch.isoformat()}  (Δ={delta_h:+.1f} h)")

            if site is not None:
                try:
                    keep, max_elev = has_pass(
                        tle, site, start_time, end_time, args.min_elevation
                    )
                except ImportError:
                    print(
                        "ERROR: skyfield not installed. Run 'pip install skyfield' "
                        "or pass --no-pass-filter.",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                except Exception as exc:
                    print(
                        f"  WARNING: pass prediction failed ({exc}); "
                        "processing group anyway",
                        file=sys.stderr,
                    )
                    keep, max_elev = True, None
                if max_elev is not None:
                    print(f"  Max elev  : {max_elev:.1f}°")
                if not keep:
                    print("  SKIP: no pass in recording window")
                    skipped += 1
                    continue

            processed += 1
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

    print(
        f"\nDone: {processed} processed, {skipped} skipped (no pass) "
        f"of {len(groups)} group(s)"
    )
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
