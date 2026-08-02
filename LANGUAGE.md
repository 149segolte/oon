# Object Overlay Notation

Version 1.0

## 1. Overview

Object Overlay Notation (OON) is a schema-driven configuration language. A
schema defines the shape and canonical initial value of an object. Ordered
overlays then transform that value through assignment, merge, set, reset,
conditional, and loop statements.

OON uses its own small grammar instead of encoding operations or references as
another format. The language favors explicit punctuation, deterministic
evaluation, strict typing, and a small number of composable operations.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** describe normative
requirements.

### 1.1 Goals

- Keep schema and overlay syntax compact and mechanically parseable.
- Give every valid schema a deterministic canonical value.
- Make overlay order and mutation behavior explicit.
- Support references and simple expressions without becoming a general-purpose
  programming language.
- Preserve source locations for all syntax, validation, and evaluation errors.

### 1.2 Non-goals

- Custom schema defaults.
- Null values.
- Computed path expressions.
- User-defined functions or an initial built-in function library.
- General-purpose iteration, `break`, `continue`, or mutation of loop variables.
- A prescribed serialization format for the evaluated configuration.
- A CLI, API, parser architecture, or implementation language.

The evaluated configuration is an abstract OON value. JSON, YAML, or another
output representation may be chosen by a future engine specification.

## 2. Documents

OON has separate schema and overlay documents.

### 2.1 Schema documents

A schema document conventionally uses the `.sch.oon` suffix. It contains:

- zero or more top-level `type` declarations; and
- exactly one top-level `schema` declaration.

Declarations are order-independent. A type may refer to a type declared later
in the document.

```oon
type package = {
    name = string;
    version = int;
};

schema config = {
    packages = list<package> key name;
    labels = map<string>;
};
```

### 2.2 Overlay documents

An overlay document conventionally uses the `.oon` suffix. Its first
non-comment declaration MUST be exactly one schema locator:

```oon
schema = "config";
```

The locator currently contains a schema name. Matching is case-insensitive.
The string form is intentionally retained so future specifications can define
file or remote locators without changing the declaration shape.

The locator is followed by zero or more overlay blocks:

```oon
schema = "config";

overlay workstation = {
    .labels.owner = "core";
};
```

An overlay may be empty. Duplicate overlay names are allowed and do not combine
or replace blocks.

Overlay files execute in caller-supplied order. Every overlay block in a file
executes in declaration order.

## 3. Lexical Structure

### 3.1 Source text

Documents are UTF-8 text. Spaces, horizontal tabs, carriage returns, and line
feeds separate tokens except inside strings. Raw line breaks are allowed only
inside triple-quoted multiline strings, not ordinary quoted strings.

### 3.2 Comments

`#` begins a comment anywhere outside a string. A comment ends at the next line
feed or at end-of-file. Inside either ordinary or multiline strings, `#` is
content.

```oon
# Whole-line comment
.profile.name = "Ada"; # Trailing comment
.fragment = "value#part"; # The first # is string content.
```

### 3.3 Names

Schema names, type names, overlay names, fixed-object fields, unquoted dynamic
map keys, tag names, variant names, keyed-list identity fields, and loop
variables use the same name syntax.

A name:

- contains ASCII letters, digits, `_`, and internal `-` characters;
- begins and ends with a letter, digit, or `_`;
- has no consecutive `-` characters; and
- is not composed entirely of digits.

Examples of valid names:

```text
profile
Profile-2
2nd-profile
_private
1-2
```

Names compare case-insensitively and normalize to lowercase. Declarations that
differ only in case collide and MUST be rejected. Unquoted dynamic map keys
also normalize to lowercase. Quoted dynamic map keys follow Section 6.2.

Because `-` may occur inside a name, the lexer uses longest-match tokenization.
Whitespace MUST separate binary subtraction where adjacent tokens could
otherwise form one name:

```oon
.difference = .left - .right;
```

### 3.4 Keywords

The following lowercase spellings have grammatical meaning:

```text
and       bool      common    else      false
float     for       if        in        int
key       list      map       merge     not
overlay   reset     schema    set       string
tag       tagged    true      tuple     type
variants  any
```

