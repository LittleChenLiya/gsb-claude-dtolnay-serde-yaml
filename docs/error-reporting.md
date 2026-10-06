# Explicit type-source reporting for deserialization errors

When deserialization fails because a YAML node does not match the type
expected by the `Deserialize` implementation, `serde_yaml::Error` records an
explicit *type source* report during the single deserialization pass:

- the **expected type** as described by the `Deserialize` implementation,
- the **kind** of YAML node that was found instead (scalar, sequence,
  mapping, or an explicitly tagged node),
- the **position** of that node in the input,
- and, when the node was reached through a YAML alias, **both** the position
  of the alias reference and the position of the anchored definition.

The input is never deserialized a second time to reconstruct the path or
position, positions that are unknown are left absent (never fabricated as
line 0), and errors produced by the user's own `Deserialize` implementation
keep their original message. Successful results are unaffected.

## API

- `Error::type_source() -> Option<TypeSource>` — the explicit report.
- `TypeSource::expected() -> &str` — e.g. `"i64"`.
- `TypeSource::found() -> NodeKind` — `Scalar`, `Sequence`, `Mapping` or
  `Tagged`.
- `TypeSource::location() -> Option<Location>` — position of the offending
  node (its definition, when reached through an alias).
- `TypeSource::alias() -> Option<AliasLocations>` — `reference()` and
  `definition()` positions when an alias was followed.
- `Error::location() -> Option<Location>` — unchanged; primary position.

## Local cargo commands

```sh
# Run only the type-source acceptance tests.
cargo test --test test_type_source

# Run the pre-existing error message tests (message compatibility).
cargo test --test test_error

# Run the doc tests, including the Error::type_source example.
cargo test --doc
```

## Three error examples

### 1. A field that expects an integer but meets a sequence

```rust
#[derive(Debug, serde::Deserialize)]
struct S {
    x: i64,
}

let err = serde_yaml::from_str::<S>("x: [1, 2]\n").unwrap_err();
```

The error is attributed to the sequence node itself:

```text
x: invalid type: sequence, expected i64 at line 1 column 4
```

`err.type_source()` reports `expected() == "i64"`,
`found() == NodeKind::Sequence`, and `location()` at line 1, column 4.

### 2. An error inside a node reached through an alias

```rust
#[derive(Debug, serde::Deserialize)]
struct S {
    a: Vec<i64>,
    b: i64,
}

let err = serde_yaml::from_str::<S>("a: &x [1, 2]\nb: *x\n").unwrap_err();
```

Both the anchored definition and the alias reference are reported:

```text
b: invalid type: sequence, expected i64 at line 1 column 4 (referenced by alias at line 2 column 4)
```

`err.type_source().unwrap().alias().unwrap()` gives
`definition()` at line 1, column 4 and `reference()` at line 2, column 4.

### 3. Multi-document input: positions do not cross between documents

```rust
let yaml = "---\nx: 1\n---\nx: [1]\n";
let mut de = serde_yaml::Deserializer::from_str(yaml);
let first = de.next().unwrap();   // deserializes fine
let second = de.next().unwrap();  // fails
let err = S::deserialize(second).unwrap_err();
```

The error position belongs to the second document, not the first:

```text
x: invalid type: sequence, expected i64 at line 4 column 4
```
