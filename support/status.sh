#!/bin/bash

set -e  # Exit on any error

echo "Getting status of existing eoi- services..."
for service in $(systemctl list-units --type=service --all | grep -o 'eoi-[^ ]*'); do
    echo "  -> Getting satus $service"
    SYSTEMD_PAGER=tail sudo systemctl --no-pager status "$service"
done
