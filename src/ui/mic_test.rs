use crate::config::AppConfig;
use crate::i18n;
use gtk::glib;
use gtk::prelude::*;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const SAMPLE_RATE: usize = 48_000;
const FRAME_SAMPLES: usize = 960; // 20 ms at 48 kHz mono
const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
const MAX_RECORD_SECS: usize = 10;
const MAX_RECORD_BYTES: usize = SAMPLE_RATE * 2 * MAX_RECORD_SECS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHealth {
    pub name: String,
    pub description: String,
    pub muted: bool,
    pub volume_percent: Option<u32>,
    pub active_port: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHealthKind {
    Muted,
    LowVolume(u32),
    Ready(Option<u32>),
}

impl SourceHealth {
    pub fn kind(&self) -> SourceHealthKind {
        if self.muted {
            return SourceHealthKind::Muted;
        }
        match self.volume_percent {
            Some(pct) if pct < 50 => SourceHealthKind::LowVolume(pct),
            other => SourceHealthKind::Ready(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcmMetrics {
    pub peak: u16,
    pub rms_dbfs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceLevelKind {
    Silent,
    Quiet,
    Good,
    Clipping,
}

const SOFT_LIMITER_KNEE: f64 = 0.85;
// Matches protocol/src/call_audio.ts MIC_VOICE_FILTER so Settings mic check
// reflects the exact voice cleanup heard by the remote peer during calls.
const MIC_VOICE_FILTER: &str = "highpass=f=90,lowpass=f=12000,afftdn=nr=20:nf=-20:tn=1,agate=threshold=-38dB:ratio=3:range=0.12:attack=10:release=220:knee=6";

pub fn apply_gain_pcm(raw: &[u8], gain: f64) -> Vec<u8> {
    let gain = if gain.is_finite() {
        gain.clamp(0.0, 4.0)
    } else {
        1.0
    };
    let even_len = raw.len() & !1;
    if (gain - 1.0).abs() < f64::EPSILON {
        return raw[..even_len].to_vec();
    }
    let headroom = 1.0 - SOFT_LIMITER_KNEE;
    let mut out = Vec::with_capacity(even_len);
    for &[lo, hi] in raw[..even_len].as_chunks::<2>().0 {
        let norm = (i16::from_le_bytes([lo, hi]) as f64 * gain) / 32768.0;
        let abs = norm.abs();
        let limited = if abs <= SOFT_LIMITER_KNEE {
            norm
        } else {
            norm.signum()
                * (SOFT_LIMITER_KNEE + headroom * ((abs - SOFT_LIMITER_KNEE) / headroom).tanh())
        };
        let scaled = (limited * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

pub fn analyze_pcm_s16le(bytes: &[u8]) -> PcmMetrics {
    let even_len = bytes.len() & !1;
    let sample_count = even_len / 2;
    if sample_count == 0 {
        return PcmMetrics {
            peak: 0,
            rms_dbfs: -120.0,
        };
    }
    let mut peak: u16 = 0;
    let mut square_sum: f64 = 0.0;
    for &[lo, hi] in bytes[..even_len].as_chunks::<2>().0 {
        let s = i16::from_le_bytes([lo, hi]) as i32;
        let abs = s.unsigned_abs().min(u16::MAX as u32) as u16;
        if abs > peak {
            peak = abs;
        }
        let sf = s as f64;
        square_sum += sf * sf;
    }
    let rms_dbfs = if square_sum > 0.0 {
        let rms = (square_sum / sample_count as f64).sqrt();
        ((200.0 * (rms / 32768.0).log10()).round() / 10.0).clamp(-120.0, 0.0)
    } else {
        -120.0
    };
    PcmMetrics { peak, rms_dbfs }
}

pub fn meter_fraction_from_dbfs(rms_dbfs: f64) -> f64 {
    if !rms_dbfs.is_finite() || rms_dbfs <= -60.0 {
        return 0.0;
    }
    ((rms_dbfs + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn classify_voice_level(metrics: PcmMetrics) -> VoiceLevelKind {
    if metrics.peak >= 32_500 || metrics.rms_dbfs >= -3.0 {
        VoiceLevelKind::Clipping
    } else if metrics.rms_dbfs >= -30.0 {
        VoiceLevelKind::Good
    } else if metrics.rms_dbfs >= -45.0 {
        VoiceLevelKind::Quiet
    } else {
        VoiceLevelKind::Silent
    }
}

fn parse_percent_str(s: &str) -> Option<u32> {
    s.trim().trim_end_matches('%').trim().parse::<u32>().ok()
}

fn extract_volume_percent(obj: &serde_json::Value) -> Option<u32> {
    let map = obj.get("volume")?.as_object()?;
    let mut sum: u32 = 0;
    let mut count: u32 = 0;
    for ch in map.values() {
        if let Some(pct) = ch
            .get("value_percent")
            .and_then(|v| v.as_str())
            .and_then(parse_percent_str)
        {
            sum = sum.saturating_add(pct);
            count += 1;
        }
    }
    sum.checked_div(count)
}

pub fn parse_pulse_devices_json(json_str: &str, is_source: bool) -> Option<Vec<(String, String)>> {
    let items: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let mut list = vec![("default".to_string(), i18n::t("audio_device_default"))];
    for item in items {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if is_source && name.contains(".monitor") {
            continue;
        }
        let desc = item
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(name);
        list.push((name.to_string(), desc.to_string()));
    }
    Some(list)
}

pub fn parse_source_health_json(
    json_str: &str,
    selected_input: &str,
    default_source: Option<&str>,
) -> Option<SourceHealth> {
    let target = if selected_input.trim().is_empty() || selected_input.trim() == "default" {
        default_source?.trim()
    } else {
        selected_input.trim()
    };
    if target.is_empty() {
        return None;
    }
    let items: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let item = items
        .into_iter()
        .find(|v| v.get("name").and_then(|n| n.as_str()) == Some(target))?;
    let name = item.get("name")?.as_str()?.to_string();
    let description = item
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&name)
        .to_string();
    let muted = item.get("mute").and_then(|v| v.as_bool()).unwrap_or(false);
    let volume_percent = extract_volume_percent(&item);
    let active_port = item
        .get("active_port")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(SourceHealth {
        name,
        description,
        muted,
        volume_percent,
        active_port,
    })
}

pub fn inspect_pulse_source(selected_input: &str) -> Option<SourceHealth> {
    let default_source = if selected_input.trim().is_empty() || selected_input.trim() == "default" {
        Command::new("pactl")
            .arg("get-default-source")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    };
    let out = Command::new("pactl")
        .args(["--format=json", "list", "sources"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_source_health_json(&text, selected_input, default_source.as_deref())
}

fn spawn_playback_ffmpeg(output_device: &str) -> std::io::Result<Child> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "s16le".to_string(),
        "-ar".to_string(),
        SAMPLE_RATE.to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-i".to_string(),
        "pipe:0".to_string(),
        "-f".to_string(),
        "pulse".to_string(),
    ];
    let sink = output_device.trim();
    if !sink.is_empty() && sink != "default" {
        args.push("-device".to_string());
        args.push(sink.to_string());
    }
    args.push("LINE GTK mic test".to_string());
    Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

fn spawn_capture_ffmpeg(input_device: &str) -> std::io::Result<Child> {
    let source = if input_device.trim().is_empty() {
        "default"
    } else {
        input_device.trim()
    };
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "pulse",
            "-i",
            source,
            "-ac",
            "1",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-af",
            MIC_VOICE_FILTER,
            "-f",
            "s16le",
            "-flush_packets",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn kill_child_slot(slot: &Arc<Mutex<Option<Child>>>) {
    if let Ok(mut guard) = slot.lock()
        && let Some(mut child) = guard.take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[derive(Debug, Clone)]
enum MicEvent {
    Level {
        generation: u64,
        gained: PcmMetrics,
        recorded_secs: f64,
    },
    CaptureStopped {
        generation: u64,
        error: Option<String>,
        recorded_secs: f64,
    },
    PlaybackStopped {
        generation: u64,
        error: Option<String>,
    },
}

struct MicTestController {
    active_stop: RefCell<Option<Arc<AtomicBool>>>,
    gain_bits: Arc<AtomicU32>,
    loopback_enabled: Arc<AtomicBool>,
    recorded_pcm: Arc<Mutex<Vec<u8>>>,
    capture_child: Arc<Mutex<Option<Child>>>,
    playback_child: Arc<Mutex<Option<Child>>>,
    tx: async_channel::Sender<MicEvent>,
    generation: Cell<u64>,
    capturing: Cell<bool>,
    playing: Cell<bool>,
}

impl MicTestController {
    fn new(initial_gain: f64) -> (Self, async_channel::Receiver<MicEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let ctl = Self {
            active_stop: RefCell::new(None),
            gain_bits: Arc::new(AtomicU32::new((initial_gain as f32).to_bits())),
            loopback_enabled: Arc::new(AtomicBool::new(false)),
            recorded_pcm: Arc::new(Mutex::new(Vec::new())),
            capture_child: Arc::new(Mutex::new(None)),
            playback_child: Arc::new(Mutex::new(None)),
            tx,
            generation: Cell::new(0),
            capturing: Cell::new(false),
            playing: Cell::new(false),
        };
        (ctl, rx)
    }

    fn set_gain(&self, gain: f64) {
        self.gain_bits
            .store((gain.clamp(0.0, 4.0) as f32).to_bits(), Ordering::Relaxed);
    }

    fn set_loopback(&self, enabled: bool) {
        self.loopback_enabled.store(enabled, Ordering::Relaxed);
        if !enabled && self.capturing.get() {
            kill_child_slot(&self.playback_child);
        }
    }

    fn has_recording(&self) -> bool {
        self.recorded_pcm
            .lock()
            .map(|buf| buf.len() >= FRAME_BYTES * 5)
            .unwrap_or(false)
    }

    fn recorded_secs(&self) -> f64 {
        self.recorded_pcm
            .lock()
            .map(|buf| buf.len() as f64 / (SAMPLE_RATE * 2) as f64)
            .unwrap_or(0.0)
    }

    fn stop_all(&self) {
        if let Some(flag) = self.active_stop.borrow_mut().take() {
            flag.store(true, Ordering::SeqCst);
        }
        kill_child_slot(&self.capture_child);
        kill_child_slot(&self.playback_child);
        self.capturing.set(false);
        self.playing.set(false);
    }

    fn start_capture(&self, input_device: String, output_device: String) {
        self.stop_all();
        if let Ok(mut buf) = self.recorded_pcm.lock() {
            buf.clear();
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let stop_flag = Arc::new(AtomicBool::new(false));
        *self.active_stop.borrow_mut() = Some(stop_flag.clone());
        self.capturing.set(true);

        let gain_bits = self.gain_bits.clone();
        let loopback_enabled = self.loopback_enabled.clone();
        let recorded_pcm = self.recorded_pcm.clone();
        let capture_child = self.capture_child.clone();
        let playback_child = self.playback_child.clone();
        let tx = self.tx.clone();

        std::thread::spawn(move || {
            let mut child = match spawn_capture_ffmpeg(&input_device) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send_blocking(MicEvent::CaptureStopped {
                        generation,
                        error: Some(e.to_string()),
                        recorded_secs: 0.0,
                    });
                    return;
                }
            };
            let Some(mut stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();
                let _ = tx.send_blocking(MicEvent::CaptureStopped {
                    generation,
                    error: Some("capture stdout unavailable".into()),
                    recorded_secs: 0.0,
                });
                return;
            };
            if let Ok(mut slot) = capture_child.lock() {
                *slot = Some(child);
            }

            let mut loopback_stdin: Option<std::process::ChildStdin> = None;
            let mut buf = vec![0u8; FRAME_BYTES * 2]; // 40 ms chunks (25 Hz UI updates)
            let mut err_msg: Option<String> = None;

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                let mut filled = 0;
                while filled < buf.len() && !stop_flag.load(Ordering::SeqCst) {
                    match stdout.read(&mut buf[filled..]) {
                        Ok(0) => break,
                        Ok(n) => filled += n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            if !stop_flag.load(Ordering::SeqCst) {
                                err_msg = Some(e.to_string());
                            }
                            break;
                        }
                    }
                }
                if filled < 2 || stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                let raw_slice = &buf[..(filled & !1)];
                let gain = f32::from_bits(gain_bits.load(Ordering::Relaxed)) as f64;
                let gained_pcm = apply_gain_pcm(raw_slice, gain);
                let gained_metrics = analyze_pcm_s16le(&gained_pcm);

                let recorded_secs = if let Ok(mut rec) = recorded_pcm.lock() {
                    rec.extend_from_slice(&gained_pcm);
                    if rec.len() > MAX_RECORD_BYTES {
                        let excess = (rec.len() - MAX_RECORD_BYTES) & !1;
                        rec.drain(..excess);
                    }
                    rec.len() as f64 / (SAMPLE_RATE * 2) as f64
                } else {
                    0.0
                };

                if loopback_enabled.load(Ordering::Relaxed) {
                    if loopback_stdin.is_none()
                        && let Ok(mut spk) = spawn_playback_ffmpeg(&output_device)
                    {
                        loopback_stdin = spk.stdin.take();
                        if let Ok(mut slot) = playback_child.lock() {
                            *slot = Some(spk);
                        }
                    }
                    if let Some(stdin) = loopback_stdin.as_mut()
                        && stdin.write_all(&gained_pcm).is_err()
                    {
                        loopback_stdin = None;
                        kill_child_slot(&playback_child);
                    }
                } else if loopback_stdin.is_some() {
                    loopback_stdin = None;
                    kill_child_slot(&playback_child);
                }

                let _ = tx.send_blocking(MicEvent::Level {
                    generation,
                    gained: gained_metrics,
                    recorded_secs,
                });
            }

            drop(loopback_stdin);
            kill_child_slot(&playback_child);
            kill_child_slot(&capture_child);

            let recorded_secs = recorded_pcm
                .lock()
                .map(|r| r.len() as f64 / (SAMPLE_RATE * 2) as f64)
                .unwrap_or(0.0);
            let _ = tx.send_blocking(MicEvent::CaptureStopped {
                generation,
                error: if stop_flag.load(Ordering::SeqCst) {
                    None
                } else {
                    err_msg
                },
                recorded_secs,
            });
        });
    }

    fn start_playback(&self, output_device: String) {
        self.stop_all();
        let pcm = self
            .recorded_pcm
            .lock()
            .map(|buf| buf.clone())
            .unwrap_or_default();
        if pcm.len() < FRAME_BYTES {
            return;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let stop_flag = Arc::new(AtomicBool::new(false));
        *self.active_stop.borrow_mut() = Some(stop_flag.clone());
        self.playing.set(true);

        let playback_child = self.playback_child.clone();
        let tx = self.tx.clone();

        std::thread::spawn(move || {
            let mut child = match spawn_playback_ffmpeg(&output_device) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send_blocking(MicEvent::PlaybackStopped {
                        generation,
                        error: Some(e.to_string()),
                    });
                    return;
                }
            };
            let Some(mut stdin) = child.stdin.take() else {
                let _ = child.kill();
                let _ = child.wait();
                let _ = tx.send_blocking(MicEvent::PlaybackStopped {
                    generation,
                    error: Some("speaker stdin unavailable".into()),
                });
                return;
            };
            if let Ok(mut slot) = playback_child.lock() {
                *slot = Some(child);
            }

            let mut err_msg: Option<String> = None;
            for chunk in pcm.chunks(FRAME_BYTES) {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(e) = stdin.write_all(chunk) {
                    if !stop_flag.load(Ordering::SeqCst) {
                        err_msg = Some(e.to_string());
                    }
                    break;
                }
            }
            drop(stdin);

            if stop_flag.load(Ordering::SeqCst) {
                kill_child_slot(&playback_child);
            } else if let Ok(mut slot) = playback_child.lock()
                && let Some(mut c) = slot.take()
            {
                let _ = c.wait();
            }

            let _ = tx.send_blocking(MicEvent::PlaybackStopped {
                generation,
                error: if stop_flag.load(Ordering::SeqCst) {
                    None
                } else {
                    err_msg
                },
            });
        });
    }
}

impl Drop for MicTestController {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub struct MicTestHandle {
    ctl: Rc<MicTestController>,
    refresh_health: Rc<dyn Fn()>,
}

impl MicTestHandle {
    pub fn set_gain(&self, gain: f64) {
        self.ctl.set_gain(gain);
    }

    pub fn on_devices_changed(&self) {
        self.ctl.stop_all();
        (self.refresh_health)();
    }

    pub fn stop(&self) {
        self.ctl.stop_all();
    }
}

fn format_idle_subtitle(health: Option<&SourceHealth>, recorded_secs: f64) -> String {
    let base = match health.map(SourceHealth::kind) {
        Some(SourceHealthKind::Muted) => i18n::t("mic_test_status_muted"),
        Some(SourceHealthKind::LowVolume(pct)) => {
            i18n::tf("mic_test_status_low_sys", &[("pct", &pct.to_string())])
        }
        Some(SourceHealthKind::Ready(Some(pct))) => {
            i18n::tf("mic_test_status_ready_pct", &[("pct", &pct.to_string())])
        }
        _ => i18n::t("mic_test_subtitle"),
    };
    if recorded_secs >= 0.2 {
        format!(
            "{base} • {}",
            i18n::tf(
                "mic_test_recorded",
                &[("secs", &format!("{recorded_secs:.1}"))]
            )
        )
    } else {
        base
    }
}

pub fn attach_mic_test_ui(
    audio_grp: &libadwaita::PreferencesGroup,
    win: &libadwaita::PreferencesWindow,
    config: Rc<RefCell<AppConfig>>,
) -> MicTestHandle {
    let initial_gain = config.borrow().call_mic_volume;
    let (ctl, rx) = MicTestController::new(initial_gain);
    let ctl = Rc::new(ctl);

    let test_row = libadwaita::ActionRow::builder()
        .title(i18n::t("mic_test_title"))
        .subtitle(i18n::t("mic_test_subtitle"))
        .build();

    let test_btn = gtk::Button::builder()
        .label(i18n::t("mic_test_start"))
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();

    let play_btn = gtk::Button::builder()
        .label(i18n::t("mic_test_play"))
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();

    test_row.add_suffix(&play_btn);
    test_row.add_suffix(&test_btn);
    audio_grp.add(&test_row);

    // Live input level bar row
    let meter_row = gtk::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .build();
    let meter_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(14)
        .margin_end(14)
        .build();
    let mic_icon = gtk::Image::from_icon_name("microphone-sensitivity-high-symbolic");
    let level_bar = gtk::LevelBar::for_interval(0.0, 1.0);
    level_bar.set_hexpand(true);
    level_bar.set_valign(gtk::Align::Center);
    level_bar.set_min_value(0.0);
    level_bar.set_max_value(1.0);
    level_bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_LOW, 0.25);
    level_bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_HIGH, 0.85);
    level_bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_FULL, 0.96);
    let db_label = gtk::Label::builder()
        .label("-- dB")
        .width_chars(9)
        .xalign(1.0)
        .css_classes(["caption", "dim-label", "numeric"])
        .build();
    meter_box.append(&mic_icon);
    meter_box.append(&level_bar);
    meter_box.append(&db_label);
    meter_row.set_child(Some(&meter_box));
    audio_grp.add(&meter_row);

    let loopback_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("mic_test_loopback"))
        .subtitle(i18n::t("mic_test_loopback_subtitle"))
        .active(false)
        .build();
    {
        let ctl = ctl.clone();
        loopback_row.connect_active_notify(move |row| {
            ctl.set_loopback(row.is_active());
        });
    }
    audio_grp.add(&loopback_row);

    let last_health: Rc<RefCell<Option<SourceHealth>>> = Rc::new(RefCell::new(None));

    let refresh_health: Rc<dyn Fn()> = {
        let cfg = config.clone();
        let ctl = ctl.clone();
        let test_row = test_row.clone();
        let test_btn = test_btn.clone();
        let play_btn = play_btn.clone();
        let level_bar = level_bar.clone();
        let db_label = db_label.clone();
        let last_health = last_health.clone();
        Rc::new(move || {
            let input = cfg.borrow().audio_input.clone();
            let health = inspect_pulse_source(&input);
            *last_health.borrow_mut() = health.clone();
            if !ctl.capturing.get() && !ctl.playing.get() {
                test_row.set_subtitle(&format_idle_subtitle(health.as_ref(), ctl.recorded_secs()));
                test_btn.set_label(&i18n::t("mic_test_start"));
                test_btn.remove_css_class("destructive-action");
                test_btn.add_css_class("suggested-action");
                play_btn.set_label(&i18n::t("mic_test_play"));
                play_btn.set_sensitive(ctl.has_recording());
                level_bar.set_value(0.0);
                db_label.set_text("-- dB");
            }
        })
    };
    refresh_health();

    {
        let ctl = ctl.clone();
        let cfg = config.clone();
        let test_row = test_row.clone();
        let play_btn = play_btn.clone();
        let level_bar = level_bar.clone();
        let db_label = db_label.clone();
        let refresh = refresh_health.clone();
        test_btn.connect_clicked(move |btn| {
            if ctl.capturing.get() {
                ctl.stop_all();
                refresh();
                return;
            }
            let (input, output, gain) = {
                let c = cfg.borrow();
                (
                    c.audio_input.clone(),
                    c.audio_output.clone(),
                    c.call_mic_volume,
                )
            };
            ctl.set_gain(gain);
            btn.set_label(&i18n::t("mic_test_stop"));
            btn.remove_css_class("suggested-action");
            btn.add_css_class("destructive-action");
            play_btn.set_label(&i18n::t("mic_test_play"));
            play_btn.set_sensitive(false);
            test_row.set_subtitle(&i18n::t("mic_test_listening"));
            level_bar.set_value(0.0);
            db_label.set_text("-- dB");
            ctl.start_capture(input, output);
        });
    }

    {
        let ctl = ctl.clone();
        let cfg = config.clone();
        let test_row = test_row.clone();
        let test_btn = test_btn.clone();
        let refresh = refresh_health.clone();
        play_btn.connect_clicked(move |btn| {
            if ctl.playing.get() {
                ctl.stop_all();
                refresh();
                return;
            }
            if !ctl.has_recording() {
                return;
            }
            let output = cfg.borrow().audio_output.clone();
            test_btn.set_label(&i18n::t("mic_test_start"));
            test_btn.remove_css_class("destructive-action");
            test_btn.add_css_class("suggested-action");
            btn.set_label(&i18n::t("mic_test_stop_play"));
            test_row.set_subtitle(&i18n::t("mic_test_playing"));
            ctl.start_playback(output);
        });
    }

    // Drain worker events on GTK main loop
    {
        let ctl = ctl.clone();
        let test_row = test_row.clone();
        let test_btn = test_btn.clone();
        let play_btn = play_btn.clone();
        let level_bar = level_bar.clone();
        let db_label = db_label.clone();
        let last_health = last_health.clone();
        let weak_win = win.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                if weak_win.upgrade().is_none() {
                    ctl.stop_all();
                    break;
                }
                match ev {
                    MicEvent::Level {
                        generation,
                        gained,
                        recorded_secs,
                    } => {
                        if generation != ctl.generation.get() || !ctl.capturing.get() {
                            continue;
                        }
                        level_bar.set_value(meter_fraction_from_dbfs(gained.rms_dbfs));
                        db_label.set_text(&if gained.rms_dbfs <= -119.0 {
                            "-- dB".to_string()
                        } else {
                            format!("{:.1} dB", gained.rms_dbfs)
                        });
                        let quality_key = match classify_voice_level(gained) {
                            VoiceLevelKind::Silent => "mic_test_level_silent",
                            VoiceLevelKind::Quiet => "mic_test_level_quiet",
                            VoiceLevelKind::Good => "mic_test_level_good",
                            VoiceLevelKind::Clipping => "mic_test_level_clipping",
                        };
                        let status = format!(
                            "{} • {}",
                            i18n::t(quality_key),
                            i18n::tf(
                                "mic_test_recorded",
                                &[("secs", &format!("{recorded_secs:.1}"))]
                            )
                        );
                        test_row.set_subtitle(&status);
                        play_btn.set_sensitive(recorded_secs >= 0.2);
                    }
                    MicEvent::CaptureStopped {
                        generation,
                        error,
                        recorded_secs,
                    } => {
                        if generation != ctl.generation.get() {
                            continue;
                        }
                        ctl.capturing.set(false);
                        test_btn.set_label(&i18n::t("mic_test_start"));
                        test_btn.remove_css_class("destructive-action");
                        test_btn.add_css_class("suggested-action");
                        level_bar.set_value(0.0);
                        db_label.set_text("-- dB");
                        play_btn.set_sensitive(ctl.has_recording());
                        if let Some(err) = error {
                            test_row.set_subtitle(&i18n::tf("mic_test_failed", &[("error", &err)]));
                        } else if !ctl.playing.get() {
                            test_row.set_subtitle(&format_idle_subtitle(
                                last_health.borrow().as_ref(),
                                recorded_secs,
                            ));
                        }
                    }
                    MicEvent::PlaybackStopped { generation, error } => {
                        if generation != ctl.generation.get() {
                            continue;
                        }
                        ctl.playing.set(false);
                        play_btn.set_label(&i18n::t("mic_test_play"));
                        play_btn.set_sensitive(ctl.has_recording());
                        if let Some(err) = error {
                            test_row.set_subtitle(&i18n::tf("mic_test_failed", &[("error", &err)]));
                        } else if !ctl.capturing.get() {
                            test_row.set_subtitle(&format_idle_subtitle(
                                last_health.borrow().as_ref(),
                                ctl.recorded_secs(),
                            ));
                        }
                    }
                }
            }
        });
    }

    {
        let ctl = ctl.clone();
        win.connect_close_request(move |_| {
            ctl.stop_all();
            glib::Propagation::Proceed
        });
    }

    MicTestHandle {
        ctl,
        refresh_health,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_pcm_computes_peak_and_dbfs_accurately() {
        let silent = vec![0u8; 1920];
        let m = analyze_pcm_s16le(&silent);
        assert_eq!(m.peak, 0);
        assert_eq!(m.rms_dbfs, -120.0);
        assert_eq!(classify_voice_level(m), VoiceLevelKind::Silent);
        assert_eq!(meter_fraction_from_dbfs(m.rms_dbfs), 0.0);

        // Full-scale square wave (+16384 / -16384) -> ~ -6.0 dBFS -> Good
        let mut loud = Vec::with_capacity(1920);
        for i in 0..960 {
            let s: i16 = if i % 2 == 0 { 16_384 } else { -16_384 };
            loud.extend_from_slice(&s.to_le_bytes());
        }
        let m2 = analyze_pcm_s16le(&loud);
        assert_eq!(m2.peak, 16_384);
        assert!((m2.rms_dbfs - (-6.0)).abs() <= 0.2);
        assert_eq!(classify_voice_level(m2), VoiceLevelKind::Good);
        assert!(meter_fraction_from_dbfs(m2.rms_dbfs) > 0.85);
    }

    #[test]
    fn apply_gain_scales_and_clamps_without_overflow() {
        let mut raw = Vec::new();
        for s in [1000i16, -20_000i16, 15_000i16, 20_000i16] {
            raw.extend_from_slice(&s.to_le_bytes());
        }
        let gained = apply_gain_pcm(&raw, 2.5);
        let samples: Vec<i16> = gained
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[lo, hi]| i16::from_le_bytes([lo, hi]))
            .collect();
        // Below soft-limiter knee (0.85 * 32768 = 27852), scaling stays linear.
        assert_eq!(samples[0], 2500);
        // Above knee, peaks compress smoothly without hard-clipping flat or overflowing.
        assert!(samples[1] < -28_000 && samples[1] > -32_767);
        assert!(samples[2] > 28_000 && samples[2] < samples[3]);
        assert!(samples[3] < 32_767);
    }

    #[test]
    fn parse_pulse_devices_json_uses_descriptions_and_skips_monitors_for_sources() {
        let json = r#"[
            {"name":"alsa_output.0.monitor","description":"Monitor of Audio"},
            {"name":"alsa_input.0","description":"Front Microphone"}
        ]"#;
        let sources = parse_pulse_devices_json(json, true).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].0, "default");
        assert_eq!(
            sources[1],
            ("alsa_input.0".into(), "Front Microphone".into())
        );

        let sinks = parse_pulse_devices_json(json, false).unwrap();
        assert_eq!(sinks.len(), 3);
    }

    #[test]
    fn parse_source_health_detects_low_volume_and_mute_for_default_source() {
        let json = r#"[
            {
                "name": "alsa_input.pci-0000_05_00.6.analog-stereo",
                "description": "Ryzen HD Audio Controller Analog Stereo",
                "mute": false,
                "volume": {
                    "front-left": {"value_percent": "35%"},
                    "front-right": {"value_percent": "35%"}
                },
                "active_port": "analog-input-front-mic"
            },
            {
                "name": "usb_mic",
                "description": "USB Mic",
                "mute": true,
                "volume": {"mono": {"value_percent": "100%"}}
            }
        ]"#;
        let h =
            parse_source_health_json(json, "", Some("alsa_input.pci-0000_05_00.6.analog-stereo"))
                .unwrap();
        assert_eq!(h.kind(), SourceHealthKind::LowVolume(35));
        assert_eq!(h.active_port.as_deref(), Some("analog-input-front-mic"));

        let muted = parse_source_health_json(json, "usb_mic", None).unwrap();
        assert_eq!(muted.kind(), SourceHealthKind::Muted);
    }
}
