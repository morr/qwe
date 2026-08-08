#!/usr/bin/env bash
# Серия прогонов из stdin: строка = «подпись<TAB>аргументы».
#
#   tools/separation_slots_lab/batch.sh /tmp/res.txt <<'EOF'
#   base
#   winner	--steer 1 --hold 1 --compress 0.2 --rate 4
#   EOF
set -euo pipefail

here=$(dirname "$0")
results=$1

while IFS=$'\t' read -r label args; do
    [[ -z ${label// /} || $label == \#* ]] && continue
    # shellcheck disable=SC2086
    "$here/run.sh" "$results" "$label" ${args:-} || true
done
