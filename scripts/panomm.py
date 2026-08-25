#!/usr/bin/env python3

import argparse
import csv
from dataclasses import dataclass, field
import dataclasses
from datetime import datetime, timedelta
from io import StringIO
import json
import logging
from pathlib import Path
import re
import sys
from typing import Callable, ClassVar, Generator, Iterable, Mapping
import xml.etree.ElementTree as ET

ODM_FORMATS = ["xml", "kvn", "json", "json-ct", "json-st", "csv", "csv-ct", "csv-st"]
DEFAULT_JSON_FORMAT = "json-ct"
DEFAULT_CSV_FORMAT = "csv-ct"


@dataclass
class MeanElements:
    object_name: str
    norad_cat_id: int
    classification_type: str
    object_id: str
    epoch: datetime = field(
        metadata={"initializer": lambda x: datetime.fromisoformat(str(x))}
    )
    mean_motion_dot: float
    mean_motion_ddot: float
    bstar: float
    ephemeris_type: int
    element_set_no: int
    inclination: float
    ra_of_asc_node: float
    eccentricity: float
    arg_of_pericenter: float
    mean_anomaly: float
    mean_motion: float
    rev_at_epoch: int

    HEADER_FIELDS = (
        "CCSDS_OMM_VERS",
        "CREATION_DATE",
        "ORIGINATOR",
    )
    METADATA_FIELDS = (
        "OBJECT_NAME",
        "OBJECT_ID",
        "CENTER_NAME",
        "REF_FRAME",
        "TIME_SYSTEM",
        "MEAN_ELEMENT_THEORY",
    )
    MEAN_ELEMENTS_FIELDS = (
        "EPOCH",
        "MEAN_MOTION",
        "ECCENTRICITY",
        "INCLINATION",
        "RA_OF_ASC_NODE",
        "ARG_OF_PERICENTER",
        "MEAN_ANOMALY",
    )
    TLE_PARAMETERS_FIELDS = (
        "EPHEMERIS_TYPE",
        "CLASSIFICATION_TYPE",
        "NORAD_CAT_ID",
        "ELEMENT_SET_NO",
        "REV_AT_EPOCH",
        "BSTAR",
        "MEAN_MOTION_DOT",
        "MEAN_MOTION_DDOT",
    )
    # All fields sorted according to CCSDS 502.0-B-3, 7.4.8
    ALL_FIELDS: ClassVar[tuple[str, ...]] = (
        HEADER_FIELDS + METADATA_FIELDS + MEAN_ELEMENTS_FIELDS + TLE_PARAMETERS_FIELDS
    )

    @classmethod
    def from_map(cls, map_: Mapping[str, str | int | float]) -> "MeanElements":
        cls._check_if_exists(map_, "CCSDS_OMM_VERS", ["2.0", "3.0"], str, True)
        cls._check_if_exists(map_, "MEAN_ELEMENT_THEORY", ["SGP4", "SGP/SGP4"], str)
        cls._check_if_exists(map_, "TIME_SYSTEM", ["UTC"], str)
        cls._check_if_exists(map_, "CENTER_NAME", ["EARTH"], str)
        cls._check_if_exists(map_, "REF_FRAME", ["TEME"], str)
        data = {}
        for field in dataclasses.fields(cls):
            name = field.name.upper()
            if name not in map_:
                raise ValueError(f"Missing required field: {field.name}")
            initializer = field.metadata.get("initializer", field.type)
            data[field.name] = initializer(map_[name])

        return MeanElements(**data)

    def to_map(self, marshal: Callable | None = None) -> dict[str, str | int | float]:
        result = {}
        for field in dataclasses.fields(self):
            name = field.name.upper()
            value = getattr(self, field.name)
            if isinstance(value, datetime):
                value = value.isoformat()
            if marshal is not None:
                value = marshal(value)
            result[name] = value
        result["CCSDS_OMM_VERS"] = "3.0"
        # TODO: options to set creation date & originator
        result["CREATION_DATE"] = ""
        result["ORIGINATOR"] = ""
        result["MEAN_ELEMENT_THEORY"] = "SGP4"
        result["TIME_SYSTEM"] = "UTC"
        result["CENTER_NAME"] = "EARTH"
        result["REF_FRAME"] = "TEME"
        return result

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
        object_name=name,
        norad_cat_id=cat_no,
        classification_type=classification,
        object_id=intl_designator,
        epoch=epoch,
        mean_motion_dot=mean_motion_dot,
        mean_motion_ddot=mean_motion_ddot,
        bstar=bstar,
        ephemeris_type=ephemeris_type,
        element_set_no=element_set_no,
        inclination=inclination,
        ra_of_asc_node=raan,
        eccentricity=eccentricity,
        arg_of_pericenter=arg_perigee,
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
    if len(s) >= 3 and s[-2] in ("+", "-"):
        s = s[:-2] + "e" + s[-2:]
    return float(s)


