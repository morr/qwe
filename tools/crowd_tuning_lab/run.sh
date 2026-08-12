#!/usr/bin/env bash
# Один прогон стенда «слоты + расталкивание» на ЛЮБОМ сценарии
# (`examples/demos/crowd_demo/`), окно — 15 РЕАЛЬНЫХ секунд.
#
#   tools/crowd_tuning_lab/run.sh <сценарий 2|3> <файл-результатов> <подпись> [аргументы демо…]
#
# Отличия от двух прошлых стендов (`separation_lab`, `separation_slots_lab`):
# окно 15 с вместо 8 (сходящейся толпе нужно не только дойти, но и ОСЕСТЬ), и
# сценарий — параметр, потому что оба критерия проверяются в паре: то, что
# помогает сходящейся толпе, обязано не ломать встречный поток.
#
# Базовые аргументы одни на всю серию — меняться от прогона к прогону должно
# ровно то, что меряют. Из вывода демо забирается строка `RESULT`; сцена
# выходит сама по истечении окна, убивать её снаружи не нужно.
set -euo pipefail

cd "$(dirname "$0")/../.."

scenario=$1
results=$2
label=$3
shift 3

# Зум под сценарий: расталкивания за кадром нет по построению, и стартовавший
# вне кадра хвост мерил бы систему там, где её нет.
#   2 (воронка) — 0.12: обод радиуса 45 м целиком в кадре с нулевого кадра;
#   3 (колонны) — 0.15: колонны длиной 40 м плюс разлёт.
case "$scenario" in
    2) base=(--scenario 2 --speed 5 --seconds 15 --zoom 0.12) ;;
    3) base=(--scenario 3 --speed 5 --seconds 15 --zoom 0.15 --width 8) ;;
    *) echo "usage: run.sh <2|3> <results> <label> [args…]" >&2; exit 2 ;;
esac

echo "=== [$scenario] $label ${*:-}" >&2
line=$(cargo run --quiet --example crowd_demo -- "${base[@]}" --label "$label" "$@" 2>/dev/null \
    | grep '^RESULT' || true)

if [[ -z $line ]]; then
    echo "!!! $label produced no RESULT line" >&2
    exit 1
fi

echo "$line SCENARIO=$scenario ARGS=${*:-none}" | tee -a "$results"