Keywords are lowercase-only and contextual. They MAY be used as names where
the grammar is unambiguous, such as after a path separator. Uppercase or
mixed-case spellings are names, not keywords.

The built-in type names `string`, `int`, `float`, `bool`, and `any` are reserved
in the top-level schema/type namespace and cannot be redeclared.

### 3.5 Strings

Strings are Unicode values and have ordinary and multiline forms. Ordinary
strings use double quotes and MUST remain on one physical source line:

```oon
.message = "one line";
```

Multiline strings use triple double quotes and may contain raw line breaks,
single unescaped `"` characters, and unescaped pairs of `"` characters:

````oon
.message = """
    first line
      indented line
    """;
````

The multiline body is normalized before escapes are decoded:

1. An immediate opening line break (`LF`, `CRLF`, or `CR`) is removed.
2. The longest exact prefix of spaces and horizontal tabs shared by all
   nonblank physical lines is removed from those lines. Spaces and tabs compare
   as distinct characters; tab display widths are not considered. Whitespace
   on blank-only lines is removed, but their line endings are retained.
3. All remaining characters and line endings are preserved exactly.

Consequently, placing the closing `"""` on its own line retains the line break
before it, while placing it immediately after the last content character does
not add one. An inline first line has no leading whitespace, so it prevents
dedenting of subsequent lines.

Both forms support the same escapes:

| Escape    | Value              |
|-----------|--------------------|
| `\\`      | Backslash          |
| `\"`      | Double quote       |
| `\n`      | Line feed          |
| `\r`      | Carriage return    |
| `\t`      | Horizontal tab     |
| `\uXXXX`  | Unicode code point |

`XXXX` is exactly four hexadecimal digits. Surrogate pairs use two consecutive
`\uXXXX` escapes. Unknown or incomplete escapes are errors. Escape decoding for
a multiline string happens after physical-line dedenting, so `\n` does not
create a line that participates in indentation calculation. To include the
triple-quote delimiter as content, write `\"\"\"`.

### 3.6 Numbers

OON has two numeric representations:

- `int`: signed 64-bit integers.
- `float`: finite IEEE-754 binary64 values.

Integer literals contain decimal digits. Float literals contain decimal digits,
a `.`, and decimal digits on both sides of the point. Leading zeroes are
allowed and have no octal meaning. Exponent, hexadecimal, octal, suffix, `NaN`,
and infinity syntax is not supported.

```oon
.count = 12;
.ratio = 0.25;
.negative = -3;
```

The leading `-` is the unary negation operator. Literal evaluation and every
integer operation MUST remain within the signed 64-bit range. Floating
operations that produce a non-finite result are errors.

### 3.7 Punctuation and terminators

`=` is mandatory for declarations, assignments, and named fields. `;`
terminates every declaration, action, object field, list item, tuple item,
conditional chain, and loop.

Commas are not part of OON.

## 4. Schema Language

### 4.1 Built-in types

OON provides these primitive types:

| Type     | Accepted runtime value |
|----------|------------------------|
| `string` | Unicode string         |
| `int`    | Signed 64-bit integer  |
| `float`  | Finite binary64 value  |
| `bool`   | Boolean                |
| `any`    | Any OON value          |

`bool` and numeric types are distinct. In particular, booleans are not
integers.

### 4.2 Literal types

A string, integer, float, or boolean literal may appear as a type. A literal
type accepts only that exact value.

```oon
type mode = "debug" | "release";
type retry-count = 0 | 1 | 2 | 3;
```

Literal unions provide enum-like constraints without a separate enum type.

### 4.3 Fixed objects

A fixed object declares a known set of fields:

```oon
type profile = {
    name = string;
    active = bool;
    nickname? = string;
};
```

The `?` suffix marks a field optional. Optionality applies only to named fields
of fixed objects. An optional field may be absent; it does not permit a null
value.

Unknown fields are invalid.

### 4.4 Maps

`map<T>` is an insertion-ordered mapping from normalized OON names to values of
type `T`.

```oon
labels = map<string>;
```

Map keys outside the OON name grammar are not supported in Version 1.0.

### 4.5 Lists

`list<T>` is an ordered, variable-length sequence of values of type `T`.

