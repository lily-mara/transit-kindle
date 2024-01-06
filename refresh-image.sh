#!/bin/sh

curl "$1" -o next-image.png

if [ $? -eq 0 ]; then
    mv next-image.png image.png
    date > updated-time

    eips -c
    eips -c

    eips -g image.png
fi
