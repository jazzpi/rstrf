#!/usr/bin/env python3

import argparse
import re


def copy_formatting(orig: bytes, new: float) -> bytes:
    """Copy float formatting from orig to new"""
    if b"." in orig:
        decimals = len(orig.split(b".")[1])
        return f"{new:.{decimals}f}".encode()
    else:
        return str(int(new)).encode()


def main():
    parser = argparse.ArgumentParser()
    parser.description = "Apply a fixed offset or PPM correction to rffft .bin files."
    parser.add_argument("-o", "--off", type=float, help="Fixed offset")
    parser.add_argument("-p", "--ppm", type=float, help="PPM correction")
    parser.add_argument("data", nargs="+", help="Data files to process")
    args = parser.parse_args()

    have_off = args.off is not None
    have_ppm = args.ppm is not None
    if have_off and have_ppm:
        parser.error("Only one of --off or --ppm can be specified")
    elif have_off:
        correct = (
            lambda m: b"FREQ"
            + m.group(1)
            + copy_formatting(m.group(2), float(m.group(2)) + args.off)
            + b" Hz"
        )
    elif have_ppm:
        correct = (
            lambda m: b"FREQ"
            + m.group(1)
            + copy_formatting(m.group(2), float(m.group(2)) * (1 + args.ppm / 1e6))
            + b" Hz"
        )
    else:
        parser.error("Either --off or --ppm must be specified")

    for filename in args.data:
        print(f"Processing {filename}...")
        with open(filename, "rb") as f:
            data = f.read()
            corr = re.sub(rb"FREQ( +)([0-9.]+) Hz", lambda m: correct(m), data)
        with open(filename, "wb") as f:
            f.write(corr)

    print("Done.")


if __name__ == "__main__":
    main()
