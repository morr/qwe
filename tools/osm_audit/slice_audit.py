#!/usr/bin/env python3
"""Срез города по глубине: здания и бастионы по полосам от портала к сердцу.

Spike 2 из `VISION.md` (раздел 12), ограниченный одним городом за запуск:
карта кладётся на город не от центра, а от **сердца** — сердце на `depth`
глубины от края портала, портал на противоположном краю. Скрипт считает по
десяти полосам глубины, что в такой рамке стоит: здания (Overpass `out count`
по bbox полосы), жилая/промышленная застройка (то же) и бастионы по тегам
(один запрос `out center` на всю рамку, раскладка по полосам локально).

    python3 tools/osm_audit/slice_audit.py tula 54.1930 37.6200 north
    python3 tools/osm_audit/slice_audit.py tula 54.1930 37.6200 west --depth 0.7
    STORE=/tmp/tula_north.json python3 tools/osm_audit/slice_audit.py ...

Позиционные аргументы: город (для подписи и `lon_scale`), широта и долгота
сердца, край портала (`north|south|east|west`). Рамка — `MAP_SIZE` из
`settings.rs`, гео-центр вычисляется так же, как его будет считать
`City::geo_center` для города со срезом: сердце, сдвинутое к краю портала на
`(depth − 0.5) × протяжённость карты вдоль оси`.

Сеть — как в `overpass_counts.py`: мимо прокси, мелкими пачками, результат
копится в JSON, перезапуск дособирает недостающее.
"""

import argparse
import json
import math
import os
import sys
import time
import urllib.parse
import urllib.request

# settings.rs::MAP_SIZE / METERS_PER_DEG_LAT
MAP_SIZE = (5600.0, 3700.0)
METERS_PER_DEG_LAT = 111_320.0
BANDS = 10

# Теги бастионов M1 (`VISION.md` 2.2). Ключ — `BastionKind`-кандидат.
BASTION_TAGS = {
    "police": 'nwr["amenity"="police"]({bbox});',
    "fire": 'nwr["amenity"="fire_station"]({bbox});',
    "church": 'nwr["amenity"="place_of_worship"]({bbox});',
    "military": 'nwr["military"]({bbox});way["landuse"="military"]({bbox});',
}
# Два бастиона одного вида ближе этого — один объект, размеченный дважды
# (нода внутри контура, контур плюс relation).
DEDUP_METERS = 30.0

# Счётчики по полосе: подпись → тело запроса.
BAND_COUNTS = [
    ("buildings", 'way["building"]({bbox});relation["building"]({bbox});'),
    ("residential", 'way["landuse"="residential"]({bbox});relation["landuse"="residential"]({bbox});'),
    ("industrial", 'way["landuse"~"^(industrial|railway|brownfield)$"]({bbox});'),
    ("roads", 'way["highway"]({bbox});'),
]

NO_PROXY = urllib.request.build_opener(urllib.request.ProxyHandler({}))
ENDPOINTS = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
]
BATCH = 4
STORE = os.environ.get("STORE", "slice_audit.json")


