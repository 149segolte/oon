// SPDX-License-Identifier: MPL-2.0

use std::fs;

use indexmap::IndexMap;
use oon::{
    Source, Value, compile_schema, evaluate_sources, evaluate_with_initial, parse_json_value,
    parse_overlay,
};

fn source(name: &str, text: &str) -> Source {
    Source {
        name: name.into(),
        text: text.into(),
    }
}

#[test]
fn complete_normative_example() {
    let schema = fs::read_to_string("tests/fixtures/config.sch.oon").unwrap();
    let overlay = fs::read_to_string("tests/fixtures/workstation.oon").unwrap();
    let result = evaluate_sources(
        source("config.sch.oon", &schema),
        vec![source("workstation.oon", &overlay)],
    )
    .unwrap();
    let Value::Object(root) = result else {
        panic!("object root")
    };
    assert_eq!(
        root.keys().map(String::as_str).collect::<Vec<_>>(),
        ["profile", "packages", "labels", "coordinates", "mode"]
    );
    assert_eq!(root["mode"], Value::String("debug".into()));
    assert_eq!(
        root["coordinates"],
        Value::Tuple(vec![Value::Float(41.8781), Value::Float(-87.6298)])
    );
    assert_eq!(
        root["labels"],
        Value::Object(IndexMap::from([
            ("owner".into(), Value::String("Ada".into())),
            ("last-package".into(), Value::String("ruff".into()))
        ]))
    );
}

#[test]
fn canonical_recursion_and_any_rules() {
    assert!(
        compile_schema(source(
            "ok",
            "type tree = { children = list<tree>; }; schema config = tree;"
        ))
        .is_ok()
    );
    assert!(
        compile_schema(source(
            "ok",
            "type tree = { child? = tree; }; schema config = tree;"
        ))
        .is_ok()
    );
    assert!(
        compile_schema(source(
            "bad",
            "type tree = { child = tree; }; schema config = tree;"
        ))
        .is_err()
    );
    assert!(compile_schema(source("bad", "schema config = any;")).is_err());
    assert!(compile_schema(source("ok", "schema config = list<any>;")).is_ok());
}

#[test]
fn unicode_surrogates_and_integer_min() {
    let result = evaluate_sources(source("s", "schema config = { text = string; value = int; };"), vec![source("o", "schema = \"config\"; overlay x = { .text = \"\\uD83D\\uDE00\"; .value = -9223372036854775808; };")]).unwrap();
    let Value::Object(value) = result else {
        panic!()
    };
    assert_eq!(value["text"], Value::String("😀".into()));
    assert_eq!(value["value"], Value::Int(i64::MIN));
}

#[test]
fn multiline_strings_dedent_preserve_boundaries_and_decode_escapes() {
    let schema = r#"
        schema config = {
            empty = string;
            inline = string;
            dedented = string;
            blank-lines = string;
            inline-first = string;
            quotes = string;
            escapes = string;
        };
    "#;
    let overlay = r###"
        schema = """config""";
        overlay strings = {
            .empty = """""";
            .inline = """hello""";
            .dedented = """
                alpha
                  beta
                """;
            .blank-lines = """
                alpha
                    
                omega""";
            .inline-first = """alpha
                beta""";
            .quotes = """He said ""hi"", wrote #, and escaped \"\"\".""";
            .escapes = """line\n\t\uD83D\uDE00""";
        };
    "###;

    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(value) = value else {
        panic!("object root")
    };
    assert_eq!(value["empty"], Value::String(String::new()));
    assert_eq!(value["inline"], Value::String("hello".into()));
    assert_eq!(value["dedented"], Value::String("alpha\n  beta\n".into()));
    assert_eq!(value["blank-lines"], Value::String("alpha\n\nomega".into()));
    assert_eq!(
        value["inline-first"],
        Value::String("alpha\n                beta".into())
    );
    assert_eq!(
        value["quotes"],
        Value::String("He said \"\"hi\"\", wrote #, and escaped \"\"\".".into())
    );
    assert_eq!(value["escapes"], Value::String("line\n\t😀".into()));
}