```oon
ports = list<int>;
```

A list may declare an identity field:

```oon
packages = list<package> key name;
```

For a keyed list:

- the item type MUST resolve to an object-shaped type;
- the identity field MUST be a required `string` or `int` field;
- every supplied item MUST explicitly contain its identity, even if other
  required fields may receive canonical values; and
- identities MUST be unique within the complete list and within each incoming
  list.

### 4.6 Tuples

`tuple<T; U; ...>` is a fixed-length, positionally typed sequence. Tuple member
types are terminated with semicolons.

```oon
coordinates = tuple<float; float;>;
metadata = tuple<string; int; bool;>;
empty = tuple<>;
```

A tuple value must always contain exactly the declared number of elements.

### 4.7 Ordinary unions

`|` forms an ordered union:

```oon
type identifier = string | int;
type mode = "debug" | "release";
```

A supplied value MUST match exactly one branch. Matching no branch is a type
error; matching multiple branches is an ambiguity error. Use a tagged union
when branch shapes overlap.

Branch order determines the canonical branch but never resolves ambiguity
during value validation.

### 4.8 Tagged unions

A tagged union is an object selected by a required discriminator field:

```oon
type source-common = {
    label = string;
};

type file-source = {
    path = string;
};

type source = tagged {
    tag = kind;
    common = source-common;

    variants = {
        file = file-source;
        service = {
            port = int;
        };
        disabled = {};
    };
};
```

Rules:

- `tag` is required and names the discriminator field.
- `variants` is required and MUST contain at least one variant.
- `common` is optional.
- `common` and variant payloads MUST be inline fixed objects or named types
  resolving to fixed objects.
- The discriminator is automatically added as a required string field.
- A variant name supplies its discriminator value.
- Discriminator matching is case-insensitive and the stored value normalizes
  to lowercase.
- A payload MUST NOT redeclare the discriminator or a common field.
- Every incoming tagged value MUST explicitly supply the discriminator,
  including values used by merge statements.
- An empty payload creates a tag-only variant.

### 4.9 Named types and recursion

Any type expression may be named:

```oon
type port = int;
type names = list<string>;
type tree = {
    value = string;
    children = list<tree>;
};
```

The schema name and all named types share one case-insensitive namespace.

Recursive references are valid only when canonical construction terminates.
Recursion behind an absent optional field, an empty list or map, or a
non-canonical union branch does not create a canonical dependency.

```oon
# Valid: children begins as an empty list.
type tree = {
    children = list<tree>;
};

# Invalid: constructing required child never terminates.
type broken = {
    child = broken;
};
```

### 4.10 Root schemas

The root schema may use any canonicalizable type:

```oon
schema config = {
    profile = profile;
};
```

An `any` root is invalid because `any` has no canonical value.

## 5. Canonical Values

Evaluation begins by constructing the root schema's canonical value.

| Type                 | Canonical value                                      |
|----------------------|------------------------------------------------------|
| `string`             | `""`                                                 |
| `int`                | `0`                                                  |
| `float`              | `0.0`                                                |
| `bool`               | `false`                                              |
| literal              | The literal itself                                   |
| fixed object         | Required fields canonicalized; optional fields absent |
| map                  | `{}`                                                 |
| list                 | `[]`                                                 |
| tuple                | Each position canonicalized                          |
| ordinary union       | Canonical value of the first branch                  |
| tagged union         | First variant, automatic tag, common fields, payload |
| `any`                | None; `any` has no canonical value                   |

OON has no null value.

`any` may occur only where canonical construction cannot reach it, including
behind an absent optional field, an empty list or map, or a non-canonical union
branch. This rule applies after resolving named aliases, rather than only to
the immediate syntax.

The dependency graph followed by canonical construction MUST be finite and
acyclic.

Required means structurally present. A required string may validly remain `""`,
an integer may remain `0`, and a boolean may remain `false`. OON does not track
whether an overlay explicitly assigned a required field.

## 6. Overlay Values and Paths

### 6.1 Absolute paths

All configuration targets and references are absolute paths beginning with
`.`:

```oon
.profile.name
.packages.0.name
.
```

