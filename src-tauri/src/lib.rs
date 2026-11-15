mod audio;
mod capture;
mod config;
mod platform;
mod session;
mod transcribe;

use base64::Engine;
use capture::Screen;
use config::Config;
use parking_lot::Mutex;
use serde_json::json;
use session::Session;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

const IDLE: u8 = 0;
const RECORDING: u8 = 1;
const PROCESSING: u8 = 2;

const LIVE: u8 = 0;
const DRAW: u8 = 1;
const BOARD: u8 = 2;

/// Size of the side rail in CSS pixels; ui/index.html uses the same numbers.
const RAIL_W: f64 = 64.0;
const RAIL_H: f64 = 260.0;
const HOLD_MS: u128 = 400;

#[derive(Default)]
struct App {
    phase: AtomicU8,
    mode: AtomicU8,
    session: Mutex<Option<Session>>,
    pressed_at: Mutex<Option<(Instant, bool)>>,
    flushed: AtomicBool,
    over_rail: AtomicBool,
}

type State = Arc<App>;

fn overlay(app: &AppHandle) -> WebviewWindow {
    app.get_webview_window("overlay").expect("overlay window")
}

fn hotkey() -> Shortcut {
    #[cfg(target_os = "macos")]
    let m = Modifiers::ALT;
    #[cfg(not(target_os = "macos"))]
    let m = Modifiers::CONTROL;
    Shortcut::new(Some(m), Code::Space)
}

fn escape() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

fn rail_contains(s: &Screen, x: f64, y: f64) -> bool {
    let w = RAIL_W * s.scale;
    let h = RAIL_H * s.scale;
    let cy = s.y as f64 + s.h as f64 / 2.0;
    x >= s.x as f64 + s.w as f64 - w && x <= (s.x as f64 + s.w as f64) && (y - cy).abs() <= h / 2.0
}

fn screen_under_cursor(app: &AppHandle) -> Option<Screen> {
    let p = app.cursor_position().ok()?;
    let m = app
        .monitor_from_point(p.x, p.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    Some(Screen { x: m.position().x, y: m.position().y, w: m.size().width, h: m.size().height, scale: m.scale_factor() })
}

fn start(app: &AppHandle, st: &State) {
    if st.phase.compare_exchange(IDLE, RECORDING, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let focus = platform::foreground();
    let Some(screen) = screen_under_cursor(app) else {
        st.phase.store(IDLE, Ordering::SeqCst);
        return;
    };
    let w = overlay(app);
    let _ = w.set_position(PhysicalPosition::new(screen.x, screen.y));
    let _ = w.set_size(PhysicalSize::new(screen.w, screen.h));
    let _ = w.set_ignore_cursor_events(true);
    st.mode.store(LIVE, Ordering::SeqCst);
    let _ = w.emit("tunnel", json!({ "state": "recording" }));
    let _ = w.show();
    let _ = w.set_always_on_top(true);
    match Session::start(&Config::load(), screen, focus) {
        Ok(s) => *st.session.lock() = Some(s),
        Err(e) => {
            fail(app, st, &e);
            return;
        }
    }
    spawn_ticker(app.clone(), st.clone(), screen);
    spawn_clicks(app.clone(), st.clone(), screen);
}

fn fail(app: &AppHandle, st: &State, msg: &str) {
    let w = overlay(app);
    let _ = w.emit("tunnel", json!({ "state": "error", "message": msg }));
    st.phase.store(IDLE, Ordering::SeqCst);
    hide_later(app.clone(), st.clone(), 3200);
}

fn end(app: &AppHandle, st: &State, keep: bool) {
    if st.phase.compare_exchange(RECORDING, PROCESSING, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let _ = app.global_shortcut().unregister(escape());
    let w = overlay(app);
    let _ = w.set_ignore_cursor_events(true);
    let drawing = st.mode.swap(LIVE, Ordering::SeqCst) != LIVE;
    st.flushed.store(!drawing, Ordering::SeqCst);
    let _ = w.emit("tunnel", json!({ "state": if keep { "processing" } else { "cancelled" } }));
    let (app, st) = (app.clone(), st.clone());
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !st.flushed.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let Some(session) = st.session.lock().take() else {
            st.phase.store(IDLE, Ordering::SeqCst);
            return;
        };
        if !keep {
            session.discard();
            st.phase.store(IDLE, Ordering::SeqCst);
            hide_later(app.clone(), st.clone(), 450);
            return;
        }
        let focus = session.focus;
        let cfg = Config::load();
        match session.finish() {
            Ok((text, _dir)) => {
                let copied = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone())).is_ok();
                let _ = overlay(&app).emit("tunnel", json!({ "state": "done", "chars": text.chars().count() }));
                std::thread::sleep(Duration::from_millis(380));
                let _ = overlay(&app).hide();
                if copied && cfg.auto_paste && platform::restore(focus) {
                    std::thread::sleep(Duration::from_millis(90));
                    platform::paste();
                }
                st.phase.store(IDLE, Ordering::SeqCst);
            }
            Err(e) => fail(&app, &st, &e),
        }
    });
}

fn hide_later(app: AppHandle, st: State, ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        if st.phase.load(Ordering::SeqCst) == IDLE {
            let _ = overlay(&app).hide();
        }
    });
}

