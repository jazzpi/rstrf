from pathlib import Path

import pytest

from tle_omm import determine_format, parse_input

FORMATS = ("3le", "xml", "kvn", "csv", "json")


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
        assert (
            parsed[i] == parsed[0]
        ), f"Parsed data for {FORMATS[i]} does not match parsed data for {FORMATS[0]}"