`.` is the complete root path. Remaining segments are separated by `.`.
Numeric-only segments are zero-based collection indexes; all other segments
are normalized names. For lists and tuples, an index selects the corresponding
item. In a reference path whose current statically resolved type is `map<T>`,
an index selects an entry in stored insertion order and MUST be followed
immediately by `key` or `value`:

```oon
.labels.2.key
.labels.2.value
```

The `key` projection has type `string`; the `value` projection has type `T` and
may be traversed further. The indexed entry itself is not a value, so a path
ending at the index is invalid. A map index is valid only when the current type
resolves directly, including through a type alias, to `map<T>`; it is not
available through `any` or a union. Positional map paths are references only and
MUST NOT be used as assignment, `merge`, `set`, or `reset` targets. Ordinary
keyed access is unchanged, so `.labels.key` selects the map entry named `key`.

Schema and runtime container types determine whether each segment is valid.
Out-of-range list, tuple, and positional map indexes are errors. A missing
referenced field or map key is an error.

Grammar position distinguishes targets from references:

```oon
.profile.name = .defaults.name;
```

The left path is a target and the right path is a reference.

### 6.2 Collection literals

Object and map values use braces:

```oon
{
    name = "Ada";
    enabled = true;
}
```

A dynamic map key may instead be a quoted string:

```oon
{
    "DISPLAY_LABEL" = "Primary";
    "feature.preview" = true;
}
```

Quoted keys preserve the decoded string exactly, including case and
punctuation. A quoted key is one path segment, so a `.` inside it is not a path
separator. Quoted segments may be used anywhere a path traverses a map, for
example `.options."feature.preview"`. They do not interpolate loop
variables, and a quoted numeric key selects a map key rather than a positional
map entry. The special positional-map projections `key` and `value` remain
unquoted names. Quoted segments MUST NOT select fixed-object or tagged-object
fields or index lists and tuples. They may traverse a value typed as `any`,
where runtime container checks still apply.

Lists use brackets, with every item terminated by `;`:

```oon
[
    "one";
    "two";
]
```

Tuples use parentheses:

```oon
("origin"; 0; true;)
```

`(expression)` is grouping. `(expression;)` is a one-element tuple. `()` is an
empty tuple.

### 6.3 Dotted object shorthand

Runtime object and map literals may use relative dotted fields:

```oon
{
    git.enabled = true;
    labels.owner = "core";
    coordinates.1 = 42.0;
}
```

Shorthand may traverse fixed objects, maps, and tuples. It MUST NOT traverse a
list. An absent optional intermediate object or map is materialized from its
canonical value. Tuple indexes must already be valid.

Within one literal, no field entry may duplicate, contain, or be contained by
another entry. This is invalid:

```oon
{
    git = { enabled = true; };
    git.enabled = false;
}
```

Dotted shorthand applies only to runtime values, not schema object
declarations.

## 7. Actions

An overlay body contains statements. There are four action forms:

```oon
.target = expression;
merge .target = expression;
set .target = expression;
reset .target;
```

Action keywords occur only in statement position. Their policies apply
recursively through the complete supplied value.

All supplied values are deep-copied. OON never creates mutable aliases between
configuration paths.

### 7.1 Hybrid assignment

Bare assignment uses hybrid behavior recursively:

- primitives and literal values overwrite;
- `any` behaves as a primitive and overwrites using its supplied runtime value;
- lists replace;
- fixed objects and maps merge by field/key;
- tuples combine by position, applying hybrid behavior to each element; and
- same-branch tagged values combine recursively.

If an incoming ordinary or tagged union value selects a different branch, the
target is replaced from the new branch's canonical value.

### 7.2 Explicit merge

`merge` uses additive behavior recursively:

- primitive and literal values overwrite;
- `any` behaves as a primitive and overwrites using its supplied runtime value;
- fixed objects and maps merge by field/key;
- ordinary lists append;
- keyed lists merge matching identities recursively, retain unmatched existing
  items, and append new identities;
- tuples merge corresponding positions; and
- same-branch tagged values merge recursively.

Changing an ordinary or tagged union branch replaces the old branch.

An absent optional container is materialized canonically before merging.
Merging a supplied value into an absent optional `any` field establishes the
supplied runtime type and value.

