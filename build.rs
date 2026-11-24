use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
    let english = catalog("en");
    let korean = catalog("ko");
    let missing_ko: Vec<_> = english
        .keys()
        .filter(|key| !korean.contains_key(*key))
        .collect();
    let missing_en: Vec<_> = korean
        .keys()
        .filter(|key| !english.contains_key(*key))
        .collect();
    assert!(
        missing_ko.is_empty() && missing_en.is_empty(),
        "Translation keys missing in ko.json: {missing_ko:?}, in en.json: {missing_en:?}"
    );

    let mut source = String::from("#[derive(Clone, Copy)]\npub enum Text {\n");
    for key in english.keys() {
        writeln!(source, "    {key},").unwrap();
    }
    source.push_str("}\n");
    for (name, translations) in [("ENGLISH", english), ("KOREAN", korean)] {
        writeln!(source, "const {name}: &[&str] = &[").unwrap();
        for value in translations.values() {
            writeln!(source, "    {value:?},").unwrap();
        }
        source.push_str("];\n");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    fs::write(output.join("translations.rs"), source).expect("Could not write translations.rs");
}

fn catalog(locale: &str) -> BTreeMap<String, String> {
    let path = format!("locales/{locale}.json");
    println!("cargo:rerun-if-changed={path}");
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let translations: BTreeMap<String, String> =
        serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{path}: {error}"));
    for (key, value) in &translations {
        assert!(
            key.starts_with(|ch: char| ch.is_ascii_uppercase())
                && key.bytes().all(|ch| ch.is_ascii_alphanumeric())
                && key != "Self",
            "{path}: invalid translation key {key:?}"
        );
        assert!(
            !value.trim().is_empty(),
            "{path}: translation {key} is blank"
        );
    }
    translations
}
