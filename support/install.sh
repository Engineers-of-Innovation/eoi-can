#!/bin/bash

#set -e  # Exit on any error

USER_DIR="/home/engineer"
SUPPORT_DIR="${USER_DIR}/support"
SYSTEMD_DIR="/etc/systemd/system"
IGNORE_SERVICE="eoi-can-init.service"

echo "Stopping and disabling existing eoi- services..."
for service in $(systemctl list-units --type=service --all | grep -o 'eoi-[^ ]*'); do
    if [[ "$service" == "$IGNORE_SERVICE" ]]; then
        echo "  -> Skipping $service (ignore service)"
        continue
    fi
    echo "  -> Stopping $service"
    sudo systemctl stop "$service" || true
    echo "  -> Disabling $service"
    sudo systemctl disable "$service" || true
    echo "  -> Removing $service file"
    sudo rm -f "$SYSTEMD_DIR/$service"
done

echo "Copying new programs that end in .new"
for file in "$USER_DIR"/*.new; do

    if [[ ! -f "$file" ]]; then
        echo "  -> No .new files found in $USER_DIR"
        continue
    fi
    echo "  -> Found $file"
    # Remove the .new suffix
    file_without_dot_new="${file%.new}"

    echo "  -> Renaming $file to $file_without_dot_new"
    mv "$file" "$file_without_dot_new"
done

echo "Copying new service files from $SUPPORT_DIR..."
for file in "$SUPPORT_DIR"/eoi-*.service; do
    if [[ $(basename "$file") == "$IGNORE_SERVICE" ]]; then
        echo "  -> Skipping $(basename "$file") (ignore service)"
        continue
    fi
    echo "  -> Installing $(basename "$file")"
    sudo cp "$file" "$SYSTEMD_DIR/"
done

echo "Reloading systemd daemon..."
sudo systemctl daemon-reload

echo "Enabling and starting new eoi- services..."
for file in "$SUPPORT_DIR"/eoi-*.service; do
    if [[ $(basename "$file") == "$IGNORE_SERVICE" ]]; then
        echo "  -> Skipping $(basename "$file") (ignore service)"
        continue
    fi
    svc=$(basename "$file")
    echo "  -> Enabling $svc"
    sudo systemctl enable "$svc"
    echo "  -> Starting $svc"
    sudo systemctl start "$svc"
done

echo "All eoi- services reinstalled and running."
