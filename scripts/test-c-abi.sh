#!/bin/sh
# SPDX-License-Identifier: MPL-2.0

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")
cd "$project_dir"

cargo build --release --lib
cc -std=c11 -Wall -Wextra -Werror -Iinclude tests/c_abi.c \
    target/release/liboon.a -lpthread -ldl -lm -o target/c_abi_test
target/c_abi_test