### 7.3 Set

`set` resets the target and then populates a fresh canonical value:

```oon
set .settings = {
    git.enabled = true;
};
```

No prior nested state survives. Omitted required fields retain canonical
values. Omitted optional fields remain absent. Tuple values must still have
exact arity.

### 7.4 Reset

`reset` restores the target to its canonical state within its parent:

- a required fixed field regains its canonical value;
- an optional fixed field becomes absent;
- a map entry becomes absent;
- a list element is deleted and later indexes shift left;
- a tuple element regains its canonical value;
- a required list or map becomes empty;
- an optional list or map becomes absent; and
- the root regains the complete root canonical value.

Resetting an already-absent optional field or map entry is a no-op.
Out-of-range list or tuple indexes remain errors.

Wildcard reset syntax does not exist.

### 7.5 Root actions

Every action accepts the root path:

```oon
. = { profile.name = "Ada"; };
merge . = .saved-config;
set . = { labels.owner = "core"; };
reset .;
```

On the right-hand side, `.` references the complete current configuration.

## 8. Expressions

Expressions may appear everywhere a runtime value is expected, including
object fields, collection items, action right-hand sides, conditions, and loop
iterables. Paths themselves are not general computed expressions.

### 8.1 Precedence

From highest to lowest:

1. grouping, literals, references, and loop variables;
2. unary `not` and unary `-`;
3. `*` and `/`;
4. `+` and binary `-`;
5. `<`, `>`, `<=`, and `>=`;
6. `==`;
7. `and`;
8. `or`.

Binary operators of the same precedence associate left-to-right. Chained
comparisons are not a special construct and will ordinarily fail type checking
after the first comparison returns `bool`.

### 8.2 Arithmetic

`+`, `-`, `*`, and `/` accept numbers. Same-type arithmetic preserves the
numeric type. Mixed `int` and `float` arithmetic widens the integer operand and
produces a float.

`+` also concatenates two strings. Strings cannot be mixed with other operand
types.

Integer division truncates toward zero. Division by zero, signed integer
overflow, and non-finite float results are errors.

There is no remainder operator.

### 8.3 Comparison and equality

`<`, `>`, `<=`, and `>=` accept same-typed numeric operands only. Mixed
`int`/`float` comparison is a type error. Strings and booleans are not ordered.

`==` accepts same-typed numbers, booleans, or strings. Compound-value equality
and `!=` are not supported.

### 8.4 Logical operators

`and`, `or`, and `not` accept booleans and numbers. Boolean false and numeric
zero are false; boolean true and every nonzero number are true. `and` and `or`
may mix boolean and numeric operands, short-circuit from left to right, and
always return a boolean. `not` also returns a boolean.

### 8.5 Assignment typing

Every expression result MUST validate against its schema destination. Mixed
numeric arithmetic may produce a float, but a float cannot be assigned to an
`int` destination. OON performs no implicit float-to-integer rounding.

If a value typed as `any` flows into a concrete destination or operator, its
actual value is checked at runtime.

Function calls and interpolation are not part of Version 1.0.

## 9. Reference Evaluation

All references within a statement resolve against the configuration state
immediately before that statement begins. A statement cannot observe writes
made by itself.

References return deep copies. Later mutation of the source or destination
cannot affect the other path.

A missing reference is a source-located evaluation error. References in
unselected conditional branches are not evaluated.

## 10. Conditionals

Conditionals are statements with C-style blocks:

```oon
if .enabled {
    .mode = "active";
} else if .retries > 0 {
    .mode = "retry";
} else {
    reset .mode;
};
```

Parentheses around a condition are optional for every expression and may be
used for grouping:

```oon
if (.enabled and .retries > 0) {
    .mode = "retry";
};
```

Conditions accept booleans or numbers using the logical truth rules. Conditions
are evaluated in order against the current configuration. Exactly the first
matching branch executes; later conditions and unselected branch bodies are
not evaluated.

All branch syntax, paths, actions, and statically knowable types are validated
even when a branch is not selected. Missing runtime values matter only if their
expressions are evaluated.

Branch bodies may contain actions, nested conditionals, and loops. Actions
execute sequentially and observe prior completed statements.

