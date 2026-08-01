#!/usr/bin/env python3
"""Сколько в bbox города лежит того, что запрос игры даже не спрашивает.

Считает через Overpass `out count` по тому же bbox, что и `GeoBounds::for_city`.
Результат копится в JSON, поэтому перезапуск дособирает недостающее.

    python3 tools/osm_audit/overpass_counts.py tula paris berlin london tokyo ny
    STORE=/tmp/paris.json python3 tools/osm_audit/overpass_counts.py paris

Две вещи, без которых это не работает:

* **Мимо прокси.** `HTTP(S)_PROXY` из окружения ведёт на общий адрес, которому
  Overpass отвечает 429/504, поэтому запросы идут через опенер с пустым
  `ProxyHandler`.
* **Мелкими пачками.** Один запрос на все 37 счётчиков сервер отбивает по
  таймауту; по три статистики за раз проходит. Города считаются независимо —
  запускать их можно параллельно, каждый со своим `STORE`.
"""

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

# city.rs::geo_center
CITIES = {
    "tula": (54.18969, 37.59148),
    "paris": (48.85565, 2.34612),
    "berlin": (52.519, 13.40133),
    "london": (51.5119, -0.1224),
    "tokyo": (35.6895, 139.729),
    "ny": (40.70979, -73.97284),
}

# (метка, тело запроса; {bbox} подставляется). REF — то, что уже рисуется:
# держим для сверки с кешем, счётчик обязан совпасть с `cache_audit.py`.
GROUPS = [
    ("REF railway ways (рисуем)", 'way["railway"]({bbox});'),
    ("REF natural=tree ноды (рисуем)", 'node["natural"="tree"]({bbox});'),
    ("REF natural=tree_row ways (рисуем)", 'way["natural"="tree_row"]({bbox});'),
    ("natural=hedge (ways)", 'way["natural"="hedge"]({bbox});'),
    ("barrier=hedge (ways)", 'way["barrier"="hedge"]({bbox});'),
    ("barrier кроме hedge и city_wall", 'way["barrier"]["barrier"!="city_wall"]["barrier"!="hedge"]({bbox});'),
    ("входы в метро (ноды)", 'node["railway"="subway_entrance"]({bbox});'),
    ("линейные waterway", 'way["waterway"~"^(river|stream|canal|drain|ditch)$"]({bbox});'),
    ("парковки", 'way["amenity"="parking"]({bbox});relation["amenity"="parking"]({bbox});'),
    ("кладбища", 'way["landuse"="cemetery"]({bbox});relation["landuse"="cemetery"]({bbox});way["amenity"="grave_yard"]({bbox});'),
    ("landuse industrial/commercial/retail", 'way["landuse"~"^(industrial|commercial|retail)$"]({bbox});relation["landuse"~"^(industrial|commercial|retail)$"]({bbox});'),
    ("landuse residential", 'way["landuse"="residential"]({bbox});relation["landuse"="residential"]({bbox});'),
    ("landuse farm/orchard/allotments", 'way["landuse"~"^(farmland|farmyard|allotments|orchard|vineyard|greenhouse_horticulture)$"]({bbox});'),
    ("landuse brownfield/quarry/military/railway", 'way["landuse"~"^(brownfield|greenfield|construction|quarry|landfill|military|railway)$"]({bbox});'),
    ("landuse village_green", 'way["landuse"="village_green"]({bbox});'),
    ("leisure=pitch", 'way["leisure"="pitch"]({bbox});'),
    ("leisure playground/sports/stadium/pool", 'way["leisure"~"^(playground|sports_centre|stadium|track|swimming_pool|golf_course|marina|dog_park|common|fitness_station|water_park)$"]({bbox});'),
    ("natural scrub/heath/wetland/rock", 'way["natural"~"^(scrub|heath|wetland|bare_rock|scree|shingle|cliff|coastline)$"]({bbox});'),
    ("relation natural=wood (нет в запросе)", 'relation["natural"="wood"]({bbox});'),
    ("relation natural=grassland/meadow (нет в запросе)", 'relation["natural"~"^(grassland|meadow)$"]({bbox});'),
    ("highway с area=yes (площади)", 'way["highway"]["area"="yes"]({bbox});'),
    ("area:highway (покрытия дорог)", 'way["area:highway"]({bbox});'),
    ("man_made", 'way["man_made"]({bbox});'),
    ("building:part", 'way["building:part"]({bbox});'),
    ("amenity-ноды", 'node["amenity"]({bbox});'),
    ("shop-ноды", 'node["shop"]({bbox});'),
    ("amenity-полигоны без building", 'way["amenity"]["building"!~"."]["amenity"!="parking"]({bbox});'),
    ("остановки и платформы (ноды)", 'node["highway"="bus_stop"]({bbox});node["public_transport"="platform"]({bbox});'),
    ("переходы и светофоры (ноды)", 'node["highway"~"^(crossing|traffic_signals)$"]({bbox});'),
    ("фонари, скамейки, фонтаны (ноды)", 'node["highway"="street_lamp"]({bbox});node["amenity"~"^(bench|fountain|waste_basket|drinking_water)$"]({bbox});'),
    ("power", 'way["power"]({bbox});node["power"]({bbox});'),
    ("tourism", 'node["tourism"]({bbox});way["tourism"]({bbox});'),
    ("historic", 'node["historic"]({bbox});way["historic"]({bbox});'),
    ("place-подписи", 'node["place"]({bbox});'),
    ("адресные ноды", 'node["addr:housenumber"]["entrance"!~"."]({bbox});'),
    ("aeroway", 'way["aeroway"]({bbox});'),
    ("REF footway/pedestrian ways (рисуем)", 'way["highway"~"^(footway|path|pedestrian|steps)$"]({bbox});'),
]

