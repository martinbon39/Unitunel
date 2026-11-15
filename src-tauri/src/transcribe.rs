use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub text: String,
}

#[derive(Deserialize)]
struct Verbose {
    #[serde(default)]
    segments: Vec<RawSeg>,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct RawSeg {
    start: f64,
    text: String,
    #[serde(default)]
    no_speech_prob: f64,
    #[serde(default)]
    avg_logprob: f64,
}

/// Whisper's known filler hallucinations on near-silence.
const HALLUCINATIONS: &[&str] = &["amara.org", "sous-titres réalisés", "sous-titrage", "merci d'avoir regardé", "thanks for watching"];

pub struct Job {
    pub key: String,
    pub model: String,
    pub language: String,
    pub prompt: String,
}

pub fn transcribe(job: &Job, wav: Vec<u8>, offset: f64) -> Result<Vec<Segment>, String> {
    if job.key.trim().is_empty() {
        return Err("clé Groq manquante".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400 * attempt));
        }
        let part = reqwest::blocking::multipart::Part::bytes(wav.clone())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("model", job.model.clone())
            .text("response_format", "verbose_json")
            .text("temperature", "0");
        if job.language.len() == 2 {
            form = form.text("language", job.language.clone());
        }
        if !job.prompt.is_empty() {
            form = form.text("prompt", job.prompt.clone());
        }
        let res = client
            .post("https://api.groq.com/openai/v1/audio/transcriptions")
            .bearer_auth(job.key.trim())
            .multipart(form)
            .send();
        match res {
            Ok(r) if r.status().is_success() => {
                let v: Verbose = r.json().map_err(|e| e.to_string())?;
                return Ok(clean(v, offset));
            }
            Ok(r) => {
                let code = r.status();
                last = format!("Groq {code}: {}", r.text().unwrap_or_default());
                if code.as_u16() == 401 || code.as_u16() == 400 {
                    break;
                }
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn clean(v: Verbose, offset: f64) -> Vec<Segment> {
    if v.segments.is_empty() {
        let t = v.text.trim();
        return if t.is_empty() { vec![] } else { vec![Segment { start: offset, text: t.into() }] };
    }
    v.segments
        .into_iter()
        .filter(|s| !(s.no_speech_prob > 0.6 && s.avg_logprob < -0.7))
        .filter(|s| {
            let l = s.text.to_lowercase();
            !HALLUCINATIONS.iter().any(|h| l.contains(h))
        })
        .map(|s| Segment { start: s.start + offset, text: s.text.trim().to_string() })
        .filter(|s| !s.text.is_empty())
        .collect()
}
