#!/bin/bash

# go through every png image in output and apply
# magick mogrify -path output -normalize *.jpg
# save the result in contract/... (with the same path)

OUTPUT_DIR="output"
CONTRAST_DIR="contrast"
mkdir -p "$CONTRAST_DIR"
find "$OUTPUT_DIR" -type f -name "*.png" | while read -r file; do
    relative_path="${file#$OUTPUT_DIR/}"
    contrast_file="$CONTRAST_DIR/${relative_path%.*}.png"
    if [ -f "$contrast_file" ]; then
        echo "Skipping $file -> $contrast_file (already exists)"
        continue
    fi
    echo "Processing $file -> $contrast_file"
    mkdir -p "$(dirname "$contrast_file")"
    magick mogrify -path "$(dirname "$contrast_file")" -normalize "$file"
done