The complete chain ends with exactly one `;`. An `if` without `else` also ends
with `;`. There are no semicolons between connected branches.

## 11. Loops

OON provides one loop form:

```oon
for i in .packages {
    merge .selected = .packages.i;
};

for key in .labels {
    .copied.key = .labels.key;
};

for i in 5 {
    merge .indexes = [i;];
};
```

The iterable expression MUST statically resolve to exactly one of:

- `int`: iterate from `0` through `n - 1`;
- `list<T>`: iterate the list's zero-based indexes; or
- `map<T>`: iterate normalized keys in stored insertion order.

Zero produces no iterations. A negative integer is an error. `any` and unions
spanning different iteration categories cannot be loop iterables.

For integer and list loops, the variable is an `int`. For map loops, it is a
`string`. The variable is immutable and is an expression within the loop body.

When a path segment exactly matches an in-scope loop variable, the variable's
index or key is substituted:

```oon
.packages.i
.labels.key
```

Variable substitution takes precedence over interpreting the segment as a
literal field or key. The same substitution rule applies to target paths,
reference paths, and relative dotted shorthand. Integer loop variables may be
used as positional map indexes. The `key` and `value` segments immediately
following a positional map index are projections and take precedence over
same-named loop variables.

The iterable is evaluated once at loop entry. Integer/list index sequences and
map key sequences are snapshotted. Configuration reads remain live; body
mutations can therefore make a later snapshotted path invalid.

Nested loops and conditionals are allowed, but simultaneously active loop
variables MUST have distinct case-insensitive names. OON has no `break`,
`continue`, alternate loop form, or assignment to a loop variable.

Every loop ends with one `;`.

## 12. Evaluation and Validation

Evaluation proceeds as follows:

1. Parse and resolve the selected schema document.
2. Reject name collisions, invalid type references, invalid identity fields,
   union/schema errors, and non-terminating canonical dependencies.
3. Construct the root canonical value.
4. Parse overlay files in caller-supplied order and verify their schema
   locators.
5. Validate all statically knowable paths, expressions, actions, branches, and
   loop bodies.
6. Execute every overlay block and statement sequentially.
7. Validate the final value against the root schema.

Each action must leave the affected value valid for its schema. Required
omissions are filled canonically where the action semantics call for fresh
construction. Unknown fixed fields, invalid collection items, duplicate keyed
identities, invalid tagged values, incompatible runtime `any` values, missing
references, invalid indexes, arithmetic faults, and expression type errors
fail evaluation.

Errors MUST identify the source document and line. Implementations SHOULD also
include the column, declaration or overlay name, relevant path, and a concise
explanation. Evaluation failure produces no successful configuration value;
partial state is not an output of the language.

## 13. Grammar Summary

The grammar below is normative in structure. Lexical productions such as
`NAME`, `STRING`, `INT`, and `FLOAT` follow Section 3. `EOF` means end-of-file.

