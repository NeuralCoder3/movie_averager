#!/bin/bash
# iterate through all files in input (recursively)
# call cargo run --release [input] [output]
# for each file where output is the same bath rust output/ instead of input/...
INPUT_DIR="input"
OUTPUT_DIR="output"
mkdir -p "$OUTPUT_DIR"
find "$INPUT_DIR" -type f | while read -r file; do
    relative_path="${file#$INPUT_DIR/}"
    output_file="$OUTPUT_DIR/${relative_path%.*}.png"
    if [ -f "$output_file" ]; then
        echo "Skipping $file -> $output_file (already exists)"
        continue
    fi
    echo "Processing $file -> $output_file"
    mkdir -p "$(dirname "$output_file")"
    cargo run --release -- "$file" "$output_file"
done