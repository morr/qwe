#!/usr/bin/env python3
"""Строки `RESULT` из файла результатов — в таблицу markdown для отчёта.

    tools/separation_slots_lab/table.py <файл> [подпись…]

Без подписей печатает все прогоны в порядке появления; с подписями — только
названные и в названном порядке (так таблица в отчёте собирается по смыслу, а
не по тому, в каком порядке гонялась серия).
"""

import sys

COLUMNS = [
    ("label", "вариант", "{}"),
    ("settled", "settled", "{}"),
    ("settled_at", "settled_at", "{}"),
    ("travel", "travel", "{}"),
    ("net", "net", "{}"),
    ("detour", "detour", "{}"),
    ("idle_drift", "idle_drift", "{}"),
    ("sep_share", "sep_share", "{}"),
    ("through", "through", "{}"),
    ("foot", "foot", "{}"),
    ("sep_ms", "sep_ms", "{}"),
]


def parse(path):
    runs = {}
    order = []
    for line in open(path):
        if not line.startswith("RESULT"):
            continue
        fields = {}
        for token in line.split():
            if "=" in token:
                key, _, value = token.partition("=")
                fields[key] = value
        label = fields.get("label")
        if label not in runs:
            order.append(label)
        runs[label] = fields
    return runs, order


def main():
    path = sys.argv[1]
    runs, order = parse(path)
    wanted = sys.argv[2:] or order
    print("| " + " | ".join(title for _, title, _ in COLUMNS) + " |")
    print("|" + "---|" * len(COLUMNS))
    for label in wanted:
        run = runs.get(label)
        if run is None:
            print(f"!!! нет прогона {label}", file=sys.stderr)
            continue
        cells = [fmt.format(run.get(key, "—")) for key, _, fmt in COLUMNS]
        print("| " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
