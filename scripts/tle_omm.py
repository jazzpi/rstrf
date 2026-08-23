#!/usr/bin/env python3

import argparse
import csv
from dataclasses import dataclass
from datetime import datetime, timedelta
import json
import logging
from pathlib import Path
import re
import sys
from typing import Generator, Iterable, Mapping
import xml.etree.ElementTree as ET

ODM_FORMATS = ["xml", "kvn", "json", "json-ct", "json-st", "csv", "csv-ct", "csv-st"]
DEFAULT_JSON_FORMAT = "json-ct"
DEFAULT_CSV_FORMAT = "csv-ct"


@dataclass
class MeanElements:
    name: str
    cat_no: int
    classification: str
    intl_designator: str
    epoch: datetime
    mean_motion_dot: float
    mean_motion_ddot: float
    bstar: float
    ephemeris_type: int
    element_set_no: int
    inclination: float
    raan: float
    eccentricity: float
    arg_perigee: float
    mean_anomaly: float
    mean_motion: float
    rev_at_epoch: int

    @classmethod
    def from_map(cls, data: Mapping[str, str | int | float]) -> "MeanElements":
        cls._check_if_exists(data, "CCSDS_OMM_VERS", ["2.0", "3.0"], str, True)
        cls._check_if_exists(data, "MEAN_ELEMENT_THEORY", ["SGP4", "SGP/SGP4"], str)
        cls._check_if_exists(data, "TIME_SYSTEM", ["UTC"], str)
        name = str(data["OBJECT_NAME"])
        intl_designator = str(data["OBJECT_ID"])

        epoch_str = str(data["EPOCH"])
        epoch = datetime.fromisoformat(epoch_str)
        mean_motion = float(data["MEAN_MOTION"])
        eccentricity = float(data["ECCENTRICITY"])
        inclination = float(data["INCLINATION"])
        raan = float(data["RA_OF_ASC_NODE"])
        arg_perigee = float(data["ARG_OF_PERICENTER"])
        mean_anomaly = float(data["MEAN_ANOMALY"])

        ephemeris_type = int(data["EPHEMERIS_TYPE"])
        classification = str(data["CLASSIFICATION_TYPE"])
        cat_no = int(data["NORAD_CAT_ID"])
        element_set_no = int(data["ELEMENT_SET_NO"])
        rev_at_epoch = int(data["REV_AT_EPOCH"])
        bstar = float(data["BSTAR"])
        mean_motion_dot = float(data["MEAN_MOTION_DOT"])
        mean_motion_ddot = float(data["MEAN_MOTION_DDOT"])

        return MeanElements(
            name=name,
            cat_no=cat_no,
            classification=classification,
            intl_designator=intl_designator,
            epoch=epoch,
            mean_motion_dot=mean_motion_dot,
            mean_motion_ddot=mean_motion_ddot,
            bstar=bstar,
            ephemeris_type=ephemeris_type,
            element_set_no=element_set_no,
            inclination=inclination,
            raan=raan,
            eccentricity=eccentricity,
            arg_perigee=arg_perigee,
            mean_anomaly=mean_anomaly,
            mean_motion=mean_motion,
            rev_at_epoch=rev_at_epoch,
        )

    @staticmethod
    def _check_if_exists(
        data: Mapping[str, str | int | float],
        key: str,
        values: list,
        typecast: type | None = None,
        warn: bool = False,
    ):
        value = data.get(key)
        if value is not None:
            if typecast is not None:
                value = typecast(value)
            if value not in values:
                msg = f"Unsupported {key}: {value}"
                if warn:
                    logging.warning(msg)
                else:
                    raise ValueError(msg)


def main() -> int:
    parser = argparse.ArgumentParser(description="Convert between TLE and OMM formats.")
    parser.add_argument(
        "input_file",
        type=Path,
        help="Path to the input file (TLE or OMM format).",
    )
    parser.add_argument(
        "output_file",
        type=Path,
        help="Path to the output file (TLE or OMM format).",
    )
    parser.add_argument(
        "--input-format",
        choices=["tle"] + ODM_FORMATS,
        help="Format of the input file (see FORMATS). If not specified, the format will be inferred from the file extension.",
    )
    parser.add_argument(
        "--output-format",
        choices=["tle"] + ODM_FORMATS,
        help="Format of the output file (see FORMATS). If not specified, the format will be inferred from the file extension.",
    )
    # TODO: How can I add a FORMATS section to the help text?
    args = parser.parse_args()

    with open(args.input_file, "r") as f:
        input_data = f.read()
        input_format = determine_format(args.input_file, args.input_format)
        elements = parse_input(input_data, input_format)
    with open(args.output_file, "w") as f:
        output_format = determine_format(args.output_file, args.output_format)
        if output_format == "tle":
            output_data = format_tles(elements)
        elif output_format in ODM_FORMATS:
            output_data = format_omm(elements, output_format)
        else:
            raise ValueError(f"Unsupported output format: {output_format}")

        f.write(output_data)

    return 0


