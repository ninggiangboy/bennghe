#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Creates the lab: a few Ubuntu LTS virtual machines with Docker and WireGuard, on real kernels,
# for the phases that touch Swarm, WireGuard and iptables. Needs Incus on a Linux host.
#
#   test/lab/incus-up.sh                       # three 2 GiB machines
#   LAB_NODES=1 LAB_MEMORY=512MiB test/lab/incus-up.sh   # the agent memory measurement
#
# Settings (environment): LAB_NODES (3), LAB_MEMORY (2GiB), LAB_CPUS (2), LAB_PREFIX (bn-lab),
# LAB_IMAGE (images:ubuntu/24.04/cloud).
set -euo pipefail

nodes="${LAB_NODES:-3}"
memory="${LAB_MEMORY:-2GiB}"
cpus="${LAB_CPUS:-2}"
prefix="${LAB_PREFIX:-bn-lab}"
image="${LAB_IMAGE:-images:ubuntu/24.04/cloud}"

command -v incus >/dev/null || { echo "error: incus is not installed (this lab needs a Linux host)" >&2; exit 1; }

user_data='#cloud-config
package_update: true
packages: [docker.io, wireguard, curl, ca-certificates]
runcmd:
  - systemctl enable --now docker
'

for i in $(seq 1 "$nodes"); do
  name="$prefix-$i"
  if incus info "$name" >/dev/null 2>&1; then
    echo "==> $name already exists, leaving it as it is"
    continue
  fi
  echo "==> Creating $name (${memory} RAM, ${cpus} CPUs)"
  incus launch "$image" "$name" --vm \
    -c limits.memory="$memory" -c limits.cpu="$cpus" \
    -c cloud-init.user-data="$user_data"
done

for i in $(seq 1 "$nodes"); do
  name="$prefix-$i"
  echo "==> Waiting for $name to finish installing Docker"
  # The guest agent starts a little after boot, so retry until it answers.
  until incus exec "$name" -- cloud-init status --wait >/dev/null 2>&1; do sleep 3; done
  incus exec "$name" -- docker info >/dev/null
done

echo
echo "Lab ready:"
incus list "$prefix-" --format csv -c n4
