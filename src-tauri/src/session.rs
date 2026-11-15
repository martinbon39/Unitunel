use crate::audio::{self, Recorder, RATE};
use crate::capture::{self, Screen};
use crate::config::Config;
use crate::platform::{self, Focus};
use crate::transcribe::{self, Job, Segment};
use chrono::{DateTime, Local};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CHUNK_MAX: usize = RATE * 24;
const CHUNK_SEARCH: usize = RATE * 6;

pub enum Kind {
    Start,
    Click { app: String, title: String, element: Option<String>, zoom: String },
    Drawing,
    Board,
}

pub struct Event {
    pub t: f64,
    pub n: usize,
    pub kind: Kind,
    pub img: String,
}

pub struct Session {
    pub dir: PathBuf,
    pub t0: Instant,
    pub started: DateTime<Local>,
    pub screen: Screen,
    pub focus: Focus,
    pub rec: Recorder,
    consumed: usize,
    jobs: Vec<JoinHandle<Result<Vec<Segment>, String>>>,
    job: Arc<Job>,
    pub events: Arc<Mutex<Vec<Event>>>,
    counter: Arc<AtomicUsize>,
    pending: Arc<AtomicUsize>,
}

impl Session {
    pub fn start(cfg: &Config, screen: Screen) -> Result<Self, String> {
        let focus = platform::foreground();
        let rec = Recorder::start()?;
        let started = Local::now();
        let dir = cfg.tunnels_dir().join(started.format("%Y-%m-%d_%H-%M-%S").to_string());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let s = Self {
            dir,
            t0: Instant::now(),
            started,
            screen,
            focus,
            rec,
            consumed: 0,
            jobs: vec![],
            job: Arc::new(Job {
                key: cfg.groq_api_key.clone(),
                model: cfg.model.clone(),
                language: cfg.language.clone(),
                prompt: cfg.whisper_prompt(),
            }),
            events: Arc::new(Mutex::new(vec![])),
            counter: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(AtomicUsize::new(0)),
        };
        s.snapshot(None, |_, _| {}, || Kind::Start);
        Ok(s)
    }

    pub fn elapsed(&self) -> f64 {
        self.t0.elapsed().as_secs_f64()
    }

    /// Sends finished stretches of speech to Whisper while recording, so stopping only waits for the tail.
    pub fn pump(&mut self, last: bool) {
        loop {
            let (chunk, offset) = {
                let buf = self.rec.samples.lock();
                let avail = buf.len() - self.consumed;
                if avail == 0 || (!last && avail < CHUNK_MAX) {
                    return;
                }
                let end = if avail <= CHUNK_MAX {
                    buf.len()
                } else {
                    let to = self.consumed + CHUNK_MAX;
                    audio::quietest_cut(&buf, to - CHUNK_SEARCH, to)
                };
                let chunk = buf[self.consumed..end].to_vec();
                let offset = self.consumed as f64 / RATE as f64;
                self.consumed = end;
                (chunk, offset)
            };
            if chunk.len() < RATE / 4 || audio::peak_rms(&chunk) < 0.004 {
                continue;
            }
            let job = self.job.clone();
            self.jobs.push(std::thread::spawn(move || transcribe::transcribe(&job, audio::wav_bytes(&chunk), offset)));
        }
    }

