#!/usr/bin/env python3
"""Таблица из строк `RESULT` — по одной строке на подпись, с медианой повторов.

    tools/crowd_tuning_lab/table.py /tmp/res.txt 2 base core core-pass

Аргументы после файла: сценарий (2 или 3) и подписи в нужном порядке; без
подписей печатаются все, в порядке первого появления. Повторы одной подписи
сворачиваются в медиану — единичному прогону по `through` верить нельзя (см.
REPORT.md прошлых стендов), а по остальным метрикам разброс ±3 %.
"""

import sys
from statistics import median

# что показываем и как: (ключ, заголовок, формат)
COLUMNS = [
    ("on_slot", "on_slot", "{:.0f}"),
    ("settled", "settled", "{:.0f}"),
    ("walking", "walking", "{:.0f}"),
    ("med_travel", "med_travel", "{:.1f}"),
    ("med_progress", "med_prog", "{:.1f}"),
    ("med_sep", "med_sep", "{:.2f}"),
    ("med_sep_share", "med_sep%", "{:.3f}"),
    ("travel", "travel", "{:.0f}"),
    ("net", "net", "{:.0f}"),
    ("idle_drift", "drift", "{:.0f}"),
    ("foot", "foot", "{:.1f}"),
    ("sep_share", "sep_share", "{:.3f}"),
    ("spread", "spread", "{:.2f}"),
    ("lane_order", "lanes", "{:.2f}"),
    ("through", "through", "{:.0f}"),
    ("worst_step", "worst_step", "{:.3f}"),
    ("arrivals", "arrivals", "{:.0f}"),
    ("sep_ms", "sep_ms", "{:.3f}"),
]


def parse(path):
    rows = []
    for line in open(path):
        if not line.startswith("RESULT"):
            continue
        row = {}
        for token in line.split():
            if "=" in token:
                key, _, value = token.partition("=")
                row[key] = value
        rows.append(row)
    return rows


def main():
    path = sys.argv[1]
    scenario = sys.argv[2] if len(sys.argv) > 2 else None
    wanted = sys.argv[3:]

    rows = [r for r in parse(path) if scenario is None or r.get("SCENARIO") == scenario]
    order = wanted or list(dict.fromkeys(r["label"] for r in rows))

    header = ["variant", "n"] + [title for _, title, _ in COLUMNS]
    print("| " + " | ".join(header) + " |")
    print("|" + "|".join("---" for _ in header) + "|")
    for label in order:
        group = [r for r in rows if r["label"] == label]
        if not group:
            continue
        cells = [label, str(len(group))]
        for key, _, fmt in COLUMNS:
            values = [float(r[key]) for r in group if key in r]
            cells.append(fmt.format(median(values)) if values else "-")
        print("| " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