class Frame:
    """Рамка среза: гео-центр, bbox и проекция в метры карты
    (`overpass.rs::GeoBounds`, начало координат — юго-западный угол)."""

    def __init__(self, heart_lat, heart_lon, edge, depth):
        self.edge = edge
        self.lon_scale = METERS_PER_DEG_LAT * math.cos(math.radians(heart_lat))
        # ось глубины: north/south — вдоль y (3700 м), east/west — вдоль x
        self.along_y = edge in ("north", "south")
        extent = MAP_SIZE[1] if self.along_y else MAP_SIZE[0]
        shift = (depth - 0.5) * extent
        # сдвиг центра К краю портала: портал north → центр севернее сердца
        sign = {"north": 1, "south": -1, "east": 1, "west": -1}[edge]
        if self.along_y:
            self.lat = heart_lat + sign * shift / METERS_PER_DEG_LAT
            self.lon = heart_lon
        else:
            self.lat = heart_lat
            self.lon = heart_lon + sign * shift / self.lon_scale
        half_lat = MAP_SIZE[1] / 2 / METERS_PER_DEG_LAT
        half_lon = MAP_SIZE[0] / 2 / self.lon_scale
        self.south = self.lat - half_lat
        self.west = self.lon - half_lon
        self.north = self.lat + half_lat
        self.east = self.lon + half_lon
        self.heart = self.project(heart_lat, heart_lon)

    def project(self, lat, lon):
        return ((lon - self.west) * self.lon_scale, (lat - self.south) * METERS_PER_DEG_LAT)

    def depth_of(self, point):
        """Глубина 0…1 от края портала."""
        x, y = point
        return {
            "north": (MAP_SIZE[1] - y) / MAP_SIZE[1],
            "south": y / MAP_SIZE[1],
            "east": (MAP_SIZE[0] - x) / MAP_SIZE[0],
            "west": x / MAP_SIZE[0],
        }[self.edge]

    def band_of(self, point):
        return min(BANDS - 1, max(0, int(self.depth_of(point) * BANDS)))

    def band_bbox(self, band):
        """bbox полосы `band` (0 — полоса портала) в гео."""
        lo, hi = band / BANDS, (band + 1) / BANDS
        if self.along_y:
            span = self.north - self.south
            # глубина от севера растёт на юг
            if self.edge == "north":
                lat_hi, lat_lo = self.north - lo * span, self.north - hi * span
            else:
                lat_lo, lat_hi = self.south + lo * span, self.south + hi * span
            return (lat_lo, self.west, lat_hi, self.east)
        span = self.east - self.west
        if self.edge == "east":
            lon_hi, lon_lo = self.east - lo * span, self.east - hi * span
        else:
            lon_lo, lon_hi = self.west + lo * span, self.west + hi * span
        return (self.south, lon_lo, self.north, lon_hi)

    def bbox_str(self, box=None):
        box = box or (self.south, self.west, self.north, self.east)
        return ",".join(f"{value:.6f}" for value in box)


def post(body, parse):
    last = None
    for attempt in range(8):
        endpoint = ENDPOINTS[attempt % len(ENDPOINTS)]
        try:
            request = urllib.request.Request(
                endpoint,
                data=urllib.parse.urlencode({"data": body}).encode(),
                headers={"User-Agent": "qwe-map-audit/1.0"},
            )
            with NO_PROXY.open(request, timeout=180) as response:
                return parse(json.loads(response.read()))
        except Exception as error:  # noqa: BLE001 — сеть, интересен только факт
            last = f"{endpoint}: {type(error).__name__} {error}"
            time.sleep(8)
    raise RuntimeError(last)


def fetch_counts(body):
    return post(
        body,
        lambda payload: [
            int(element["tags"]["total"])
            for element in payload["elements"]
            if element["type"] == "count"
        ],
    )


