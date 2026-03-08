use std::collections::BTreeMap;
use std::env;
use std::sync::OnceLock;

use serde_json::Value;
use unic_langid::LanguageIdentifier;

include!(concat!(env!("OUT_DIR"), "/i18n_bundle.rs"));

static CATALOGS: OnceLock<BTreeMap<&'static str, BTreeMap<String, String>>> = OnceLock::new();

pub fn select_locale(cli_locale: Option<&str>) -> String {
    let supported = supported_locales();

    if let Some(cli) = cli_locale
        && let Some(found) = resolve_locale(cli, &supported)
    {
        return found;
    }

    if let Some(env_loc) = detect_env_locale()
        && let Some(found) = resolve_locale(&env_loc, &supported)
    {
        return found;
    }

    if let Some(sys_loc) = sys_locale::get_locale()
        && let Some(found) = resolve_locale(&sys_loc, &supported)
    {
        return found;
    }

    "en".to_string()
}

pub fn t(locale: &str, key: &str) -> String {
    let catalogs = catalogs();
    if let Some(value) = lookup(catalogs, locale, key) {
        return value.to_string();
    }
    if let Some(value) = lookup(catalogs, "en", key) {
        return value.to_string();
    }
    key.to_string()
}

pub fn tf(locale: &str, key: &str, args: &[(&str, String)]) -> String {
    let mut rendered = t(locale, key);
    for (name, value) in args {
        let token = format!("{{{name}}}");
        rendered = rendered.replace(&token, value);
    }
    rendered
}

fn lookup<'a>(
    catalogs: &'a BTreeMap<&'static str, BTreeMap<String, String>>,
    locale: &str,
    key: &str,
) -> Option<&'a str> {
    if let Some(cat) = catalogs.get(locale)
        && let Some(value) = cat.get(key)
    {
        return Some(value.as_str());
    }
    let base = base_language(locale)?;
    catalogs
        .get(base.as_str())
        .and_then(|cat| cat.get(key))
        .map(String::as_str)
}

fn catalogs() -> &'static BTreeMap<&'static str, BTreeMap<String, String>> {
    CATALOGS.get_or_init(|| {
        let mut catalogs = BTreeMap::new();
        for (locale, raw) in BUNDLE {
            let parsed: Value = serde_json::from_str(raw)
                .unwrap_or_else(|e| panic!("failed to parse embedded locale {locale}: {e}"));
            let obj = parsed
                .as_object()
                .unwrap_or_else(|| panic!("embedded locale {locale} must be a JSON object"));
            let mut flat = BTreeMap::new();
            for (key, value) in obj {
                flat.insert(
                    key.clone(),
                    value
                        .as_str()
                        .unwrap_or_else(|| {
                            panic!("embedded locale {locale} key {key} must map to a string")
                        })
                        .to_string(),
                );
            }
            catalogs.insert(*locale, flat);
        }
        catalogs
    })
}

fn supported_locales() -> Vec<&'static str> {
    BUNDLE.iter().map(|(locale, _)| *locale).collect()
}

fn detect_env_locale() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn resolve_locale(candidate: &str, supported: &[&str]) -> Option<String> {
    let norm = normalize_locale(candidate)?;
    if supported.iter().any(|s| *s == norm) {
        return Some(norm);
    }
    let base = base_language(&norm)?;
    if supported.iter().any(|s| *s == base) {
        return Some(base);
    }
    None
}

fn normalize_locale(raw: &str) -> Option<String> {
    let mut cleaned = raw.trim();
    if cleaned.is_empty() {
        return None;
    }
    if let Some((head, _)) = cleaned.split_once('.') {
        cleaned = head;
    }
    if let Some((head, _)) = cleaned.split_once('@') {
        cleaned = head;
    }
    let cleaned = cleaned.replace('_', "-");
    cleaned
        .parse::<LanguageIdentifier>()
        .ok()
        .map(|lid| lid.to_string())
}

fn base_language(tag: &str) -> Option<String> {
    tag.split('-').next().map(|s| s.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::select_locale;

    #[test]
    fn locale_exact_match_wins() {
        assert_eq!(select_locale(Some("en-GB")), "en-GB");
    }

    #[test]
    fn locale_base_language_falls_back() {
        assert_eq!(select_locale(Some("nl-NL")), "nl");
    }
}
