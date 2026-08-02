#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

set -eu

version="0.23.0"
generator="${ANTLR4_RUST_GEN:-antlr4-rust-gen}"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")
cd "$project_dir"

actual="$($generator --version 2>&1)"
case "$actual" in
  *"$version"*) ;;
  *)
    echo "antlr4-rust-gen $version is required (found: $actual)" >&2
    exit 1
    ;;
esac

mkdir -p src/generated
rm -f src/generated/oon_lexer.rs src/generated/oon_parser.rs \
    src/generated/decisions.json src/generated/semantics.json
"$generator" grammar/Oon.g4 --lib grammar --out-dir src/generated --visitor