def parse_omm(input_data: str, input_format: str) -> list[MeanElements]:
    if input_format == "xml":
        return parse_odm_xml(input_data)
    elif input_format == "kvn":
        return parse_odm_kvn(input_data)
    elif input_format.startswith("json"):
        return parse_odm_json(input_data)
    elif input_format.startswith("csv"):
        return parse_odm_csv(input_data)
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


def parse_odm_json(input_data: str) -> list[MeanElements]:
    data = json.loads(input_data)
    if isinstance(data, dict):
        data = [data]
    result = []
    for omm in data:
        elements = MeanElements.from_map(omm)
        result.append(elements)
    return result


def parse_odm_csv(input_data: str) -> list[MeanElements]:
    reader = csv.DictReader(input_data.strip().splitlines())
    result = []
    for row in reader:
        elements = MeanElements.from_map(row)
        result.append(elements)
    return result


def format_tles(elements_list: list[MeanElements]) -> str:
    lines = []
    for elements in elements_list:
        line1, line2 = format_tle(elements)
        lines.append(elements.object_name)
        lines.append(line1)
        lines.append(line2)
    return "\n".join(lines) + "\n"


def format_tle(elements: MeanElements) -> tuple[str, str]:
    line1 = (
        f"1 {format_alpha5(elements.norad_cat_id):05}{elements.classification_type} "
        f"{format_tle_designator(elements.object_id)} "
        f"{format_tle_epoch(elements.epoch)} "
        f"{format_tle_mmdot(elements.mean_motion_dot)} "
        f"{format_tle_exponential(elements.mean_motion_ddot)} "
        f"{format_tle_exponential(elements.bstar)} "
        f"{elements.ephemeris_type} "
        f"{elements.element_set_no:04}"
    )
    line2 = (
        f"2 {format_alpha5(elements.norad_cat_id):05} "
        f"{elements.inclination:8.4f} "
        f"{elements.ra_of_asc_node:8.4f} "
        f"{format_tle_assumed_decimal(elements.eccentricity, 7)} "
        f"{elements.arg_of_pericenter:8.4f} "
        f"{elements.mean_anomaly:8.4f} "
        f"{elements.mean_motion:11.8f}"
        f"{elements.rev_at_epoch:05}"
    )
    return line1 + _tle_checksum(line1), line2 + _tle_checksum(line2)


def format_alpha5(cat_no: int) -> str:
    if cat_no < 100_000:
        return f"{cat_no:05}"
    else:
        first = cat_no // 100_000
        rest = cat_no % 100_000
        if 10 <= first <= 17:
            first_char = chr(ord("A") + (first - 10))
        elif 18 <= first <= 22:
            first_char = chr(ord("J") + (first - 18))
        elif 23 <= first <= 33:
            first_char = chr(ord("P") + (first - 23))
        else:
            raise ValueError(f"Catalog number too large for alpha-5 format: {cat_no}")
        return f"{first_char}{rest:04}"


def format_tle_designator(desig: str) -> str:
    return f"{desig[2:4]}{desig[5:8]}{desig[8:]:<3}"


def format_tle_epoch(epoch: datetime) -> str:
    year = epoch.year % 100
    day_of_year = (epoch - datetime(epoch.year, 1, 1)).days + 1
    fraction_of_day = (
        epoch.hour * 3600 + epoch.minute * 60 + epoch.second + epoch.microsecond / 1e6
    ) / 86400
    fractional_day = day_of_year + fraction_of_day
    return f"{year:02}{fractional_day:012.8f}"