def fetch_bastions(frame):
    """Все бастионы рамки: (вид, имя, метры карты). Way/relation — по
    `center`, нода — по своим координатам."""
    body = "[out:json][timeout:120];\n"
    for kind, template in BASTION_TAGS.items():
        body += "(" + template.format(bbox=frame.bbox_str()) + ")->.%s;\n" % kind
    body += "(" + ";".join(f".{kind}" for kind in BASTION_TAGS) + ";);\nout center tags;\n"
    elements = post(body, lambda payload: payload["elements"])
    found = []
    for element in elements:
        tags = element.get("tags", {})
        if tags.get("amenity") == "police":
            kind = "police"
        elif tags.get("amenity") == "fire_station":
            kind = "fire"
        elif tags.get("amenity") == "place_of_worship":
            kind = "church"
        elif "military" in tags or tags.get("landuse") == "military":
            kind = "military"
        else:
            continue
        center = element.get("center", element)
        if "lat" not in center:
            continue
        point = frame.project(center["lat"], center["lon"])
        # bbox ловит way по любой ноде, а центр большого полигона (полигон
        # `landuse=military` в полгорода) лежит за картой — такой бастион в
        # игре тоже отбросят, здесь его не считаем
        if not (0 <= point[0] <= MAP_SIZE[0] and 0 <= point[1] <= MAP_SIZE[1]):
            continue
        found.append((kind, tags.get("name", "?"), point))
    # дедуп: тот же вид в DEDUP_METERS — один бастион
    kept = []
    for kind, name, point in found:
        if any(
            k == kind and math.dist(p, point) < DEDUP_METERS for k, _, p in kept
        ):
            continue
        kept.append((kind, name, point))
    return kept, len(found)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("city")
    parser.add_argument("heart_lat", type=float)
    parser.add_argument("heart_lon", type=float)
    parser.add_argument("edge", choices=["north", "south", "east", "west"])
    parser.add_argument("--depth", type=float, default=0.7)
    args = parser.parse_args()

    frame = Frame(args.heart_lat, args.heart_lon, args.edge, args.depth)
    key = f"{args.city}:{args.edge}:{args.depth}:{args.heart_lat},{args.heart_lon}"
    store = json.load(open(STORE)) if os.path.exists(STORE) else {}
    done = store.setdefault(key, {})

    print(f"===== {args.city}: портал {args.edge}, сердце на {args.depth} глубины")
    print(f"  geo_center = ({frame.lat:.5f}, {frame.lon:.5f})")
    print(f"  bbox       = {frame.bbox_str()}")
    print(f"  сердце в метрах карты = ({frame.heart[0]:.0f}, {frame.heart[1]:.0f})")
    sys.stdout.flush()

    def save():
        with open(STORE, "w") as handle:
            json.dump(store, handle, ensure_ascii=False, indent=1)

    # счётчики по полосам, пачками по BATCH полос на одну статистику
    for label, template in BAND_COUNTS:
        counts = done.setdefault(label, {})
        todo = [band for band in range(BANDS) if str(band) not in counts]
        for start in range(0, len(todo), BATCH):
            chunk = todo[start : start + BATCH]
            body = "[out:json][timeout:120];\n"
            for band in chunk:
                body += "(" + template.format(bbox=frame.bbox_str(frame.band_bbox(band))) + ");out count;\n"
            try:
                totals = fetch_counts(body)
            except RuntimeError as error:
                print(f"     СБОЙ {label} {chunk} ({error})")
                sys.stdout.flush()
                continue
            for band, total in zip(chunk, totals):
                counts[str(band)] = total
            save()
            print(f"  {label}: полосы {chunk} готовы")
            sys.stdout.flush()

    if "bastions" not in done:
        kept, raw = fetch_bastions(frame)
        done["bastions"] = {
            "raw": raw,
            "list": [
                {"kind": kind, "name": name, "x": round(x), "y": round(y), "band": frame.band_of((x, y))}
                for kind, name, (x, y) in kept
            ],
        }
        save()

    bastions = done["bastions"]["list"]
    header = f"{'полоса':>6} {'глубина':>9} " + " ".join(f"{label:>11}" for label, _ in BAND_COUNTS)
    header += " " + " ".join(f"{kind:>8}" for kind in BASTION_TAGS)
    print()
    print(header)
    for band in range(BANDS):
        lo, hi = band / BANDS, (band + 1) / BANDS
        mark = " ← портал" if band == 0 else (" ← сердце" if lo <= args.depth < hi else "")
        row = f"{band:>6} {lo:>4.1f}–{hi:<4.1f} "
        row += " ".join(f"{done.get(label, {}).get(str(band), '?'):>11}" for label, _ in BAND_COUNTS)
        row += " " + " ".join(
            f"{sum(1 for b in bastions if b['band'] == band and b['kind'] == kind):>8}"
            for kind in BASTION_TAGS
        )
        print(row + mark)
    print(f"\n  бастионов всего: {len(bastions)} (до дедупа {done['bastions']['raw']})")
    for b in sorted(bastions, key=lambda b: (b["band"], b["kind"])):
        print(f"    полоса {b['band']}  {b['kind']:<8} ({b['x']:>5}, {b['y']:>5})  {b['name']}")


if __name__ == "__main__":
    main()