def determine_format(file_path: Path, specified_format: str | None) -> str:
    if specified_format:
        return specified_format
    ext = file_path.suffix.lower()
    if ext in (".tle", ".2le", ".3le", ".txt"):
        return "tle"
    elif ext == ".xml":
        return "xml"
    elif ext == ".kvn":
        return "kvn"
    elif ext == ".json":
        return DEFAULT_JSON_FORMAT
    elif ext == ".csv":
        return DEFAULT_CSV_FORMAT
    else:
        raise ValueError(f"Cannot determine format from file extension: {ext}")


def parse_input(input_data: str, input_format: str) -> list[MeanElements]:
    if input_format == "tle":
        return parse_tles(input_data)
    elif input_format in ODM_FORMATS:
        return parse_omm(input_data, input_format)
    else:
        raise ValueError(f"Unsupported input format: {input_format}")


def parse_tles(input_data: str) -> list[MeanElements]:
    lines = input_data.strip().splitlines()
    idx = 0
    result = []
    while idx < len(lines):
        lines_used, elements = parse_tle_maybe(lines[idx : idx + 3])
        result.append(elements)
        idx += lines_used
    return result


def parse_tle_maybe(lines: list[str]) -> tuple[int, MeanElements]:
    if len(lines) == 3 and lines[1].startswith("1 ") and lines[2].startswith("2 "):
        name = lines[0].strip()
        if name.startswith("0 "):
            name = name[2:].strip()
        line1 = lines[1].strip()
        line2 = lines[2].strip()
        lines_used = 3
    elif len(lines) >= 2 and lines[0].startswith("1 ") and lines[1].startswith("2 "):
        name = "UNKNOWN"
        line1 = lines[0].strip()
        line2 = lines[1].strip()
        lines_used = 2
    else:
        raise ValueError(f"Invalid TLE format for lines: {lines}")

    return lines_used, parse_tle(name, line1, line2)


def parse_tle(name: str, line1: str, line2: str) -> MeanElements:
    cat_no = parse_alpha5(line1[2:7].strip())
    cat_no2 = parse_alpha5(line2[2:7].strip())
    if cat_no != cat_no2:
        raise ValueError(
            f"Catalog number mismatch between line 1 and line 2: {cat_no} != {cat_no2}"
        )
    classification = line1[7]
    intl_designator = parse_tle_designator(line1[9:17].strip())
    epoch = parse_tle_epoch(line1[18:32].strip())
    mean_motion_dot = parse_tle_float(line1[33:43].strip())
    mean_motion_ddot = parse_tle_float(line1[44:52].strip(), True)
    bstar = parse_tle_float(line1[53:61].strip(), True)
    ephemeris_type = int(line1[62])
    element_set_no = int(line1[64:68].strip())
    inclination = parse_tle_float(line2[8:16].strip())
    raan = parse_tle_float(line2[17:25].strip())
    eccentricity = parse_tle_float(line2[26:33].strip(), True)
    arg_perigee = parse_tle_float(line2[34:42].strip())
    mean_anomaly = parse_tle_float(line2[43:51].strip())
    mean_motion = parse_tle_float(line2[52:63].strip())
    rev_at_epoch = int(line2[63:68].strip())
    # TODO: validate checksum?

    return MeanElements(
        name=name,
        cat_no=cat_no,
        classification=classification,
        intl_designator=intl_designator,
        epoch=epoch,
        mean_motion_dot=mean_motion_dot,
        mean_motion_ddot=mean_motion_ddot,
        bstar=bstar,
        ephemeris_type=ephemeris_type,
        element_set_no=element_set_no,
        inclination=inclination,
        raan=raan,
        eccentricity=eccentricity,
        arg_perigee=arg_perigee,
        mean_anomaly=mean_anomaly,
        mean_motion=mean_motion,
        rev_at_epoch=rev_at_epoch,
    )


def parse_alpha5(s: str) -> int:
    if len(s) < 5 or s[0].isdigit():
        return int(s)
    elif len(s) == 5 and s[0].isalpha() and s[1:].isdigit():
        first_ord = ord(s[0])
        if ord("A") <= first_ord <= ord("H"):
            first = 10 + (first_ord - ord("A"))
        elif ord("J") <= first_ord <= ord("N"):
            first = 18 + (first_ord - ord("J"))
        elif ord("P") <= first_ord <= ord("Z"):
            first = 23 + (first_ord - ord("P"))
        else:
            raise ValueError(f"Invalid alpha-5 string: {s}")
        rest = int(s[1:])
        return first * 10000 + rest
    raise ValueError(f"Invalid alpha-5 string: {s}")