    /// Captures the screen in the background and records it as an event at the current time.
    pub fn snapshot<F, K>(&self, at: Option<(i32, i32)>, decorate: F, kind: K)
    where
        F: FnOnce(&mut image::RgbaImage, f32) + Send + 'static,
        K: FnOnce() -> Kind + Send + 'static,
    {
        let t = self.elapsed();
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let (screen, dir, events, pending) = (self.screen, self.dir.clone(), self.events.clone(), self.pending.clone());
        pending.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let kind = kind();
            let r = (|| {
                let (mut img, k) = capture::grab(&screen)?;
                let stem = match &kind {
                    Kind::Start => "00-debut".to_string(),
                    Kind::Click { .. } => format!("{n:02}-clic"),
                    Kind::Drawing => format!("{n:02}-dessin"),
                    Kind::Board => unreachable!(),
                };
                let mut kind = kind;
                if let (Some((x, y)), Kind::Click { zoom, .. }) = (at, &mut kind) {
                    let (lx, ly) = ((x - screen.x) as f32 * k, (y - screen.y) as f32 * k);
                    capture::mark_click(&mut img, lx, ly, k.max(1.0));
                    let z = dir.join(format!("{stem}-zoom.jpg"));
                    capture::save_jpeg(&capture::crop_around(&img, lx, ly, (900.0 * k) as u32, (560.0 * k) as u32), &z, 1800).ok()?;
                    *zoom = z.display().to_string();
                }
                decorate(&mut img, k);
                let path = dir.join(format!("{stem}.jpg"));
                capture::save_jpeg(&img, &path, 1920).ok()?;
                events.lock().push(Event { t, n, kind, img: path.display().to_string() });
                Some(())
            })();
            if r.is_none() {
                eprintln!("capture {n} échouée");
            }
            pending.fetch_sub(1, Ordering::SeqCst);
        });
    }

    pub fn click(&self, x: i32, y: i32) {
        self.snapshot(Some((x, y)), |_, _| {}, move || {
            let win = active_win_pos_rs::get_active_window().ok();
            let (app, title) = win.map(|w| (w.app_name, w.title)).unwrap_or_default();
            Kind::Click { app, title, element: platform::element_at(x, y), zoom: String::new() }
        });
    }

    /// Strokes in 0..1 coordinates of the overlay, which covers `screen` exactly.
    pub fn strokes(&self, strokes: Vec<Vec<[f32; 2]>>) {
        let (w, h) = (self.screen.w as f32, self.screen.h as f32);
        self.snapshot(
            None,
            move |img, k| {
                let px: Vec<Vec<(f32, f32)>> =
                    strokes.iter().map(|s| s.iter().map(|p| (p[0] * w * k, p[1] * h * k)).collect()).collect();
                capture::draw_strokes(img, &px, k.max(1.0));
            },
            || Kind::Drawing,
        );
    }

    pub fn board(&self, png: &[u8]) {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let path = self.dir.join(format!("{n:02}-tableau.png"));
        if std::fs::write(&path, png).is_ok() {
            self.events.lock().push(Event { t: self.elapsed(), n, kind: Kind::Board, img: path.display().to_string() });
        }
    }

    /// Stops recording, waits for every transcription and capture, writes the tunnel and returns its text.
    pub fn finish(mut self) -> Result<(String, PathBuf), String> {
        let duration = self.elapsed();
        self.rec.stop();
        self.pump(true);
        let all: Vec<f32> = self.rec.samples.lock().clone();
        let dir = self.dir.clone();
        let audio_thread = std::thread::spawn(move || std::fs::write(dir.join("voix.wav"), audio::wav_bytes(&all)));
        let mut segments = vec![];
        let mut errors = vec![];
        for j in self.jobs.drain(..) {
            match j.join() {
                Ok(Ok(s)) => segments.extend(s),
                Ok(Err(e)) => errors.push(e),
                Err(_) => errors.push("transcription interrompue".into()),
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.pending.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(15));
        }
        segments.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut events = std::mem::take(&mut *self.events.lock());
        events.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.n.cmp(&b.n)));
        let text = render(&self, duration, &segments, &events, &errors);
        let _ = std::fs::write(self.dir.join("tunnel.md"), &text);
        let _ = audio_thread.join();
        if segments.is_empty() && !errors.is_empty() {
            return Err(errors.join(" ; "));
        }
        Ok((text, self.dir.clone()))
    }

    pub fn discard(mut self) {
        self.rec.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn clock(t: f64) -> String {
    let s = t.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn describe(e: &Event) -> String {
    let t = clock(e.t);
    match &e.kind {
        Kind::Start => format!("[{t}] ÉCRAN AU DÉPART — {}", e.img),
        Kind::Click { app, title, element, zoom } => {
            let mut s = format!("[{t}] CLIC {}", e.n);
            let place = match (app.is_empty(), title.is_empty()) {
                (false, false) => format!("{app} — « {title} »"),
                (false, true) => app.clone(),
                (true, false) => format!("« {title} »"),
                _ => String::new(),
            };
            if !place.is_empty() {
                s += &format!(" · {place}");
            }
            if let Some(el) = element {
                s += &format!(" · sur : {el}");
            }
            s += &format!(" — capture {} (zoom {})", e.img, zoom);
            s
        }
        Kind::Drawing => format!("[{t}] DESSIN {} sur l'écran — {}", e.n, e.img),
        Kind::Board => format!("[{t}] TABLEAU BLANC {} — {}", e.n, e.img),
    }
}

fn render(s: &Session, duration: f64, segments: &[Segment], events: &[Event], errors: &[String]) -> String {
    let mins = (duration / 60.0) as u64;
    let secs = duration as u64 % 60;
    let mut out = format!(
        "# Tunnel du {} · {} min {:02} s\n\n\
Je t'explique ce que je veux en parlant, en cliquant et en dessinant sur mon écran. \
Ci-dessous, ma voix transcrite mot à mot et horodatée, entrecoupée de ce que je fais au même moment. \
Chaque capture montre mon écran à cet instant : le rond blanc marque l'endroit exact où j'ai cliqué, \
les traits blancs sont ce que j'ai dessiné. Ouvre les images citées : elles portent autant de sens que mes mots.\n\n",
        s.started.format("%d/%m/%Y %H:%M"),
        mins,
        secs
    );
    let mut ev = events.iter().peekable();
    for seg in segments {
        while let Some(e) = ev.next_if(|e| e.t <= seg.start) {
            out += &format!("> {}\n\n", describe(e));
        }
        out += &format!("[{}] {}\n\n", clock(seg.start), seg.text);
    }
    for e in ev {
        out += &format!("> {}\n\n", describe(e));
    }
    if segments.is_empty() {
        out += "(aucune parole transcrite)\n\n";
    }
    if !errors.is_empty() {
        out += &format!("_Transcription incomplète : {}_\n\n", errors.join(" ; "));
    }
    out += &format!("Dossier du tunnel (captures, audio) : {}\n", s.dir.display());
    out
}