NO_PROXY = urllib.request.build_opener(urllib.request.ProxyHandler({}))
ENDPOINTS = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
]
BATCH = 3
STORE = os.environ.get("STORE", "osm_counts.json")


def bbox(lat, lon):
    """overpass.rs::GeoBounds::for_city — та же равнопромежуточная рамка."""
    lon_scale = METERS_PER_DEG_LAT * math.cos(math.radians(lat))
    half_lat = MAP_SIZE[1] / 2 / METERS_PER_DEG_LAT
    half_lon = MAP_SIZE[0] / 2 / lon_scale
    return (lat - half_lat, lon - half_lon, lat + half_lat, lon + half_lon)


def fetch(body):
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
                payload = json.loads(response.read())
            return [
                int(element["tags"]["total"])
                for element in payload["elements"]
                if element["type"] == "count"
            ]
        except Exception as error:  # noqa: BLE001 — сеть, интересен только факт
            last = f"{endpoint}: {type(error).__name__} {error}"
            time.sleep(8)
    raise RuntimeError(last)


def main(cities):
    store = json.load(open(STORE)) if os.path.exists(STORE) else {}
    for city in cities:
        area = ",".join(str(value) for value in bbox(*CITIES[city]))
        done = store.setdefault(city, {})
        todo = [group for group in GROUPS if group[0] not in done]
        print(f"===== {city}: {len(done)} готово, {len(todo)} осталось")
        sys.stdout.flush()

        for start in range(0, len(todo), BATCH):
            chunk = todo[start : start + BATCH]
            body = "[out:json][timeout:120];\n"
            for _, template in chunk:
                body += "(" + template.format(bbox=area) + ");out count;\n"
            try:
                totals = fetch(body)
            except RuntimeError as error:
                print(f"     СБОЙ {[label for label, _ in chunk]} ({error})")
                sys.stdout.flush()
                continue
            for (label, _), total in zip(chunk, totals):
                done[label] = total
                print(f"   {total:>7}  {label}")
            with open(STORE, "w") as handle:
                json.dump(store, handle, ensure_ascii=False, indent=1)
            sys.stdout.flush()


if __name__ == "__main__":
    main(sys.argv[1:] or list(CITIES))