def parse_tle_designator(s: str) -> str:
    if not 6 <= len(s) <= 8:
        raise ValueError(f"Invalid international designator: {s}")
    year = parse_tle_year(s[0:2])
    launch_number = int(s[2:5])
    piece = s[5:]
    return f"{year:04d}-{launch_number:03d}{piece}"


def parse_tle_epoch(s: str) -> datetime:
    if len(s) != 14:
        raise ValueError(f"Invalid epoch string: {s}")
    year = parse_tle_year(s[0:2])
    day_of_year = float(s[2:14])
    epoch = datetime(year, 1, 1) + timedelta(days=day_of_year - 1)
    return epoch


def parse_tle_year(s: str) -> int:
    if len(s) != 2 or not s.isdigit():
        raise ValueError(f"Invalid year string: {s}")
    year = int(s)
    if year >= 57:  # Sputnik was launched in 1957
        return 1900 + year
    else:
        return 2000 + year


def parse_tle_float(s: str, leading_decimal: bool = False) -> float:
    if not s:
        raise ValueError("Empty string cannot be converted to float")
    if leading_decimal:
        if s[0] in ("+", "-"):
            s = s[0] + "0." + s[1:]
        else:
            s = "0." + s
    if len(s) >= 3 and s[-2] in ("+", "-") and s[-3].upper() != "E":
        s = s[:-2] + "e" + s[-2:]
    return float(s)


def parse_omm(input_data: str, input_format: str) -> list[MeanElements]:
    if input_format == "xml":
        return parse_odm_xml(input_data)
    elif input_format == "kvn":
        return parse_odm_kvn(input_data)
    elif input_format.startswith("json"):
        return parse_odm_json(input_data, input_format)
    elif input_format.startswith("csv"):
        return parse_odm_csv(input_data, input_format)
    else:
        raise ValueError(f"Unsupported OMM format: {input_format}")


def parse_odm_xml(input_data: str) -> list[MeanElements]:
    root = ET.fromstring(input_data)
    if root.tag == "ndm":
        result = []
        for child in root:
            elements = parse_odm_xml_omm(child)
            result.append(elements)
        assert result, "No OMMs found in NDM"
    elif root.tag == "omm":
        elements = parse_odm_xml_omm(root)
        result = [elements]
    else:
        raise ValueError(f"Invalid XML root tag: {root.tag}")
    return result


def parse_odm_xml_omm(omm: ET.Element) -> MeanElements:
    if omm.attrib["id"] != "CCSDS_OMM_VERS":
        raise ValueError(f"Invalid OMM id: {omm.attrib['id']}")
    version = omm.attrib["version"]

    # TODO: does this find <omm><body><segment> or only <omm><segment>?
    segment = _find(omm, "body/segment")

    metadata = _node_to_dict(_find(segment, "metadata"))
    data = _find(segment, "data")

    mean_elements = _node_to_dict(_find(data, "meanElements"))
    tle_parameters = _node_to_dict(_find(data, "tleParameters"))

    data_map = {
        "CCSDS_OMM_VERS": version,
        **metadata,
        **mean_elements,
        **tle_parameters,
    }

    return MeanElements.from_map(data_map)


def _find(parent: ET.Element, path: str) -> ET.Element:
    elem = parent.find(path)
    assert elem is not None, f"OMM {path} not found"
    return elem


def _node_to_dict(node: ET.Element) -> dict[str, str]:
    result = {}
    for child in node:
        if child.text is not None:
            result[child.tag] = child.text.strip()
    return result


def parse_odm_kvn(input_data: str) -> list[MeanElements]:
    result = []
    for omm in _group_kvn(input_data.strip().splitlines()):
        elements = MeanElements.from_map(omm)
        result.append(elements)
    return result


_KVN_LINE_REGEX = re.compile(
    r"(?:\s*)(?P<keyword>\S*)(?:\s*)=(?:\s*)(?P<value>.*)(?:\s*)$"
)


def _group_kvn(lines: Iterable[str]) -> Generator[dict[str, str]]:
    group = {}
    for line in lines:
        match = _KVN_LINE_REGEX.match(line.strip())
        if match:
            keyword = match.group("keyword")
            value = match.group("value")
            if keyword == "CCSDS_OMM_VERS" and group:
                yield group
                group = {}
            group[keyword] = value
    if group:
        yield group


def parse_odm_json(input_data: str, input_format: str) -> list[MeanElements]:
    # TODO: does input_format matter?
    data = json.loads(input_data)
    if isinstance(data, dict):
        data = [data]
    result = []
    for omm in data:
        elements = MeanElements.from_map(omm)
        result.append(elements)
    return result


def parse_odm_csv(input_data: str, input_format: str) -> list[MeanElements]:
    # TODO: does input_format matter?
    reader = csv.DictReader(input_data.strip().splitlines())
    result = []
    for row in reader:
        elements = MeanElements.from_map(row)
        result.append(elements)
    return result


if __name__ == "__main__":
    sys.exit(main())
