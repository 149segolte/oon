#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

cp "$project_dir/src/generated/oon_lexer.rs" "$temporary/oon_lexer.rs"
cp "$project_dir/src/generated/oon_parser.rs" "$temporary/oon_parser.rs"
cp "$project_dir/src/generated/decisions.json" "$temporary/decisions.json"
cp "$project_dir/src/generated/semantics.json" "$temporary/semantics.json"

sh "$script_dir/regenerate-parser.sh"

cmp "$temporary/oon_lexer.rs" "$project_dir/src/generated/oon_lexer.rs"
cmp "$temporary/oon_parser.rs" "$project_dir/src/generated/oon_parser.rs"
cmp "$temporary/decisions.json" "$project_dir/src/generated/decisions.json"
cmp "$temporary/semantics.json" "$project_dir/src/generated/semantics.json"

