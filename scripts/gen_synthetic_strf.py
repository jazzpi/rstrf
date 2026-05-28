#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate synthetic STRF `.bin` files for testing rstrf.

Format (matches `src/spectrogram.rs`):
    Per spectrum: 256-byte ASCII header (space-padded), followed by
    `nchan` f32 little-endian linear power values.

    Header text:
        HEADER
        UTC_START    YYYY-MM-DDTHH:MM:SS.mmm
        FREQ         {freq} Hz
        BW           {bw} Hz
        LENGTH       {length} s
        NCHAN        {nchan}
        NSUB         {nsub}
        END
"""

import argparse
import math
import random
import struct
from datetime import datetime, timedelta, timezone
from pathlib import Path

import numpy as np


HEADER_SIZE = 256


def format_header(start: datetime, freq: float, bw: float, length: float,
                  nchan: int, nsub: int) -> bytes:
    # rstrf strips the trailing Z, so produce ISO without timezone designator.
    ts = start.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + \
        f"{start.microsecond // 1000:03d}"
    text = (
        "HEADER\n"
        f"UTC_START    {ts}\n"
        f"FREQ         {freq} Hz\n"
        f"BW           {bw} Hz\n"
        f"LENGTH       {length} s\n"
        f"NCHAN        {nchan}\n"
        f"NSUB         {nsub}\n"
        "END\n"
    )
    encoded = text.encode("ascii")
    if len(encoded) > HEADER_SIZE:
        raise ValueError(f"Header too large: {len(encoded)} > {HEADER_SIZE}")
    return encoded + b" " * (HEADER_SIZE - len(encoded))


class Signal:
    """A time-limited signal occupying part of the spectrum."""

    def __init__(self, t_start: float, t_end: float,
                 f_start_hz: float, drift_hz_per_s: float,
                 amplitude: float, width_hz: float):
        self.t_start = t_start
        self.t_end = t_end
        self.f_start_hz = f_start_hz   # offset from center, in Hz
        self.drift_hz_per_s = drift_hz_per_s
        self.amplitude = amplitude     # peak linear power
        self.width_hz = width_hz       # Gaussian sigma in Hz

    def add_to(self, spectrum: np.ndarray, t: float, freqs_hz: np.ndarray) -> None:
        if t < self.t_start or t > self.t_end:
            return
        f = self.f_start_hz + self.drift_hz_per_s * (t - self.t_start)
        spectrum += self.amplitude * np.exp(
            -0.5 * ((freqs_hz - f) / self.width_hz) ** 2
        )


def make_signals(rng: random.Random, total_length: float, bw: float,
                 num_signals: int, noise_floor: float) -> list[Signal]:
    signals = []
    for _ in range(num_signals):
        duration = rng.uniform(0.1, 0.6) * total_length
        t_start = rng.uniform(0.0, max(0.0, total_length - duration))
        t_end = t_start + duration

        # Offset from center, leaving margin so signal doesn't fall off the edge.
        f_start = rng.uniform(-0.4 * bw, 0.4 * bw)

        # Half the signals are constant-frequency, half drift.
        if rng.random() < 0.5:
            drift = 0.0
        else:
            max_drift = 0.3 * bw / max(duration, 1e-6)
            drift = rng.uniform(-max_drift, max_drift)

        amplitude = noise_floor * rng.uniform(5.0, 50.0)
        width_hz = bw / 200.0 * rng.uniform(0.5, 3.0)

        signals.append(Signal(t_start, t_end, f_start, drift, amplitude, width_hz))
    return signals


def _jittered_dt(nominal_dt: float, jitter_frac: float, jitter_bias: float,
                 rng: random.Random) -> float:
    return nominal_dt * (1.0 + jitter_bias + rng.uniform(-jitter_frac, jitter_frac))


def write_file(path: Path, start: datetime, num_spectra: int, total_length: float,
               nchan: int, freq: float, bw: float,
               noise_floor: float, noise_std: float,
               jitter_frac: float, jitter_bias: float, num_signals: int,
               rng: random.Random) -> datetime:
    """Write one .bin file; returns the timestamp of the last spectrum written."""
    nominal_dt = total_length / num_spectra
    freqs_hz = np.linspace(-bw / 2, bw / 2, nchan, endpoint=False)
    signals = make_signals(rng, total_length, bw, num_signals, noise_floor)

    # Per-channel noise floor variation (bandpass shape).
    bandpass = noise_floor * (1.0 + 0.3 * np.cos(
        np.linspace(-math.pi / 2, math.pi / 2, nchan)
    ))

    last_time = start
    with path.open("wb") as f:
        spectrum_time = start
        for i in range(num_spectra):
            if i > 0:
                spectrum_time += timedelta(
                    seconds=_jittered_dt(nominal_dt, jitter_frac, jitter_bias, rng)
                )

            t = (spectrum_time - start).total_seconds()
            spectrum = bandpass + rng_gaussian(nchan, noise_std, rng)
            for sig in signals:
                sig.add_to(spectrum, t, freqs_hz)
            np.maximum(spectrum, 1e-12, out=spectrum)

            header = format_header(spectrum_time, freq, bw,
                                   nominal_dt, nchan, num_spectra)
            f.write(header)
            f.write(spectrum.astype("<f4").tobytes())
            last_time = spectrum_time

    return last_time


def rng_gaussian(n: int, std: float, rng: random.Random) -> np.ndarray:
    # Use numpy's RNG seeded from the python rng for reproducibility.
    seed = rng.getrandbits(32)
    return np.random.default_rng(seed).normal(0.0, std, size=n)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("-o", "--output", required=True, type=Path,
                        help="Output file base name (e.g. 'synth' → synth_000.bin, ...)")
    parser.add_argument("-n", "--num-spectra", type=int, default=1000,
                        help="Number of spectra per output file")
    parser.add_argument("--num-files", type=int, default=1,
                        help="Number of output files to generate")
    parser.add_argument("--total-length", type=float, default=60.0,
                        help="Total duration covered by one file, in seconds")
    parser.add_argument("--nchan", type=int, default=1024,
                        help="Number of frequency channels")
    parser.add_argument("--freq", type=float, default=437e6,
                        help="Center frequency in Hz")
    parser.add_argument("--bw", type=float, default=100e3,
                        help="Bandwidth in Hz")
    parser.add_argument("--noise-floor", type=float, default=1.0,
                        help="Mean linear power of the noise floor")
    parser.add_argument("--noise-std", type=float, default=0.2,
                        help="Standard deviation of per-channel noise (linear power)")
    parser.add_argument("--jitter", type=float, default=0.05,
                        help="Fractional jitter on inter-spectrum interval (0 = none)")
    parser.add_argument("--jitter-bias", type=float, default=0.0,
                        help="Fractional bias added to every interval (e.g. 0.02 → always ~2%% longer, simulating processing overhead)")
    parser.add_argument("--num-signals", type=int, default=3,
                        help="Number of synthetic signals per file")
    parser.add_argument("--seed", type=int, default=None,
                        help="RNG seed (default: nondeterministic)")
    parser.add_argument("--start-time", type=str, default=None,
                        help="ISO 8601 start time (default: now, UTC)")
    args = parser.parse_args()

    rng = random.Random(args.seed)
    start = (datetime.fromisoformat(args.start_time)
             if args.start_time else datetime.now(timezone.utc))
    if start.tzinfo is None:
        start = start.replace(tzinfo=timezone.utc)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    nominal_dt = args.total_length / args.num_spectra
    file_start = start
    for i in range(args.num_files):
        if args.num_files == 1:
            path = args.output.with_suffix(".bin") \
                if args.output.suffix != ".bin" else args.output
        else:
            stem = args.output.stem
            path = args.output.with_name(f"{stem}_{i:03d}.bin")

        print(f"Writing {path} ({args.num_spectra} spectra, "
              f"{args.total_length}s, start={file_start.isoformat()})")
        last_time = write_file(path, file_start, args.num_spectra, args.total_length,
                               args.nchan, args.freq, args.bw,
                               args.noise_floor, args.noise_std,
                               args.jitter, args.jitter_bias, args.num_signals, rng)
        # Next file starts one jittered dt after the last spectrum of this file.
        file_start = last_time + timedelta(
            seconds=_jittered_dt(nominal_dt, args.jitter, args.jitter_bias, rng)
        )


if __name__ == "__main__":
    main()
