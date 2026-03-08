use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let i18n_dir = manifest_dir.join("i18n");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let bundle_path = out_dir.join("i18n_bundle.rs");

    println!("cargo:rerun-if-changed={}", i18n_dir.display());

    let locales_path = i18n_dir.join("locales.json");
    let locales_raw = fs::read_to_string(&locales_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", locales_path.display()));
    let locales: Vec<String> = serde_json::from_str(&locales_raw)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", locales_path.display()));

    let mut output = String::new();
    output.push_str("pub static BUNDLE: &[(&str, &str)] = &[\n");
    for locale in locales {
        let rel = format!("i18n/{locale}.json");
        let abs = normalize_for_include(&manifest_dir.join(&rel));
        output.push_str(&format!(
            "    (\"{locale}\", include_str!(r#\"{abs}\"#)),\n"
        ));
    }
    output.push_str("];\n");

    fs::write(&bundle_path, output)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", bundle_path.display()));
}

fn normalize_for_include(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}
