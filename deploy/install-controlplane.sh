#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Installs the Bennghe control plane with Docker Compose.
#
#   ./deploy/install-controlplane.sh --domain bennghe.example.com --email you@example.com
#
# It writes the configuration next to docker-compose.yml, creates the encryption and signing
# keys, prints a one-time setup token, starts everything and waits until HTTPS answers.
# Run it again to upgrade or repair: existing keys, passwords and data are never overwritten.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose_dir="$here/controlplane"
domain="" email="" image="" wait_seconds=300

usage() {
  cat <<EOF
Usage: $0 --domain <name> [--email <address>] [--image <image>] [--wait <seconds>]

  --domain   Public DNS name of this server. It must already point here, and ports 80 and 443
             must be reachable from the internet: the certificate is issued for this name.
  --email    Contact address for the certificate authority (recommended).
  --image    Use a prebuilt image instead of building from this checkout.
  --wait     How long to wait for HTTPS before giving up (default 300).
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --domain) domain="${2:-}"; shift 2 ;;
    --email) email="${2:-}"; shift 2 ;;
    --image) image="${2:-}"; shift 2 ;;
    --wait) wait_seconds="${2:-}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ -n "$domain" ] || { echo "error: --domain is required" >&2; usage >&2; exit 2; }
case "$domain" in
  *[!a-zA-Z0-9.-]* | .* | *. ) echo "error: '$domain' is not a valid host name" >&2; exit 2 ;;
esac
command -v docker >/dev/null || { echo "error: docker is not installed" >&2; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "error: the Docker Compose plugin is not installed" >&2; exit 1; }
command -v openssl >/dev/null || { echo "error: openssl is required to generate the database password" >&2; exit 1; }

cd "$compose_dir"
env_file="$compose_dir/.env"

# Settings other than the password are rewritten on every run; the password is created once,
# because the database was initialised with it.
password=""
if [ -f "$env_file" ]; then
  password="$(sed -n 's/^POSTGRES_PASSWORD=//p' "$env_file" | head -n1)"
fi
[ -n "$password" ] || password="$(openssl rand -hex 24)"

umask 077
{
  echo "POSTGRES_PASSWORD=$password"
  echo "BN_PUBLIC_DOMAIN=$domain"
  echo "BN_ACME_EMAIL=$email"
  [ -z "$image" ] || echo "BN_IMAGE=$image"
  # Passed through for private CAs and tests; unset in normal installs.
  [ -z "${BN_ACME_DIRECTORY:-}" ] || echo "BN_ACME_DIRECTORY=$BN_ACME_DIRECTORY"
} >"$env_file"
chmod 600 "$env_file"

if [ -z "$image" ]; then
  echo "==> Building the control plane image (this takes a few minutes the first time)"
  docker compose build controlplane
fi

echo "==> Starting the database"
docker compose up -d --wait postgres

echo "==> Creating keys (existing keys are kept)"
docker compose run --rm --no-deps controlplane init --data-dir /data

echo "==> Creating the one-time setup token"
setup_token="$(docker compose run --rm controlplane setup-token 2>/dev/null | tail -n1 || true)"

echo "==> Starting the control plane"
docker compose up -d controlplane

echo "==> Waiting for https://$domain (the certificate is requested on first start)"
deadline=$(( $(date +%s) + wait_seconds ))
until curl -fsS -o /dev/null "https://$domain/api/v1/health" 2>/dev/null; do
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "error: https://$domain did not answer within ${wait_seconds}s." >&2
    echo "Check that $domain points at this server, that ports 80 and 443 are open, and see:" >&2
    echo "  docker compose -f $compose_dir/docker-compose.yml logs controlplane" >&2
    exit 1
  fi
  sleep 3
done

echo
echo "Bennghe is running at https://$domain"
if [ -n "$setup_token" ]; then
  echo
  echo "Open https://$domain/setup and create the owner account with this one-time token:"
  echo
  echo "    $setup_token"
else
  echo "The installation already has an owner; no setup token was created."
fi
cat <<EOF

IMPORTANT: back up the encryption key now. Without it every stored secret is unrecoverable.

    docker compose -f $compose_dir/docker-compose.yml cp controlplane:/data/master.key ./master.key

Keep that file somewhere other than this server.
EOF
