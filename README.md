# OON 1.0

This is a standalone Rust 2024 implementation of Object Overlay Notation 1.0.
It builds an idiomatic Rust library, the `oon` CLI, and a C-compatible static
library (`liboon.a`). Objects and maps retain normalized insertion order all the
way through Serde JSON output.

Strings may use ordinary double quotes or triple quotes. Triple-quoted strings
can span physical lines, remove a shared spaces/tabs indentation prefix, and
support the same escapes as ordinary strings. An immediate line break after the
opening delimiter is omitted; other line endings are preserved.

````oon
.message = """
    first line
      indented line
    """;
````

OON is licensed under the [Mozilla Public License 2.0](LICENSE).

## Build and test

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
sh scripts/test-c-abi.sh
```

The generated lexer, parser, typed listener/visitor, decision manifest, and
semantics manifest are checked in. Normal builds need only Cargo and Rust.

To regenerate them, install the generator pinned to the runtime version:

```sh
cargo install antlr-rust-runtime --version 0.23.0 --locked \
  --features codegen --bin antlr4-rust-gen
sh scripts/regenerate-parser.sh
sh scripts/check-generated.sh
```

## CLI

```sh
oon --schema CONFIG.sch.oon [--initial-value VALUE.json] [OVERLAY.oon ...]
oon --schema CONFIG.sch.oon [--initial-value VALUE.json] --overlays-dir DIRECTORY
```

The first form preserves argument order. The directory form selects top-level
regular `*.oon` files other than `*.sch.oon` and sorts them lexically.
`--initial-value` loads a JSON value, validates and recursively completes it
against the schema, and uses it as the starting configuration. Missing required
object fields receive schema-canonical values; optional fields remain absent.

## Rust API

Use `parse_json_value` for schema-guided JSON decoding, then pass the resulting
`Value` by reference to `evaluate_with_initial`. The evaluator clones the input,
so successful or failed evaluation never changes caller-owned data. A supplied
value changes only the starting configuration: later `reset` and fresh `set`
operations continue to construct canonical values from the schema.

```rust
let schema = oon::compile_schema(schema_source)?;
let initial = oon::parse_json_value(&schema, json_source)?;
let overlay = oon::parse_overlay(overlay_source)?;
let result = oon::evaluate_with_initial(&schema, &initial, &[overlay])?;
```

## C ABI

Consumers include [`include/oon.h`](include/oon.h), link `liboon.a`, and release
every `OonOutput` with `oon_output_free`. The boundary copies inputs, catches
panics, and uses no mutable global state.

`oon_value_from_json_v1` parses JSON into an opaque immutable `OonValue`.
`oon_evaluate_value_v1` borrows that handle, so it can be reused or evaluated
concurrently, and `oon_value_free` releases it after all evaluations finish.
Parsing returns status `0` with an empty `OonOutput`; JSON or OON diagnostics use
status `1`, invalid pointers or UTF-8 use status `2`, and caught panics use
status `3`. The original `oon_evaluate_v1` remains available and unchanged.

```c
OonValue *value = NULL;
OonOutput parsed = oon_value_from_json_v1(&json_source, &value);
if (parsed.status == 0) {
    OonOutput result =
        oon_evaluate_value_v1(&schema_source, value, overlays, overlay_count);
    /* Read result.bytes/result.len before releasing it. */
    oon_output_free(result);
}
oon_output_free(parsed);
oon_value_free(value);
```
