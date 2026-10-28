//! User preferences and the locally copied avatar, separate from fetched data.
use std::{fs, io, path::PathBuf, sync::{OnceLock, RwLock}};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings { language: Language }
static LANGUAGE: OnceLock<RwLock<Language>> = OnceLock::new();

fn directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_RESET_SETTINGS_DIR") {
        return Some(path.into());
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/CodexResets"))
}
fn stored_language() -> Language {
    directory().and_then(|dir| fs::read(dir.join("settings.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
        .unwrap_or_default().language
}
pub fn language() -> Language {
    *LANGUAGE.get_or_init(|| RwLock::new(stored_language())).read().unwrap()
}
pub fn set_language(language: Language) -> io::Result<()> {
    let dir = directory().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory unavailable"))?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("settings.json.tmp"), serde_json::to_vec(&Settings { language })?)?;
    fs::rename(dir.join("settings.json.tmp"), dir.join("settings.json"))?;
    *LANGUAGE.get_or_init(|| RwLock::new(language)).write().unwrap() = language;
    Ok(())
}
pub fn avatar_path() -> Option<PathBuf> { Some(directory()?.join("avatar.png")) }
pub fn load_avatar() -> Option<Vec<u8>> { fs::read(avatar_path()?).ok() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_language_defaults_to_english_and_both_languages_round_trip() {
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().language, Language::English);
        for language in [Language::English, Language::SimplifiedChinese] {
            let bytes = serde_json::to_vec(&Settings { language }).unwrap();
            assert_eq!(serde_json::from_slice::<Settings>(&bytes).unwrap().language, language);
        }
        assert!(serde_json::from_str::<Settings>(r#"{"language":"unknown"}"#).is_err());
    }
}
