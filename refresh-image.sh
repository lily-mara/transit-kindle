#!/bin/sh

curl "$1/black.png?target=kindle" -o black.png
curl "$1/stops.png?target=kindle" -o next-image.png

if [ $? -eq 0 ]; then
    mv next-image.png image.png

    eips -g black.png
    eips -g black.png

    eips -g image.png
fi
