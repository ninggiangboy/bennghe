#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Usage (inside the spike container): measure.sh <label> <agent-binary> [env assignments...]
# Starts the fake server (once), runs the agent, samples RSS at fixed offsets.
label=$1; bin=$2; shift 2
pgrep -f bn-spike-server >/dev/null || { /spike/rust/server/out/release/bn-spike-server /run/spike >/run/spike/server.log 2>&1 & sleep 1; }
env "$@" "$bin" >"/run/spike/$label.log" 2>&1 &
pid=$!
start=$(date +%s)
for t in 20 60 120 200 300 330; do
  now=$(( $(date +%s) - start )); [ $t -gt $now ] && sleep $(( t - now ))
  rss=$(awk '/VmRSS/{print $2}' /proc/$pid/status 2>/dev/null)
  hwm=$(awk '/VmHWM/{print $2}' /proc/$pid/status 2>/dev/null)
  ps_rss=$(ps -o rss= -p $pid 2>/dev/null | tr -d ' ')
  echo "$label t=${t}s VmRSS=${rss}kB VmHWM=${hwm}kB ps_rss=${ps_rss}kB"
done
kill $pid 2>/dev/null
