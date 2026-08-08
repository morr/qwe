#!/usr/bin/env bash
# Серия замеров подряд: каждая строка stdin — «подпись<TAB>аргументы демо».
#
#   tools/separation_lab/batch.sh результаты.txt <<'EOF'
#   baseline
#   hold-0	--hold 0
#   antic	--horizon 1.5 --anticipation 1.5
#   EOF
#
# Падение одного прогона не роняет серию: без строки `RESULT` пишется пометка и
# работа идёт дальше — иначе одна опечатка в ручке стоила бы всех оставшихся
# двадцати минут.
set -euo pipefail

cd "$(dirname "$0")/../.."
results=$1

while IFS=$'\t' read -r label args; do
    [[ -z ${label// /} || $label == \#* ]] && continue
    # shellcheck disable=SC2086  # аргументы обязаны разбиться по пробелам
    tools/separation_lab/run.sh "$results" "$label" ${args:-} || echo "FAILED $label" | tee -a "$results"
done
