//! Reading an explicit sequence of three YAML documents, each into its own
//! Rust type, with [`serde_yaml::from_str_documents`].
//!
//! Run with:
//!
//! ```sh
//! cargo run --example three_documents
//! ```
//!
//! The stream below contains three documents: a mapping read into `Person`,
//! a mapping read into `Point`, and an empty document read as YAML `null`.

use serde::de::Deserialize as _;
use serde_derive::Deserialize;
use serde_yaml::Value;

#[derive(Debug, Deserialize)]
struct Person {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Point {
    x: i64,
    y: i64,
}

fn main() -> serde_yaml::Result<()> {
    let input = "\
---
name: Ada Lovelace
---
x: 3
y: 4
---
...
";

    let mut person: Option<Person> = None;
    let mut point: Option<Point> = None;
    let mut empty: Option<Value> = Some(Value::Bool(true));

    serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                person = Some(Person::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                point = Some(Point::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                empty = Some(Value::deserialize(de)?);
                Ok(())
            },
        ],
    )?;

    println!("document 1: {:?}", person.unwrap());
    println!("document 2: {:?}", point.unwrap());
    println!("document 3 (empty -> null): {:?}", empty.unwrap());
    Ok(())
}
