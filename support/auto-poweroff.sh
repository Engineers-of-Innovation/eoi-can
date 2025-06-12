#!/bin/bash

seconds_till_shutdown=60  # 1 minutes in seconds

seconds_remaining=$seconds_till_shutdown
while true; do
    echo "get battery_power_plugged" | nc -q 0 127.0.0.1 8423 | grep -q "battery_power_plugged: false"
    if [ $? -eq 0 ]; then
        echo "Warn: Power is not plugged in, waiting for $seconds_remaining seconds before shutdown..."
        seconds_remaining=$((seconds_remaining - 1))
        if [ $seconds_remaining -le 0 ]; then
            echo "Error: Power is not plugged in for $seconds_till_shutdown seconds, shutting down..."
            poweroff
        fi
    else
        echo "Info: Power is plugged in"
        seconds_remaining=$seconds_till_shutdown
    fi
    sleep 1
done