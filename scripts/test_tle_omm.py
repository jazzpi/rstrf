import dataclasses
import math
from pathlib import Path

import pytest

from tle_omm import MeanElements, determine_format, format_omm, format_tles, parse_input

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


@pytest.mark.parametrize("base_path_str", ["spacetrack/spire"])
def test_formats_match(base_path_str: str) -> None:
    base_path = Path(base_path_str)

    parsed = []
    for format in FORMATS:
        input_path = base_path.with_suffix(f".{format}")
        with open(input_path, "r") as f:
            input_data = f.read()
        parsed.append(parse_input(input_data, determine_format(input_path, None)))

    for i in range(1, len(parsed)):
        for a, b in zip(parsed[i], parsed[0]):
            assert_elements_close(a, b)


@pytest.mark.parametrize("base_path_str", ["spacetrack/spire"])
@pytest.mark.parametrize("format", FORMATS)
def test_roundtrip(base_path_str: str, format: str) -> None:
    base_path = Path(base_path_str)

    input_path = base_path.with_suffix(f".{format}")
    with open(input_path, "r") as f:
        input_data = f.read()
    resolved_format = determine_format(input_path, None)
    parsed = parse_input(input_data, resolved_format)

    if resolved_format == "tle":
        output_data = format_tles(parsed)
    else:
        output_data = format_omm(parsed, resolved_format)
    parsed_roundtrip = parse_input(output_data, resolved_format)

    for a, b in zip(parsed_roundtrip, parsed):
        assert_elements_close(a, b)
