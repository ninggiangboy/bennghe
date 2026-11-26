#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Deletes every lab machine whose name starts with LAB_PREFIX (default bn-lab). Machines created
# by anything else are never touched.
set -euo pipefail

prefix="${LAB_PREFIX:-bn-lab}"
command -v incus >/dev/null || { echo "error: incus is not installed" >&2; exit 1; }

names="$(incus list --format csv -c n | grep -E "^${prefix}-[0-9]+$" || true)"
if [ -z "$names" ]; then
  echo "no lab machines found"
  exit 0
fi
for name in $names; do
  echo "==> Deleting $name"
  incus delete --force "$name"
done
