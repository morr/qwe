#!/usr/bin/env bash
# Серия прогонов: строки «сценарий<TAB>подпись<TAB>аргументы» со стандартного
# ввода, по строке на прогон. Пустые строки и `#`-комментарии пропускаются.
#
#   tools/crowd_tuning_lab/batch.sh /tmp/res.txt <<'EOF'
#   2	base
#   2	pass	--pass-squeeze 0.6
#   3	pass	--pass-squeeze 0.6
#   EOF
set -uo pipefail

here=$(dirname "$0")
results=$1

while IFS=$'\t' read -r scenario label args; do
    [[ -z ${scenario:-} || $scenario == \#* ]] && continue
    # shellcheck disable=SC2086
    "$here/run.sh" "$scenario" "$results" "$label" ${args:-} || echo "!!! [$scenario] $label failed" >&2
done
