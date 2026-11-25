use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    embed_icon(&output);

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
    fs::write(output.join("translations.rs"), source).expect("Could not write translations.rs");
}

fn embed_icon(output: &Path) {
    const RESOURCE: &str = "build/player.rc";
    println!("cargo:rerun-if-changed={RESOURCE}");
    println!("cargo:rerun-if-changed=build/player.ico");

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let object = output.join("player-icon.o");
        // A relative input path avoids spaces in the windres preprocessor command.
        let status = Command::new("windres")
            .args(["-J", "rc", "-O", "coff", RESOURCE])
            .arg(&object)
            .status()
            .expect("Could not run windres from MinGW-w64");
        assert!(
            status.success(),
            "windres failed to compile the player icon"
        );
        println!("cargo:rustc-link-arg={}", object.display());
    } else {
        winresource::WindowsResource::new()
            .set_resource_file(RESOURCE)
            .compile()
            .expect("Could not embed the Windows application icon");
    }
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