def format_tle_exponential(value: float) -> str:
    if abs(value) < 1e-9:
        value = 0.0
    if value == 0.0:
        return f"+00000+0"
    value_str = f"{value:+.{4}e}"
    # Turn `+2.1e-03` into `21-4`
    exponent = int(value_str[-3:])
    assert exponent < 0
    exponent += 1
    sign = value_str[0]
    value_str = value_str[1] + value_str[3:-4]  # Omit sign, decimal point, and exponent
    value_str = f"{sign}{value_str}{exponent}"
    assert (
        len(value_str) == 8
    ), f"Value {value} cannot be formatted in exponential notation"
    return value_str


def format_tle_assumed_decimal(value: float, width: int) -> str:
    assert value >= 0.0, f"Value {value} cannot be negative for assumed decimal format"
    value_str = f"{value:.{width}f}"
    return value_str[2:]  # skip leading "0."


def format_tle_mmdot(value: float) -> str:
    value_str = f"{value:+.8f}"
    return value_str[0] + value_str[2:]  # Keep the sign, skip leading "0.


def _tle_checksum(line: str) -> str:
    checksum = 0
    for c in line:
        if c.isdigit():
            checksum += int(c)
        elif c == "-":
            checksum += 1
    return str(checksum % 10)


def format_omm(elements_list: list[MeanElements], output_format: str) -> str:
    if output_format == "xml":
        return format_odm_xml(elements_list)
    elif output_format == "kvn":
        return format_odm_kvn(elements_list)
    elif output_format.startswith("json"):
        return format_odm_json(elements_list, output_format)
    elif output_format.startswith("csv"):
        return format_odm_csv(elements_list, output_format)
    else:
        raise ValueError(f"Unsupported OMM format: {output_format}")


def format_odm_xml(elements_list: list[MeanElements]) -> str:
    result = """<?xml version="1.0" encoding="UTF-8"?>
<ndm xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:noNamespaceSchemaLocation="https://sanaregistry.org/r/ndmxml_unqualified/ndmxml-2.0.0-master-2.0.xsd">
"""
    for elements in elements_list:
        result += format_odm_xml_omm(elements) + "\n"
    result += "</ndm>"
    return result


def format_odm_xml_omm(elements: MeanElements) -> str:
    result = (
        '<omm id="CCSDS_OMM_VERS" version="3.0"><header><CREATION_DATE/><ORIGINATOR/></header><body><segment>'
        "<metadata>"
    )
    data = elements.to_map()
    for field in MeanElements.METADATA_FIELDS:
        value = data[field]
        result += f"<{field}>{value}</{field}>"
    result += "</metadata><data><meanElements>"
    for field in MeanElements.MEAN_ELEMENTS_FIELDS:
        value = data[field]
        result += f"<{field}>{value}</{field}>"
    result += "</meanElements><tleParameters>"
    for field in MeanElements.TLE_PARAMETERS_FIELDS:
        value = data[field]
        result += f"<{field}>{value}</{field}>"
    result += "</tleParameters></data></segment></body></omm>"
    return result


def format_odm_kvn(elements_list: list[MeanElements]) -> str:
    result = ""
    for elements in elements_list:
        result += format_odm_kvn_omm(elements) + "\n"
    return result


def format_odm_kvn_omm(elements: MeanElements) -> str:
    result = ""
    data = elements.to_map()
    for field in MeanElements.ALL_FIELDS:
        value = data[field]
        result += f"{field} = {value}\n"
    return result


def format_odm_json(elements_list: list[MeanElements], output_format: str) -> str:
    data = [elements.to_map() for elements in elements_list]
    if output_format == "json-st":
        data = [{k: str(v) for k, v in d.items()} for d in data]
    # TODO: Option for pretty-printing
    return json.dumps(data)


def format_odm_csv(elements_list: list[MeanElements], output_format: str) -> str:
    output = StringIO()
    # Always write header without quotes
    output.write(",".join(MeanElements.ALL_FIELDS) + "\n")
    quoting = csv.QUOTE_MINIMAL
    if output_format == "csv-st":
        quoting = csv.QUOTE_ALL
    writer = csv.DictWriter(output, fieldnames=MeanElements.ALL_FIELDS, quoting=quoting)
    for elements in elements_list:
        data = elements.to_map()
        writer.writerow(data)
    return output.getvalue()


if __name__ == "__main__":
    sys.exit(main())
