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
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("unitunel").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let mut cfg: Config = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| {
                let cfg = Config::default();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default());
                cfg
            });
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
            let cut = p[..700].rfind(',').unwrap_or(700);
            p.truncate(cut);
            p.push('.');
        }
        p
    }
}
