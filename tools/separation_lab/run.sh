#!/usr/bin/env bash
# Прогон серии замеров расталкивания на `examples/demos/crowd_demo.rs`.
#
#   tools/separation_lab/run.sh <файл-результатов> <подпись> [аргументы демо…]
#
# Базовые аргументы одни на всю серию (сценарий, скорость, окно, зум, толпа) —
# меняться от прогона к прогону должно ровно то, что меряют, и ничего больше.
# Из вывода демо забирается одна строка `RESULT` и дописывается в файл; сцена
# выходит сама по истечении окна, убивать её снаружи не нужно.
set -euo pipefail

cd "$(dirname "$0")/../.."

results=$1
label=$2
shift 2

# зум 0.15: при 1400 px окна это ±105 м, то есть обе колонны по 40 пешек с
# шагом 2 м (хвост на ±98 м) целиком в кадре. Считаются только пешки в кадре —
# за ним расталкивания нет по построению, и вылезший хвост занижал бы всё
base=(--scenario 3 --speed 5 --seconds 15 --zoom 0.15 --pawns 40)

echo "=== $label ${*:-}" >&2
line=$(cargo run --quiet --example crowd_demo -- "${base[@]}" --label "$label" "$@" 2>/dev/null \
    | grep '^RESULT' || true)

if [[ -z $line ]]; then
    echo "!!! $label produced no RESULT line" >&2
    exit 1
fi

echo "$line ARGS=${*:-none}" | tee -a "$results"