#[test]
fn multiline_strings_preserve_crlf_and_cr_line_endings() {
    for (line_ending, expected) in [("\r\n", "one\r\n\ttwo\r\n"), ("\r", "one\r\ttwo\r")] {
        let overlay = format!(
            "schema = \"config\"; overlay x = {{ . = \"\"\"{line_ending}\tone{line_ending}\t\ttwo{line_ending}\t\"\"\"; }};"
        );
        let value = evaluate_sources(
            source("schema", "schema config = string;"),
            vec![source("overlay", &overlay)],
        )
        .unwrap();
        assert_eq!(value, Value::String(expected.into()));
    }
}

#[test]
fn multiline_strings_work_as_literal_types_and_report_lexical_errors() {
    let canonical = evaluate_sources(
        source("schema", "schema config = \"\"\"\n  alpha\n  \"\"\";"),
        vec![],
    )
    .unwrap();
    assert_eq!(canonical, Value::String("alpha\n".into()));

    let ordinary_newline = parse_overlay(source(
        "ordinary",
        "schema = \"config\"; overlay x = { . = \"first\nsecond\"; };",
    ));
    assert!(ordinary_newline.is_err());

    for (name, text, message) in [
        (
            "unterminated",
            "schema = \"config\"; overlay x = { . = \"\"\"never closed; };",
            "unterminated multiline string",
        ),
        (
            "unknown",
            "schema = \"config\"; overlay x = { . = \"\"\"\\q\"\"\"; };",
            "unknown string escape",
        ),
        (
            "unicode",
            "schema = \"config\"; overlay x = { . = \"\"\"\\u12\"\"\"; };",
            "incomplete Unicode escape",
        ),
        (
            "surrogate",
            "schema = \"config\"; overlay x = { . = \"\"\"\\uD800\"\"\"; };",
            "high surrogate must be followed by a low surrogate",
        ),
    ] {
        let error = parse_overlay(source(name, text)).unwrap_err();
        assert_eq!(error.diagnostics[0].message, message);
    }
}