/// Voice level for the frame animation, and click-through except over the rail or while drawing.
fn spawn_ticker(app: AppHandle, st: State, screen: Screen) {
    std::thread::spawn(move || {
        let w = overlay(&app);
        let mut through = true;
        let mut last_pump = Instant::now();
        while st.phase.load(Ordering::SeqCst) == RECORDING {
            let (level, t) = match st.session.lock().as_ref() {
                Some(s) => (s.rec.level(), s.elapsed()),
                None => break,
            };
            let _ = w.emit("level", json!({ "level": level, "t": t }));
            let over = app.cursor_position().map(|p| rail_contains(&screen, p.x, p.y)).unwrap_or(false);
            st.over_rail.store(over, Ordering::SeqCst);
            let want = st.mode.load(Ordering::SeqCst) == LIVE && !over;
            if want != through {
                through = want;
                let _ = w.set_ignore_cursor_events(want);
            }
            if last_pump.elapsed() > Duration::from_millis(500) {
                last_pump = Instant::now();
                if let Some(s) = st.session.lock().as_mut() {
                    s.pump(false);
                }
            }
            std::thread::sleep(Duration::from_millis(33));
        }
    });
}

/// Global left clicks while live: each one becomes a captured, located step of the tunnel.
fn spawn_clicks(app: AppHandle, st: State, screen: Screen) {
    std::thread::spawn(move || {
        use device_query::DeviceQuery;
        let Some(dev) = device_query::DeviceState::checked_new() else { return };
        let mut was = true;
        while st.phase.load(Ordering::SeqCst) == RECORDING {
            let down = dev.get_mouse().button_pressed.get(1).copied().unwrap_or(false);
            if down && !was && st.mode.load(Ordering::SeqCst) == LIVE && !st.over_rail.load(Ordering::SeqCst) {
                if let Ok(p) = app.cursor_position() {
                    let (x, y) = (p.x as i32, p.y as i32);
                    if screen.contains(x, y) {
                        if let Some(s) = st.session.lock().as_ref() {
                            s.click(x, y);
                        }
                        let _ = overlay(&app).emit("click", json!({ "x": (x - screen.x) as f64 / screen.scale, "y": (y - screen.y) as f64 / screen.scale }));
                    }
                }
            }
            was = down;
            std::thread::sleep(Duration::from_millis(8));
        }
    });
}

fn on_shortcut(app: &AppHandle, sc: &Shortcut, ev: ShortcutEvent) {
    let state = ev.state();
    let st = app.state::<State>().inner().clone();
    if *sc == escape() {
        if state == ShortcutState::Pressed {
            set_mode_inner(app, &st, LIVE);
        }
        return;
    }
    match state {
        ShortcutState::Pressed => {
            let mut p = st.pressed_at.lock();
            if p.is_some() {
                return;
            }
            match st.phase.load(Ordering::SeqCst) {
                IDLE => {
                    *p = Some((Instant::now(), true));
                    drop(p);
                    start(app, &st);
                }
                RECORDING => {
                    *p = Some((Instant::now(), false));
                    drop(p);
                    end(app, &st, true);
                }
                _ => {}
            }
        }
        ShortcutState::Released => {
            let Some((t, started)) = st.pressed_at.lock().take() else { return };
            if started && t.elapsed().as_millis() >= HOLD_MS {
                end(app, &st, true);
            }
        }
    }
}

fn set_mode_inner(app: &AppHandle, st: &State, mode: u8) {
    st.mode.store(mode, Ordering::SeqCst);
    let w = overlay(app);
    // Escape is only taken from other apps while drawing, to leave the drawing.
    let _ = if mode == LIVE { app.global_shortcut().unregister(escape()) } else { app.global_shortcut().register(escape()) };
    let name = ["live", "draw", "board"][mode as usize];
    let _ = w.emit("mode", json!({ "mode": name }));
}

#[tauri::command]
fn set_mode(app: AppHandle, st: tauri::State<State>, mode: String) {
    let m = match mode.as_str() {
        "draw" => DRAW,
        "board" => BOARD,
        _ => LIVE,
    };
    set_mode_inner(&app, &st, m);
}

#[tauri::command]
fn stroke(st: tauri::State<State>, strokes: Vec<Vec<[f32; 2]>>) {
    if let Some(s) = st.session.lock().as_ref() {
        s.strokes(strokes);
    }
}

#[tauri::command]
fn board(st: tauri::State<State>, png: String) {
    let b64 = png.split_once(',').map(|(_, d)| d).unwrap_or(&png);
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
        if let Some(s) = st.session.lock().as_ref() {
            s.board(&bytes);
        }
    }
}

#[tauri::command]
fn flushed(st: tauri::State<State>) {
    st.flushed.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn finish(app: AppHandle, st: tauri::State<State>, keep: bool) {
    end(&app, st.inner(), keep);
}

pub fn run() {
    let state: State = Arc::new(App::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(on_shortcut).build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![set_mode, stroke, board, flushed, finish])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let _ = overlay(app.handle()).set_ignore_cursor_events(true);
            if let Err(e) = app.global_shortcut().register(hotkey()) {
                eprintln!("raccourci indisponible : {e}");
            }
            {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                if !al.is_enabled().unwrap_or(true) {
                    let _ = al.enable();
                }
            }
            let cfg = Config::load();
            let open_dir = MenuItem::with_id(app, "dir", "Ouvrir les tunnels", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "cfg", "Réglages (clé Groq…)", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_dir, &settings, &quit])?;
            let tunnels = cfg.tunnels_dir();
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip(if cfg!(target_os = "macos") { "Unitunel · ⌥ Espace" } else { "Unitunel · Ctrl+Espace" })
                .menu(&menu)
                .on_menu_event(move |app, e| match e.id().as_ref() {
                    "dir" => {
                        let _ = std::fs::create_dir_all(&tunnels);
                        let _ = open::that(&tunnels);
                    }
                    "cfg" => {
                        let _ = open::that(config::config_path());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("unitunel");
}
