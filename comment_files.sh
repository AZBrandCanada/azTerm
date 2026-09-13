#!/usr/bin/env bash

find . -type f -name "*.rs" | while IFS= read -r file; do
    # Relative file path (e.g., "src/main.rs")
    rel_path="${file#./}"
    
    # Folder-only path for cleanup (e.g., "src")
    rel_dir="$(dirname "$file")"
    rel_dir="${rel_dir#./}"
    [ "$rel_dir" = "." ] && rel_dir="."

    expected_comment="// $rel_path"
    old_folder_comment="// $rel_dir"

    first_line=$(head -n 1 "$file" | tr -d '\r')

    if [ "$first_line" = "$expected_comment" ]; then
        echo "Already correct: $rel_path"
    elif [ "$first_line" = "$old_folder_comment" ]; then
        # Replace the previous folder-only comment on line 1
        tmp_file=$(mktemp)
        printf "%s\n" "$expected_comment" > "$tmp_file"
        tail -n +2 "$file" >> "$tmp_file"
        mv "$tmp_file" "$file"
        echo "Fixed: $rel_path"
    else
        # Prepend to line 1
        tmp_file=$(mktemp)
        printf "%s\n" "$expected_comment" > "$tmp_file"
        cat "$file" >> "$tmp_file"
        mv "$tmp_file" "$file"
        echo "Added: $rel_path"
    fi
done
