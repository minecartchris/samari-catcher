#!/bin/bash
# Samari Catcher deployment script
# Usage: ./deploy.sh [user@]host

set -e

HOST="${1:-root@192.168.1.242}"
KEY="$HOME/.ssh/wordel_deploy"
DIR="/var/www/cloud-catcher"
SERVICE="cloud-catcher"

echo "=== Building ==="
npm run build

echo "=== Stopping service ==="
ssh -i "$KEY" "$HOST" "systemctl stop $SERVICE"

echo "=== Uploading ==="
scp -i "$KEY" -r _bin _site "$HOST:$DIR/"

echo "=== Starting service ==="
ssh -i "$KEY" "$HOST" "systemctl start $SERVICE && systemctl status $SERVICE --no-pager"

echo "=== Done! ==="
ssh -i "$KEY" "$HOST" "curl -s localhost:8080/ | head -5"