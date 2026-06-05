# rSTRF

rSTRF is a WIP rewrite of the excellent [strf satellite tracking toolkit for
radio observations](https://github.com/cbassa/strf).

![Screenshot](docs/screenshot.png)

Of the STRF tools, currently there is only an equivalent of the  `rfplot` tool.
I plan to add at least `rffit` and `rffft` as well.

---

rSTRF uses GPU-accelerated plotting instead of STRF's pgplot, which enables
fluid mouse-based panning and zooming in the waterfall. It also makes it easy to
have multiple plots active at the same time.

The different tech stack (Rust + Iced) makes building for Windows actually
possible and should make building for non-Debian systems a bit easier.

It is still very much alpha software, but at least stable enough that I use it
regularly for my day job.

## Installation

### Ubuntu

```sh
sudo apt install build-essential libssl-dev pkg-config fontconfig \
  libfontconfig1-dev libopenblas-dev
```

### Nix

This repository includes a flake, so you can just

```sh
nix run github:jazzpi/rstrf
```

Alternatively, install these packages:

```
libxkbcommon
wayland
mesa
libGL
libglvnd
vulkan-loader
udev
openblas
dbus
pkg-config
openssl
fontconfig
```

### Windows

Building on Windows is slightly more involved than on an average Linux system,
mainly due to [`vcpkg`](https://vcpkg.io). If you don't have it installed, you
can find instructions to do so
[here](https://learn.microsoft.com/en-us/vcpkg/get_started/overview).

After installing `vcpkg` (or if you already had it installed), it should be as
simple as running:
```sh
vcpkg install openblas --triplet x64-windows
```

> [!TIP]
> See [`openblas-src` repository README][openblas-src-readme] for more
> information.

### Build

You will also need to [install Rust](https://rust-lang.org/tools/install/),
then:

```sh
git clone https://github.com/jazzpi/rstrf
cd rstrf
cargo build --release
```

After the build is complete, you can run it with

```sh
cargo run --release
```

## Usage

### Spectrogram data

rSTRF does not (currently) include a way to record/generate spectrograms. For
this, please use STRF's `rffft` to generate `.bin` files.

### Plotting

Use the `plot` subcommand and pass the `.bin` files you want to display. Unlike
STRF's `rfplot`, you need to pass all the files instead of just the beginning of
the file name (before the index). For example:

```sh
cargo run --release -- plot \
  /path/to/rffft_data/2026-02-19T00\:00\:01_0000{00..59}.bin \
  -c /path/to/bulk.tle \
  -F /path/to/frequencies.txt \
  --zmin -38 \
  -C 4801
```

You can also restrict the initial view with `--fmin`/`--fmax` (Hz) and
`--tmin`/`--tmax` (seconds since the start of the spectrogram).

For more usage information, see `cargo run --release -- plot -h`.

Using the mouse, you can

- scroll to zoom (use `Ctrl`/`Shift`+scroll to zoom vertically/horizontally)
- click+drag to pan

There is a toolbar above the plot. Hover over the buttons for an explanatory
tooltip. Additionally, you can use the following hotkeys:

- `r` -> Reset view
- `p` -> Toggle predictions
- `z` -> Zoom to rectangle
- `d` -> Delete trackpoints/signals in rectangle
- `ESC` -> Cancel rectangle action
- `s` -> Add trackpoint
- `f` -> Find signals around trackpoints ([see below](#signal-export))
- `D` -> Manually mark a signal ([see below](#signal-export))
- Arrow keys -> Pan (full plot width/height)
- `SHIFT` + arrow keys -> Pan (half plot width/height)

### Signal export

`s`/`f`/`D` work a little differently from STRF's `rfplot`. For one, pressing
`s`/`D` does not directly add a track/mark point. Instead, it enters a "mode" in
which each mouse click adds a track/mark point. You can exit this mode by
pressing `ESC`.

Further, pressing `f`/`D` by themselves does not generate any
`out.dat`/`mark.dat` files. Instead, once you have found/marked all the signals
(and potentially cleaned them up using `d`), press the *Save* button in the
toolbar. This will write all signals into a `.dat` file directory.

Currently, the sigma field in the `out.dat` file is set to 5 for all signals.
The site ID field can be controlled using the `-C` CLI argument.

### Following your STRF site

rSTRF can read the observer ground site from STRF's `sites.txt` instead of using
the site configured in the preferences. Enable "Follow STRF site" in the
preferences. The site is then looked up from `sites.txt` (located via the
`$ST_SITES_TXT` / `$ST_DATADIR` environment variables) using the COSPAR site ID
from `$ST_COSPAR` or the `-C` command-line argument.

### Generating pass images

The `pass-png` subcommand batch-generates a PNG for each pass of a given
satellite over the spectrogram, without opening the GUI. Select the satellite by
its NORAD ID (`-i`) and one or more transmitter frequencies (`-f`, repeatable,
in Hz) — or load a `frequencies.txt` with `-F`. Output files are named
`<prefix>_000.png`, `<prefix>_001.png`, ...

```sh
cargo run --release -- pass-png \
  /path/to/rffft_data/2026-02-19T00\:00\:01_*{00..59}.bin \
  -c /path/to/bulk.tle \
  -i 25544 \
  -f 435.5e6 \
  -o /path/to/output/pass \
  --zmin -38
```

You can set the image size with `-W`/`-H` (default 800x600). For more usage
information, see `cargo run --release -- pass-png -h`.

**NOTE**: For technical reasons, rSTRF opens a plot window and navigates to each
pass, then saves a screenshot of the full window.

## Troubleshooting

You can increase the log verbosity with `-v` (debug) or `-vv` (trace), e.g.
`cargo run --release -- -vv plot ...`.

For finer control, you can enable debug logs and backtraces by setting the
`RUST_LOG`/`RUST_BACKTRACE` environment variables (`RUST_LOG` overrides `-v`):

```sh
export RUST_LOG=debug
export RUST_BACKTRACE=1
```

## `rsmedfilt`

This repo also includes a CLI tool called `rsmedfilt` for preprocessing
spectrograms. It estimates the local noise floor using a median filter, then
subtracts it from the spectrogram. This can be helpful to bring out detail.

To run it, just run

```sh
# Nix
nix run github.com:jazzpi/rstrf#rsmedfilt --help
# Other systems
cargo run --bin rsmedfilt -- --help
```

[openblas-src-readme]: https://github.com/blas-lapack-rs/openblas-src/blob/openblas-src-v0.10.14/README.md#windows-and-vcpkg