#[test]
fn statement_references_use_a_snapshot() {
    let result = evaluate_sources(
        source(
            "s",
            "schema config = { a = int; pair = tuple<int; int;>; };",
        ),
        vec![source(
            "o",
            "schema = \"config\"; overlay x = { .a = 4; .pair = (.a; .a + 1;); };",
        )],
    )
    .unwrap();
    let Value::Object(value) = result else {
        panic!()
    };
    assert_eq!(
        value["pair"],
        Value::Tuple(vec![Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn unselected_branches_are_static_checked_but_not_evaluated() {
    let invalid = evaluate_sources(
        source("s", "schema config = { ok = bool; value = int; };"),
        vec![source(
            "o",
            "schema = \"config\"; overlay x = { if false { .missing = 1; }; };",
        )],
    );
    assert!(invalid.is_err());
    let valid = evaluate_sources(
        source("s", "schema config = { ok = bool; value = int; };"),
        vec![source(
            "o",
            "schema = \"config\"; overlay x = { if false { .value = .missing; }; };",
        )],
    );
    assert!(
        valid.is_err(),
        "statically invalid reference path is rejected"
    );
}

#[test]
fn action_policies_keyed_lists_and_resets() {
    let schema = r#"
        type item = { id = string; count = int; tags = list<string>; };
        schema config = {
            items = list<item> key id;
            nested? = { value = int; };
            tuple = tuple<int; list<string>;>;
        };
    "#;
    let overlay = r#"
        schema = "config";
        overlay x = {
            merge .items = [{ id = "a"; count = 1; tags = ["x";]; };];
            merge .items = [
                { id = "a"; count = 2; tags = ["y";]; };
                { id = "b"; };
            ];
            .nested.value = 9;
            merge .tuple = (3; ["z";];);
            reset .items.1;
            reset .nested;
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    let Value::List(items) = &root["items"] else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    let Value::Object(item) = &items[0] else {
        panic!()
    };
    assert_eq!(item["count"], Value::Int(2));
    assert_eq!(
        item["tags"],
        Value::List(vec![Value::String("x".into()), Value::String("y".into())])
    );
    assert!(!root.contains_key("nested"));
}

#[test]
fn dotted_tuple_shorthand_and_loop_variable_precedence() {
    let schema = "schema config = { labels = map<int>; points = map<tuple<int; int;>>; copied = map<int>; };";
    let overlay = r#"
        schema = "config";
        overlay x = {
            .labels = { first = 10; second = 20; };
            .points = { home.1 = 7; };
            for key in .labels { .copied.key = .labels.key; };
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    let Value::Object(points) = &root["points"] else {
        panic!()
    };
    assert_eq!(
        points["home"],
        Value::Tuple(vec![Value::Int(0), Value::Int(7)])
    );
    let Value::Object(copied) = &root["copied"] else {
        panic!()
    };
    assert_eq!(
        copied.keys().map(String::as_str).collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn quoted_map_keys_preserve_case_and_punctuation_in_keyed_list_overlays() {
    let schema = r#"
        schema config = {
            catalog = {
                entries = list<entry> key id;
            };
        };
        type entry = {
            category = string;
            id = string;
            details = {
                revision = string;
                labels = map<string>;
                flags = map<bool>;
            };
        };
    "#;
    let overlay = r#"
        schema = "config";

        overlay sample = {
          merge .catalog.entries = [
            {
              category = "service";
              id = "worker";
              details = {
                revision = "2";
                labels = {
                  "DISPLAY_LABEL" = "Primary";
                };
                flags = {
                  "feature.preview" = true;
                };
              };
            };
          ];
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    let Value::Object(catalog) = &root["catalog"] else {
        panic!()
    };
    let Value::List(entries) = &catalog["entries"] else {
        panic!()
    };
    let Value::Object(entry) = &entries[0] else {
        panic!()
    };
    let Value::Object(details) = &entry["details"] else {
        panic!()
    };
    let Value::Object(labels) = &details["labels"] else {
        panic!()
    };
    let Value::Object(flags) = &details["flags"] else {
        panic!()
    };
    assert_eq!(labels["DISPLAY_LABEL"], Value::String("Primary".into()));
    assert_eq!(flags["feature.preview"], Value::Bool(true));
}

#[test]
fn quoted_map_keys_work_in_read_and_target_paths() {
    let schema = "schema config = { values = map<string>; copied = string; };";
    let overlay = r#"
        schema = "config";
        overlay quoted = {
            .values."Mixed.key" = "value";
            .values."0" = "numeric key";
            .copied = .values."Mixed.key";
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    let Value::Object(values) = &root["values"] else {
        panic!()
    };
    assert_eq!(values["Mixed.key"], Value::String("value".into()));
    assert_eq!(values["0"], Value::String("numeric key".into()));
    assert_eq!(root["copied"], Value::String("value".into()));
}

#[test]
fn positional_map_reads_expose_ordered_keys_and_values() {
    let schema = r#"
        type item = { count = int; child = { name = string; }; };
        type item-map = map<item>;
        schema config = {
            data = item-map;
            once = map<int>;
            first-key = string;
            middle-key = string;
            last-key = string;
            first-value = int;
            nested-value = string;
            ordinary-value = int;
            indexed-values = list<int>;
            key-selector = string;
            value-selector = int;
            replacement-key = string;
            reinserted-key = string;
        };
    "#;
    let overlay = r#"
        schema = "config";
        overlay x = {
            .data = {
                alpha = { count = 1; child.name = "a"; };
                beta = { count = 2; child.name = "b"; };
                gamma = { count = 3; child.name = "c"; };
            };
            .once = { only = 0; };
            .first-key = .data.0.key;
            .middle-key = .data.1.key;
            .last-key = .data.2.key;
            .first-value = .data.0.value.count;
            .nested-value = .data.2.value.child.name;
            .ordinary-value = .data.beta.count;
            for i in 3 { merge .indexed-values = [.data.i.value.count;]; };
            for key in .once { .key-selector = .data.1.key; };
            for value in .once { .value-selector = .data.1.value.count; };
            .data.beta = { count = 20; };
            .replacement-key = .data.1.key;
            reset .data.alpha;
            .data.alpha = { count = 10; };
            .reinserted-key = .data.2.key;
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    assert_eq!(root["first-key"], Value::String("alpha".into()));
    assert_eq!(root["middle-key"], Value::String("beta".into()));
    assert_eq!(root["last-key"], Value::String("gamma".into()));
    assert_eq!(root["first-value"], Value::Int(1));
    assert_eq!(root["nested-value"], Value::String("c".into()));
    assert_eq!(root["ordinary-value"], Value::Int(2));
    assert_eq!(
        root["indexed-values"],
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(root["key-selector"], Value::String("beta".into()));
    assert_eq!(root["value-selector"], Value::Int(2));
    assert_eq!(root["replacement-key"], Value::String("beta".into()));
    assert_eq!(root["reinserted-key"], Value::String("alpha".into()));
    let Value::Object(data) = &root["data"] else {
        panic!("map")
    };
    assert_eq!(
        data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["beta", "gamma", "alpha"]
    );
}

#[test]
fn positional_map_reads_reject_invalid_selectors_bounds_and_targets() {
    let schema = "schema config = { data = map<int>; result = int; key-result = string; };";
    for (statement, message) in [
        (".result = .data.0;", "map index must be followed"),
        (".result = .data.0.other;", "map index must be followed"),
        (".result = .data.0.\"value\";", "map index must be followed"),
        (".data.0.value = 2;", "positional map access is read-only"),
        (
            "merge .data.0.value = 2;",
            "positional map access is read-only",
        ),
        (
            "set .data.0.value = 2;",
            "positional map access is read-only",
        ),
        ("reset .data.0.value;", "positional map access is read-only"),
        (
            ".data.0.key = \"renamed\";",
            "positional map access is read-only",
        ),
    ] {
        let overlay =
            format!("schema = \"config\"; overlay x = {{ .data = {{ one = 1; }}; {statement} }};");
        let error = evaluate_sources(source("schema", schema), vec![source("overlay", &overlay)])
            .unwrap_err();
        assert!(error.to_string().contains(message), "{statement}: {error}");
    }

    for statement in [".key-result = .data.0.key;", ".result = .data.0.value;"] {
        let overlay = format!("schema = \"config\"; overlay x = {{ {statement} }};");
        let error = evaluate_sources(source("schema", schema), vec![source("overlay", &overlay)])
            .unwrap_err();
        assert!(error.to_string().contains("map index is out of range"));
    }
}

#[test]
fn positional_map_reads_do_not_change_any_or_union_traversal() {
    let schema = r#"
        type keyed = { key = int; };
        schema config = {
            dynamic? = any;
            choice = list<keyed> | map<keyed>;
            dynamic-result = int;
            union-result = int;
        };
    "#;
    let overlay = r#"
        schema = "config";
        overlay x = {
            .dynamic = [{ key = 7; };];
            .choice = [{ key = 8; };];
            .dynamic-result = .dynamic.0.key;
            .union-result = .choice.0.key;
        };
    "#;
    let value =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = value else { panic!() };
    assert_eq!(root["dynamic-result"], Value::Int(7));
    assert_eq!(root["union-result"], Value::Int(8));

    let map_union = evaluate_sources(
        source(
            "schema",
            "schema config = { data = map<int> | map<string>; result = int; };",
        ),
        vec![source(
            "overlay",
            "schema = \"config\"; overlay x = { .result = .data.0.value; };",
        )],
    );
    assert!(
        map_union.is_err(),
        "map positions are unavailable through unions"
    );
}

#[test]
fn unions_preserve_canonical_first_branch_but_reject_supplied_ambiguity() {
    let canonical =
        evaluate_sources(source("schema", "schema config = string | string;"), vec![]).unwrap();
    assert_eq!(canonical, Value::String(String::new()));
    let ambiguous = evaluate_sources(
        source("schema", "schema config = { value = string | string; };"),
        vec![source(
            "overlay",
            "schema = \"config\"; overlay x = { .value = \"x\"; };",
        )],
    );
    assert!(ambiguous.is_err());
}

#[test]
fn tagged_object_shapes_may_refer_forward() {
    let schema = r#"
        schema config = choice;
        type choice = tagged {
            tag = kind;
            common = common-shape;
            variants = { first = payload; };
        };
        type payload = { value = int; };
        type common-shape = { label = string; };
    "#;
    let canonical = evaluate_sources(source("schema", schema), vec![]).unwrap();
    let Value::Object(value) = canonical else {
        panic!()
    };
    assert_eq!(
        value.keys().map(String::as_str).collect::<Vec<_>>(),
        ["kind", "label", "value"]
    );
}

#[test]
fn tagged_unions_are_object_shaped_for_keyed_lists() {
    let schema = r#"
        schema config = {
            packages = list<package> key name;
            by-kind = list<package> key kind;
        };
        type package = tagged {
            tag = kind;
            common = { name = string; };
            variants = {
                brew = {};
                custom = { files = list<string>; };
            };
        };
    "#;
    let overlay = r#"
        schema = "config";
        overlay packages = {
            merge .packages = [{ kind = "brew"; name = "tool"; };];
            merge .packages = [
                { kind = "custom"; name = "tool"; files = ["config";]; };
            ];
            merge .by-kind = [
                { kind = "brew"; name = "formula"; };
                { kind = "custom"; name = "source"; };
            ];
        };
    "#;
    let result =
        evaluate_sources(source("schema", schema), vec![source("overlay", overlay)]).unwrap();
    let Value::Object(root) = result else {
        panic!("object root")
    };
    let Value::List(packages) = &root["packages"] else {
        panic!("packages list")
    };
    assert_eq!(packages.len(), 1);
    let Value::Object(package) = &packages[0] else {
        panic!("package object")
    };
    assert_eq!(package["kind"], Value::String("custom".into()));
    let Value::List(by_kind) = &root["by-kind"] else {
        panic!("by-kind list")
    };
    assert_eq!(by_kind.len(), 2);
}

#[test]
fn tagged_keyed_lists_require_a_valid_identity_in_every_variant() {
    let missing = r#"
        schema config = list<item> key id;
        type item = tagged {
            tag = kind;
            variants = {
                with-id = { id = string; };
                without-id = {};
            };
        };
    "#;
    let error = compile_schema(source("missing", missing)).unwrap_err();
    assert_eq!(
        error.diagnostics[0].message,
        "keyed-list identity `id` is not present in every tagged variant"
    );

    for invalid_field in ["id? = string;", "id = bool;"] {
        let schema = format!(
            r#"
                schema config = list<item> key id;
                type item = tagged {{
                    tag = kind;
                    common = {{ {invalid_field} }};
                    variants = {{ first = {{}}; }};
                }};
            "#
        );
        let error = compile_schema(source("invalid", &schema)).unwrap_err();
        assert_eq!(
            error.diagnostics[0].message,
            "keyed-list identity must be a required string or int field"
        );
    }
}

#[test]
fn lexical_names_keywords_and_recovery_rejection() {
    let valid = r#"
        schema Config = {
            merge = int;
            1-2 = int;
            Profile-2 = string;
            _private = bool;
        };
    "#;
    assert!(compile_schema(source("valid", valid)).is_ok());
    assert!(compile_schema(source("bad", "schema config = { a--b = int; };")).is_err());
    assert!(compile_schema(source("bad", "schema config = { value = int };")).is_err());
    assert!(
        compile_schema(source(
            "bad",
            "schema config = { value = string; }; trailing"
        ))
        .is_err()
    );
    let unknown_escape = evaluate_sources(
        source("schema", "schema config = string;"),
        vec![source(
            "overlay",
            "schema = \"config\"; overlay x = { . = \"\\q\"; };",
        )],
    );
    assert!(unknown_escape.is_err());
    let lone_surrogate = evaluate_sources(
        source("schema", "schema config = string;"),
        vec![source(
            "overlay",
            "schema = \"config\"; overlay x = { . = \"\\uD800\"; };",
        )],
    );
    assert!(lone_surrogate.is_err());
}

#[test]
fn key_may_follow_a_list_alias_and_identity_is_explicit() {
    let schema = r#"
        type item = { id = string; value = int; };
        type items = list<item>;
        schema config = items key id;
    "#;
    assert!(compile_schema(source("schema", schema)).is_ok());
    let missing = evaluate_sources(
        source("schema", schema),
        vec![source(
            "overlay",
            "schema = \"config\"; overlay x = { merge . = [{ value = 1; };]; };",
        )],
    );
    assert!(missing.is_err());
}

#[test]
fn initial_value_is_completed_and_overlays_see_it_without_mutating_it() {
    let schema = compile_schema(source(
        "schema",
        r#"
            type item = { id = string; count = int; };
            type nested = { supplied = int; defaulted = string; };
            schema config = {
                x = int;
                copied = int;
                nested = nested;
                items = list<item> key id;
                mapped = map<nested>;
                pair = tuple<int; int;>;
                optional? = string;
            };
        "#,
    ))
    .unwrap();
    let initial = parse_json_value(
        &schema,
        source(
            "initial.json",
            r#"{
                "x": 10,
                "nested": {"supplied": 3},
                "items": [{"id": "a"}],
                "mapped": {"one": {"supplied": 4}},
                "pair": [8, 9]
            }"#,
        ),
    )
    .unwrap();
    let original = initial.clone();
    let overlay = parse_overlay(source(
        "overlay",
        r#"schema = "config"; overlay test = { .copied = .x; reset .x; };"#,
    ))
    .unwrap();

    let result = evaluate_with_initial(&schema, &initial, &[overlay]).unwrap();
    assert_eq!(initial, original);
    let Value::Object(root) = result else {
        panic!("object root")
    };
    assert_eq!(root["x"], Value::Int(0));
    assert_eq!(root["copied"], Value::Int(10));
    assert!(!root.contains_key("optional"));
    let Value::Object(nested) = &root["nested"] else {
        panic!("nested object")
    };
    assert_eq!(nested["supplied"], Value::Int(3));
    assert_eq!(nested["defaulted"], Value::String(String::new()));
    let Value::List(items) = &root["items"] else {
        panic!("item list")
    };
    let Value::Object(item) = &items[0] else {
        panic!("item object")
    };
    assert_eq!(item["count"], Value::Int(0));
    let Value::Object(mapped) = &root["mapped"] else {
        panic!("map")
    };
    let Value::Object(mapped_item) = &mapped["one"] else {
        panic!("mapped object")
    };
    assert_eq!(mapped_item["defaulted"], Value::String(String::new()));
    assert_eq!(
        root["pair"],
        Value::Tuple(vec![Value::Int(8), Value::Int(9)])
    );
}

#[test]
fn invalid_initial_values_are_rejected_against_the_schema() {
    let schema = compile_schema(source(
        "config.sch.oon",
        r#"
            type keyed = { id = string; };
            type tagged-choice = tagged {
                tag = kind;
                common = {};
                variants = { first = {}; };
            };
            schema config = {
                number = int;
                pair = tuple<int; int;>;
                keyed = list<keyed> key id;
                choice = tagged-choice;
                ambiguous = string | string;
            };
        "#,
    ))
    .unwrap();
    for json in [
        r#"{"number":"bad"}"#,
        r#"{"unknown":1}"#,
        r#"{"number":null}"#,
        r#"{"pair":[1]}"#,
        r#"{"keyed":[{}]}"#,
        r#"{"choice":{}}"#,
        r#"{"ambiguous":"value"}"#,
    ] {
        let error = parse_json_value(&schema, source("initial.json", json)).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.starts_with("config.sch.oon:1:1: schema:"));
        assert!(rendered.contains("initial validation failed:"));
    }

    let native = Value::Object(IndexMap::from([(
        "number".into(),
        Value::String("bad".into()),
    )]));
    let error = evaluate_with_initial(&schema, &native, &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("config.sch.oon:1:1: schema: initial validation failed:")
    );
}

#[test]
fn malformed_json_reports_its_own_source_position() {
    let schema = compile_schema(source("schema", "schema config = { value = int; };")).unwrap();
    let error = parse_json_value(&schema, source("broken.json", "{\n  \"value\": }")).unwrap_err();
    let diagnostic = &error.diagnostics[0];
    assert_eq!(diagnostic.source, "broken.json");
    assert_eq!(diagnostic.line, 2);
    assert_eq!(diagnostic.phase, "JSON");
}