```ebnf
STRING             = ordinary-string | multiline-string ;
ordinary-string    = '"', { escaped-character | ordinary-character }, '"' ;
multiline-string   = '"""', { escaped-character | multiline-character },
                     '"""' ;

schema-document    = { type-declaration }, schema-declaration,
                     { type-declaration }, EOF ;
type-declaration   = "type", NAME, "=", type-expression, ";" ;
schema-declaration = "schema", NAME, "=", type-expression, ";" ;

overlay-document   = schema-locator, { overlay-declaration }, EOF ;
schema-locator     = "schema", "=", STRING, ";" ;
overlay-declaration
                   = "overlay", NAME, "=", block, ";" ;

type-expression    = union-type ;
union-type         = keyed-type, { "|", keyed-type } ;
keyed-type         = primary-type, [ "key", NAME ] ;
primary-type       = primitive-type
                   | literal-type
                   | NAME
                   | object-type
                   | list-type
                   | map-type
                   | tuple-type
                   | tagged-type
                   | "(", type-expression, ")" ;
primitive-type     = "string" | "int" | "float" | "bool" | "any" ;
literal-type       = STRING | signed-number | "true" | "false" ;
signed-number      = [ "-" ], ( INT | FLOAT ) ;

object-type        = "{", { field-declaration }, "}" ;
field-declaration  = NAME, [ "?" ], "=", type-expression, ";" ;
list-type          = "list", "<", type-expression, ">" ;
map-type           = "map", "<", type-expression, ">" ;
tuple-type         = "tuple", "<", { type-expression, ";" }, ">" ;

tagged-type        = "tagged", "{",
                       "tag", "=", NAME, ";",
                       [ "common", "=", object-shape, ";" ],
                       "variants", "=", variant-block, ";",
                     "}" ;
object-shape       = object-type | NAME ;
variant-block      = "{", variant-declaration,
                     { variant-declaration }, "}" ;
variant-declaration
                   = NAME, "=", object-shape, ";" ;

block              = "{", { statement }, "}" ;
statement          = action
                   | conditional
                   | loop ;

action             = path, "=", expression, ";"
                   | "merge", path, "=", expression, ";"
                   | "set", path, "=", expression, ";"
                   | "reset", path, ";" ;

conditional        = "if", expression, block,
                     { "else", "if", expression, block },
                     [ "else", block ], ";" ;
loop               = "for", NAME, "in", expression, block, ";" ;

path               = ".", [ path-segment, { ".", path-segment } ] ;
path-segment       = NAME | INDEX | STRING ;

expression         = or-expression ;
or-expression      = and-expression, { "or", and-expression } ;
and-expression     = equality-expression, { "and", equality-expression } ;
equality-expression
                   = comparison-expression, { "==", comparison-expression } ;
comparison-expression
                   = additive-expression,
                     { ( "<" | ">" | "<=" | ">=" ), additive-expression } ;
additive-expression
                   = multiplicative-expression,
                     { ( "+" | "-" ), multiplicative-expression } ;
multiplicative-expression
                   = unary-expression,
                     { ( "*" | "/" ), unary-expression } ;
unary-expression   = ( "not" | "-" ), unary-expression
                   | primary-expression ;
primary-expression = STRING | INT | FLOAT | "true" | "false"
                   | path
                   | NAME
                   | object-literal
                   | list-literal
                   | tuple-literal
                   | "(", expression, ")" ;

object-literal     = "{", { relative-assignment }, "}" ;
relative-assignment
                   = relative-path, "=", expression, ";" ;
relative-path      = ( NAME | STRING ), { ".", path-segment } ;
list-literal       = "[", { expression, ";" }, "]" ;
tuple-literal      = "(", { expression, ";" }, ")" ;
```

`key NAME` is valid only when the preceding primary type resolves to a list.
Bare `NAME` expressions are valid only when they resolve to in-scope loop
variables. Semantic restrictions described in earlier sections apply in
addition to this context-free grammar.

## 14. Complete Example

Schema file `config.sch.oon`:

```oon
# Declarations may refer forward.
schema config = {
    profile = profile;
    packages = list<package> key name;
    labels = map<string>;
    source? = source;
    coordinates = tuple<float; float;>;
    mode = "debug" | "release";
};

type profile = {
    name = string;
    active = bool;
    nickname? = string;
};

type package = {
    name = string;
    version = int;
    metadata = map<any>;
};

type source = tagged {
    tag = kind;
    common = {
        label = string;
    };
    variants = {
        file = {
            path = string;
        };
        service = {
            port = int;
        };
        disabled = {};
    };
};
```

Overlay file `workstation.oon`:

```oon
schema = "Config";

overlay defaults = {
    set .profile = {
        name = "Ada";
        active = true;
    };

    .coordinates = (41.8781; -87.6298;);
    .mode = "debug";
};

overlay workstation = {
    merge .packages = [
        {
            name = "ruff";
            version = 1;
            metadata.channel = "stable";
        };
    ];

    if .profile.active and .packages.0.version > 0 {
        .labels.owner = .profile.name;
    } else {
        reset .labels;
    };

    for i in .packages {
        merge .labels = {
            last-package = .packages.i.name;
        };
    };
};
```

The schema's canonical configuration exists before `defaults` runs. Required
strings begin as `""`, booleans as `false`, collections empty, tuples
positionally canonicalized, and optional fields absent. The two overlay blocks
then execute in their declaration order.
