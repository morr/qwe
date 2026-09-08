#!/usr/bin/env python3
"""Атрибуты улиц, по которым рисуется разметка: `oneway`, `lanes`, `junction`.

Считает по кешу `assets/osm/*.json` улицы шириной от 8 м (те же классы, что
`parse/tags.rs::road_class` даёт residential и выше — проезды и дорожки линий
не несут) и печатает, сколько из них односторонние, сколько — кольца и у
скольких есть тег `lanes`. Пороги обязаны совпадать с `map/roads.rs::lane_count`.

    python3 tools/osm_audit/road_attributes.py assets/osm/*.json
"""

import collections
import json
import sys

# parse/tags.rs::road_class — классы шириной от STREET_MIN_WIDTH (8 м)
STREETS = {
    "motorway", "trunk", "primary", "secondary", "tertiary",
    "residential", "unclassified", "living_street",
}
# parse/tags.rs::is_oneway / is_roundabout
ONEWAY = {"yes", "1", "true", "-1"}
ROUNDABOUT = {"roundabout", "circular"}


def is_oneway(tags):
    """parse/tags.rs::is_oneway — кольцо одностороннее по определению."""
    return tags.get("oneway") in ONEWAY or tags.get("junction") in ROUNDABOUT


def main(paths):
    for path in paths:
        with open(path) as handle:
            elements = json.load(handle)["elements"]
        tags = [
            element["tags"]
            for element in elements
            if element.get("type") == "way"
            and element.get("tags", {}).get("highway") in STREETS
        ]
        roundabouts = [t for t in tags if t.get("junction") in ROUNDABOUT]
        oneway = [t for t in tags if is_oneway(t)]
        lanes = [t for t in tags if "lanes" in t]
        odd_twoway = [
            t for t in lanes
            if not is_oneway(t) and t["lanes"] in ("1", "3", "5", "7")
        ]
        print(path)
        print(f"  streets >= 8 m:          {len(tags)}")
        print(f"  oneway (incl. rings):    {len(oneway)}")
        print(f"  roundabout ways:         {len(roundabouts)}")
        share = 100 * len(lanes) / len(tags) if tags else 0
        print(f"  with lanes tag:          {len(lanes)} ({share:.0f}%)")
        print(f"  lanes values:            {dict(collections.Counter(t['lanes'] for t in lanes))}")
        print(f"  two-way with odd lanes:  {len(odd_twoway)}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
