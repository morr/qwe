#!/usr/bin/env python3
"""Что из выгрузки Overpass парсер берёт, а что выбрасывает.

Повторяет правила `src/map/osm/parse.rs` по кешу `assets/osm/*.json` и печатает
две колонки: KEPT (дошло до `MapData`) и DROPPED (скачали, но не рисуем).
Обновлять вместе с парсером — иначе аудит врёт.

    python3 tools/osm_audit/cache_audit.py assets/osm/*.json
"""

import collections
import json
import os
import sys

# parse.rs::road_class
ROAD_OK = {
    "motorway", "trunk", "primary", "secondary", "tertiary",
    "residential", "unclassified", "living_street", "service",
    "footway", "path", "pedestrian", "cycleway", "steps", "track",
}
# parse.rs::rail_class
RAIL_OK = {
    "rail", "light_rail", "narrow_gauge", "subway", "tram",
    "abandoned", "disused", "razed", "dismantled",
}
# parse.rs::water_class — линейные водотоки; `riverbank` тут не значится,
# это площадь и её забирает area_kind
WATER_OK = {"river", "canal", "weir", "stream", "brook", "ditch", "drain"}
# parse.rs::NON_WALKABLE_ENTRANCES
NON_WALKABLE = {"no", "garage", "emergency"}


def area_kind(tags):
    """parse.rs::area_kind — порядок веток важен, луг проверяется до парка."""
    if "building" in tags:
        return "building"
    natural, landuse = tags.get("natural"), tags.get("landuse")
    if natural == "water" or tags.get("waterway") == "riverbank":
        return "water"
    if natural in ("sand", "beach"):
        return "sand"
    if landuse in ("grass", "meadow") or natural in ("grassland", "meadow"):
        return "grass"
    if natural == "wood" or landuse == "forest":
        return "wood"
    if tags.get("leisure") in ("park", "garden") or landuse == "recreation_ground":
        return "park"
    return None


def below_zero(tags):
    try:
        return float(tags.get("layer", "0")) < 0.0
    except ValueError:
        return False


def underground(tags):
    """parse.rs::is_underground — `tunnel` кроме `no`/`building_passage`, либо `layer<0`."""
    tunnel = tags.get("tunnel") not in (None, "no", "building_passage")
    return tunnel or below_zero(tags)


def building_passage(tags):
    """parse.rs::is_building_passage — арка, проезд сквозь дом."""
    return tags.get("tunnel") == "building_passage" or tags.get("covered") in (
        "yes",
        "building_passage",
    )


def road_underground(tags):
    """parse.rs::is_road_underground.

    Мост и арка правилу не подчиняются (обе роли — на уровне ходьбы, и обе
    режут навмеш), а `tunnel=culvert` описывает ручей, а не улицу над ним.
    """
    if tags.get("bridge") not in (None, "no") or building_passage(tags):
        return False
    if tags.get("tunnel") == "culvert":
        return below_zero(tags)
    return underground(tags)


def analyse(path):
    with open(path) as handle:
        elements = json.load(handle)["elements"]

    kept, dropped = collections.Counter(), collections.Counter()
    unclassified = []

    for element in elements:
        tags = element.get("tags", {})
        kind = element["type"]

        if kind == "node":
            if tags.get("natural") == "tree":
                kept["tree node"] += 1
                continue
            entrance = tags.get("entrance")
            if entrance is None:
                dropped["node: ни вход, ни дерево"] += 1
            elif entrance in NON_WALKABLE:
                dropped[f"node: entrance={entrance}"] += 1
            else:
                kept["entrance node"] += 1
            continue

        if kind == "way":
            # рельсы и аллеи разбираются до дорог и проваливаются дальше:
            # way бывает одновременно `railway=tram` и `highway=*`
            claimed = False
            railway = tags.get("railway")
            if railway is not None:
                claimed = True
                if railway not in RAIL_OK:
                    dropped[f"way railway={railway}"] += 1
                elif underground(tags):
                    dropped[f"way railway={railway} (под землёй)"] += 1
                else:
                    kept[f"rail {railway}"] += 1
            if tags.get("natural") == "tree_row":
                kept["tree_row"] += 1
                claimed = True

            # водоток — тоже до дорог и тоже проваливается дальше: ручей в
            # трубе под улицей размечен на том же way, что и `highway=*`.
            # Значение не из белого списка тут не считается: в парсере оно
            # просто едет дальше, в area_kind (`riverbank` там станет водой,
            # `dam`/`dock` — «не классифицировано»)
            waterway = tags.get("waterway")
            if waterway in WATER_OK:
                kept["culvert" if underground(tags) else f"waterway {waterway}"] += 1
                claimed = True

            highway = tags.get("highway")
            if highway is not None:
                if highway not in ROAD_OK:
                    dropped[f"way highway={highway}"] += 1
                elif road_underground(tags):
                    dropped[f"way highway={highway} (под землёй)"] += 1
                else:
                    kept["road"] += 1
                continue
            if claimed:
                continue
            if tags.get("barrier") == "city_wall":
                kept["city_wall"] += 1
                continue

            area = area_kind(tags)
            geometry = element.get("geometry") or []
            closed = len(geometry) >= 4 and geometry[0] == geometry[-1]
            if area is None:
                dropped["way area: не классифицировано"] += 1
                if len(unclassified) < 5:
                    unclassified.append(tags)
            elif not closed:
                dropped[f"way {area}: незамкнутое кольцо"] += 1
            else:
                kept[f"area {area}"] += 1
            continue

        if kind == "relation":
            area = area_kind(tags)
            if area is None:
                dropped["relation: не классифицировано"] += 1
                if len(unclassified) < 5:
                    unclassified.append(tags)
            else:
                kept[f"relation {area}"] += 1

    return kept, dropped, unclassified


def main(paths):
    for path in paths:
        kept, dropped, unclassified = analyse(path)
        print("=" * 70)
        print(os.path.basename(path))
        print("-- KEPT")
        for label, count in kept.most_common():
            print(f"   {count:>7} {label}")
        print("-- DROPPED")
        for label, count in dropped.most_common(40):
            print(f"   {count:>7} {label}")
        for tags in unclassified:
            print(f"   пример: {json.dumps(tags, ensure_ascii=False)[:200]}")
        sys.stdout.flush()


if __name__ == "__main__":
    main(sys.argv[1:])
