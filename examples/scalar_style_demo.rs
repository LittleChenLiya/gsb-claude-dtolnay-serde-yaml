use serde::Deserialize;
use serde_yaml::ScalarStyle;

fn report(label: &str, yaml: &str) {
    match serde_yaml::from_str::<i64>(yaml) {
        Ok(value) => println!("{label:<22} -> Ok({value})"),
        Err(err) => {
            let style = err
                .scalar_style()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "none".to_owned());
            let (line, column) = err
                .location()
                .map(|loc| (loc.line(), loc.column()))
                .unwrap_or((0, 0));
            println!("{label:<22} -> Err(style={style}, line={line}, column={column}): {err}");
        }
    }
}

fn main() {
    report("plain 107", "107");
    report("plain abc", "abc");
    report("double-quoted 107", "\"107\"");
    report("single-quoted 107", "'107'");
    report("literal block 107", "|\n  107\n");
    report("folded block 107", ">\n  107\n");

    // Multi-document: each document's error is diagnosed independently.
    let yaml = "---\nabc\n...\n---\n\"def\"\n";
    for (i, document) in serde_yaml::Deserializer::from_str(yaml).enumerate() {
        match i64::deserialize(document) {
            Ok(value) => println!("doc {} -> Ok({value})", i + 1),
            Err(err) => {
                let loc = err.location().unwrap();
                println!(
                    "doc {} -> Err(style={}, line={}, column={}): {}",
                    i + 1,
                    err.scalar_style()
                        .map_or("none".to_owned(), |s| s.to_string()),
                    loc.line(),
                    loc.column(),
                    err,
                );
            }
        }
    }

    // Temp-file fixture.
    let dir = std::env::temp_dir().join(format!("serde_yaml_demo_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("block.yaml");
    std::fs::write(&path, "|\n  107\n").unwrap();
    let err = serde_yaml::from_reader::<_, i64>(std::fs::File::open(&path).unwrap()).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Block));
    println!(
        "temp file {:?} -> Err(style={}, {})",
        path,
        err.scalar_style().unwrap(),
        err
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
