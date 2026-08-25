import dataclasses
import json
import math
from pathlib import Path
import re

import pytest

from panomm import MeanElements, determine_format, format_omm, format_tles, parse_input

FORMATS = ("tle", "xml", "kvn", "csv", "json")


def assert_elements_close(a: MeanElements, b: MeanElements) -> None:
    for field in dataclasses.fields(MeanElements):
        va = getattr(a, field.name)
        vb = getattr(b, field.name)
        if isinstance(va, float):
            assert math.isclose(
                va, vb, rel_tol=1e-2, abs_tol=1e-8
            ), f"{field.name}: {va} != {vb}"
        else:
            assert va == vb, f"{field.name}: {va} != {vb}"


BASE_PATH = Path(__file__).parent.parent / "resources/testdata"
SPACETRACK_PATH = BASE_PATH / "spacetrack/spire"
CELESTRAK_PATH = BASE_PATH / "celestrak/spire"
BASE_PATHS = [SPACETRACK_PATH, CELESTRAK_PATH]


@pytest.mark.parametrize("base_path", BASE_PATHS)
def test_formats_match(base_path: Path) -> None:
    parsed = []
    for format in FORMATS:
        input_path = base_path.with_suffix(f".{format}")
        with open(input_path, "r") as f:
            input_data = f.read()
        parsed.append(parse_input(input_data, determine_format(input_path, None)))

    for i in range(1, len(parsed)):
        for a, b in zip(parsed[i], parsed[0]):
            assert_elements_close(a, b)


@pytest.mark.parametrize("base_path", BASE_PATHS)
@pytest.mark.parametrize("format", FORMATS)
def test_roundtrip(base_path: Path, format: str) -> None:
    input_path = base_path.with_suffix(f".{format}")
    with open(input_path, "r") as f:
        input_data = f.read()
    resolved_format = determine_format(input_path, None)
    parsed = parse_input(input_data, resolved_format)

    if resolved_format == "tle":
        output_data = format_tles(parsed)
    else:
        output_data = format_omm(parsed, resolved_format)
    # Output might not be byte-identical to input (newlines, quotes
    # etc.). So here we only check parsed objects. Formatting is tested
    # below in format-specific tests.
    parsed_roundtrip = parse_input(output_data, resolved_format)

    for a, b in zip(parsed_roundtrip, parsed):
        assert_elements_close(a, b)


@pytest.mark.parametrize("base_path", BASE_PATHS)
def test_json_st_quotes_all_values(base_path: Path) -> None:
    input_path = base_path.with_suffix(".json")
    with open(input_path, "r") as f:
        parsed = parse_input(f.read(), "json")

    output_data = format_omm(parsed, "json-st")

    for row in json.loads(output_data):
        for key, value in row.items():
            assert isinstance(value, str), f"{key} is not a quoted string: {value!r}"


def test_json_ct_keeps_types() -> None:
    input_path = CELESTRAK_PATH.with_suffix(".json")
    with open(input_path, "r") as f:
        input_data = f.read()
        parsed = parse_input(input_data, "json")

    output_data = format_omm(parsed, "json-ct")

    for input_row, output_row in zip(json.loads(input_data), json.loads(output_data)):
        # Check only fields in input_row, since output_row may have
        # extra fields (e.g. "CCSDS_OMM_VERS")
        for key, value in input_row.items():
            assert key in output_row, f"{key} missing in output"
            in_type = type(value)
            out_type = type(output_row[key])
            # Celestrak JSON has `0` (int) for MEAN_MOTION_DDOT. We
            # output it  as `0.0` (float).
            if key == "MEAN_MOTION_DDOT":
                assert out_type in (
                    float,
                    int,
                ), f"{key} has different type: {in_type} != {out_type}"
            else:
                assert (
                    in_type == out_type
                ), f"{key} has different type: {in_type} != {out_type}"


_CSV_ALL_FIELDS_QUOTED_LINE = re.compile(r'^"[^"]*"(?:,"[^"]*")*$')


@pytest.mark.parametrize("base_path", BASE_PATHS)
def test_csv_st_quotes_values_not_header(base_path: Path) -> None:
    with open(base_path.with_suffix(".csv"), "r") as f:
        parsed = parse_input(f.read(), "csv")

    output_data = format_omm(parsed, "csv-st")

    lines = output_data.strip("\r\n").splitlines()
    assert '"' not in lines[0], f"header should not be quoted: {lines[0]!r}"
    for line in lines[1:]:
        assert _CSV_ALL_FIELDS_QUOTED_LINE.match(
            line
        ), f"not all fields are quoted: {line!r}"


@pytest.mark.parametrize("base_path", BASE_PATHS)
def test_kvn_keeps_field_order(base_path: Path) -> None:
    with open(base_path.with_suffix(".kvn"), "r") as f:
        input_data = f.read()
        parsed = parse_input(input_data, "kvn")

    output_data = format_omm(parsed, "kvn")

    input_lines = _kvn_remove_empty(input_data.splitlines())
    output_lines = _kvn_remove_empty(output_data.splitlines())

    # Input may have extra lines (e.g. USER_DEFINED_*), and we only
    # output mandatory fields. So only check output lines.
    in_idx = 0
    for out_line in output_lines:
        while not _kvn_same_key(input_lines[in_idx], out_line):
            in_idx += 1
            if in_idx >= len(input_lines):
                raise AssertionError(
                    f"Ran out of input lines while searching for output line: {out_line!r}"
                )


def _kvn_remove_empty(lines: list[str]) -> list[str]:
    return [line for line in lines if line.strip() and not line.startswith("COMMENT ")]


def _kvn_same_key(line1: str, line2: str) -> bool:
    key1 = line1.split("=", 1)[0].strip()
    key2 = line2.split("=", 1)[0].strip()
    return key1 == key2
