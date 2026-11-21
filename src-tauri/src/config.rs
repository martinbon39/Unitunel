use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub groq_api_key: String,
    pub language: String,
    pub model: String,
    pub vocabulary: Vec<String>,
    pub auto_paste: bool,
    pub output_dir: Option<PathBuf>,
    pub shortcut: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            groq_api_key: String::new(),
            language: "fr".into(),
            model: "whisper-large-v3".into(),
            vocabulary: vec!["Unitunel".into(), "Claude Code".into()],
            auto_paste: true,
            output_dir: None,
            shortcut: DEFAULT_SHORTCUT.into(),
        }
    }
}

#[cfg(target_os = "macos")]
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_SHORTCUT: &str = "Control+Space";

fn read_file_value() -> Result<serde_json::Value, String> {
    match std::fs::read_to_string(config_path()) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("config.json contient une erreur, corrige-le d'abord ({e})")),
        Err(_) => serde_json::to_value(Config::default()).map_err(|e| e.to_string()),
    }
}

/// The settings as written in the file, without the GROQ_API_KEY fallback that `load` adds.
pub fn read_file() -> Result<Config, String> {
    serde_json::from_value(read_file_value()?).map_err(|e| format!("config.json contient une valeur invalide ({e})"))
}

/// Writes only the given keys, so the rest of the file (unknown keys included) stays as it is.
pub fn save_fields(fields: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let mut v = read_file_value()?;
    let obj = v.as_object_mut().ok_or("config.json n'est pas un objet")?;
    obj.extend(fields);
    serde_json::from_value::<Config>(v.clone()).map_err(|e| format!("Valeur invalide ({e})"))?;
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("unitunel").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
                eprintln!("config.json illisible, valeurs par défaut utilisées : {e}");
                Config::default()
            }),
            Err(_) => {
                let cfg = Config::default();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default());
                cfg
            }
        };
        if cfg.groq_api_key.trim().is_empty() {
            if let Ok(k) = std::env::var("GROQ_API_KEY") {
                cfg.groq_api_key = k;
            }
        }
        cfg
    }

    pub fn tunnels_dir(&self) -> PathBuf {
        self.output_dir.clone().unwrap_or_else(|| {
            dirs::document_dir().unwrap_or_else(std::env::temp_dir).join("Unitunel")
        })
    }

    pub fn whisper_prompt(&self) -> String {
        if self.vocabulary.is_empty() {
            return String::new();
        }
        let mut p = format!("Vocabulaire : {}.", self.vocabulary.join(", "));
        if p.len() > 700 {
            let mut end = 700;
            while !p.is_char_boundary(end) {
                end -= 1;
            }
            let cut = p[..end].rfind(',').unwrap_or(end);
            p.truncate(cut);
            p.push('.');
        }
        p
    }
}
