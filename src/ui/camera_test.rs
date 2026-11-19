use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::AppConfig;
use crate::i18n;

pub const PREVIEW_WIDTH: i32 = 320;
pub const PREVIEW_HEIGHT: i32 = 180;
const PREVIEW_FPS: u32 = 15;
#[cfg(test)]
const PREVIEW_FRAME_BYTES: usize = (PREVIEW_WIDTH as usize) * (PREVIEW_HEIGHT as usize) * 3;

/// Parse `v4l2-ctl --list-devices` output into `(device_path, card_name)` pairs.
pub fn parse_v4l2_list_devices(output: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current_card: Option<String> = None;
    for raw in output.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            current_card = None;
            continue;
        }
        if !raw.starts_with('\t') && !raw.starts_with(' ') && trimmed.ends_with(':') {
            let label = trimmed.trim_end_matches(':');
            // Strip trailing bus info like " (usb-0000:01:00.0-5)" for cleaner UI display.
            let clean = label
                .rsplit_once(" (")
                .map(|(name, _)| name.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(label);
            current_card = Some(clean.to_string());
        } else if trimmed.starts_with("/dev/video") {
            let card = current_card
                .clone()
                .unwrap_or_else(|| trimmed.to_string());
            out.push((trimmed.to_string(), card));
        }
    }
    out
}

/// Check `v4l2-ctl -d <dev> --info` output for actual video capture capability
/// (filters out V4L2 metadata-only nodes such as `/dev/video1`).
pub fn v4l2_info_supports_capture(info: &str) -> bool {
    let mut in_device_caps = false;
    let mut saw_device_caps = false;
    for line in info.lines() {
        let t = line.trim();
        if t.starts_with("Device Caps") {
            in_device_caps = true;
            saw_device_caps = true;
            continue;
        }
        if in_device_caps {
            if !line.starts_with('\t') && !line.starts_with("    ") && t.contains(':') {
                in_device_caps = false;
                continue;
            }
            if t == "Video Capture" || t == "Video Capture Multiplanar" {
                return true;
            }
        }
    }
    if saw_device_caps {
        return false;
    }
    info.lines()
        .map(str::trim)
        .any(|t| t == "Video Capture" || t == "Video Capture Multiplanar")
}

fn device_supports_capture(path: &str) -> bool {
    let Ok(out) = Command::new("v4l2-ctl")
        .args(["-d", path, "--info"])
        .output()
    else {
        return true;
    };
    if !out.status.success() {
        return false;
    }
    v4l2_info_supports_capture(&String::from_utf8_lossy(&out.stdout))
}

/// Enumerate real V4L2 video capture cameras as `(device_path, display_label)`.
/// First entry is always `("", "Auto")`.
pub fn list_v4l2_cameras() -> Vec<(String, String)> {
    let mut result = vec![(String::new(), i18n::t("camera_device_auto"))];
    let mut candidates = Vec::new();
    if let Ok(out) = Command::new("v4l2-ctl").arg("--list-devices").output()
        && out.status.success()
    {
        candidates = parse_v4l2_list_devices(&String::from_utf8_lossy(&out.stdout));
    }
    if candidates.is_empty()
        && let Ok(entries) = std::fs::read_dir("/sys/class/video4linux")
    {
        let mut sys_devs: Vec<(String, String)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with("video") {
                    return None;
                }
                let dev_path = format!("/dev/{name}");
                let label = std::fs::read_to_string(e.path().join("name"))
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| dev_path.clone());
                Some((dev_path, label))
            })
            .collect();
        sys_devs.sort_by(|a, b| a.0.cmp(&b.0));
        candidates = sys_devs;
    }
    for (dev_path, card) in candidates {
        if device_supports_capture(&dev_path) {
            result.push((dev_path.clone(), format!("{card} ({dev_path})")));
        }
    }
    result
}

/// Resolve configured `video_input` into a concrete `/dev/videoN` path if any exists.
pub fn resolve_camera_device(configured: &str) -> Option<String> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() && trimmed != "default" {
        return Some(trimmed.to_string());
    }
    list_v4l2_cameras()
        .into_iter()
        .find(|(path, _)| !path.is_empty())
        .map(|(path, _)| path)
}

fn spawn_camera_preview_ffmpeg(
    device: &str,
    width: i32,
    height: i32,
    mirror: bool,
    explicit_size: bool,
) -> std::io::Result<Child> {
    let vf = if mirror {
        format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,hflip"
        )
    } else {
        format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
        )
    };
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "v4l2".to_string(),
        "-framerate".to_string(),
        PREVIEW_FPS.to_string(),
    ];
    if explicit_size {
        args.push("-video_size".to_string());
        args.push("640x360".to_string());
    }
    args.extend([
        "-i".to_string(),
        device.to_string(),
        "-vf".to_string(),
        vf,
        "-pix_fmt".to_string(),
        "rgb24".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "pipe:1".to_string(),
    ]);
    Command::new("ffmpeg")
        .args(&args)
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

enum PreviewMsg {
    Frame {
        generation: u64,
        rgb: Vec<u8>,
        fps: f64,
    },
    Stopped {
        generation: u64,
        error: Option<String>,
    },
}

/// Reusable live V4L2 camera preview handle (used by Settings camera test
/// and by the Video Call pre-call confirmation dialog).
pub struct CameraPreviewHandle {
    generation: Cell<u64>,
    running: Cell<bool>,
    active_stop: RefCell<Option<Arc<AtomicBool>>>,
    child: Arc<Mutex<Option<Child>>>,
    tx: async_channel::Sender<PreviewMsg>,
    width: i32,
    height: i32,
}

impl CameraPreviewHandle {
    pub fn new(
        picture: gtk::Picture,
        width: i32,
        height: i32,
        on_status: impl Fn(Option<f64>, Option<String>, bool) + 'static,
    ) -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded::<PreviewMsg>();
        let handle = Rc::new(Self {
            generation: Cell::new(0),
            running: Cell::new(false),
            active_stop: RefCell::new(None),
            child: Arc::new(Mutex::new(None)),
            tx,
            width,
            height,
        });

        let weak = Rc::downgrade(&handle);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(msg) = rx.recv().await {
                let Some(ctrl) = weak.upgrade() else {
                    break;
                };
                match msg {
                    PreviewMsg::Frame {
                        generation,
                        rgb,
                        fps,
                    } => {
                        if generation != ctrl.generation.get() || !ctrl.running.get() {
                            continue;
                        }
                        let stride = (ctrl.width as usize) * 3;
                        let bytes = glib::Bytes::from_owned(rgb);
                        let tex = gdk::MemoryTexture::new(
                            ctrl.width,
                            ctrl.height,
                            gdk::MemoryFormat::R8g8b8,
                            &bytes,
                            stride,
                        );
                        picture.set_paintable(Some(&tex));
                        on_status(Some(fps), None, true);
                    }
                    PreviewMsg::Stopped { generation, error } => {
                        if generation != ctrl.generation.get() {
                            continue;
                        }
                        ctrl.running.set(false);
                        picture.set_paintable(None::<&gdk::Paintable>);
                        on_status(None, error, false);
                    }
                }
            }
        });

        handle
    }

    pub fn is_running(&self) -> bool {
        self.running.get()
    }

    pub fn start(&self, configured_device: &str, mirror: bool) {
        self.stop();
        let Some(device) = resolve_camera_device(configured_device) else {
            let generation = self.generation.get().wrapping_add(1);
            self.generation.set(generation);
            let _ = self.tx.send_blocking(PreviewMsg::Stopped {
                generation,
                error: Some(i18n::t("camera_not_found")),
            });
            return;
        };

        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let stop_flag = Arc::new(AtomicBool::new(false));
        *self.active_stop.borrow_mut() = Some(stop_flag.clone());
        self.running.set(true);

        let child_slot = self.child.clone();
        let tx = self.tx.clone();
        let width = self.width;
        let height = self.height;
        let frame_bytes = (width as usize) * (height as usize) * 3;

        std::thread::spawn(move || {
            let mut last_err: Option<String> = None;
            let mut got_any_frame = false;

            for explicit_size in [true, false] {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                let mut child =
                    match spawn_camera_preview_ffmpeg(&device, width, height, mirror, explicit_size)
                    {
                        Ok(c) => c,
                        Err(e) => {
                            last_err = Some(e.to_string());
                            break;
                        }
                    };
                let Some(mut stdout) = child.stdout.take() else {
                    let _ = child.kill();
                    let _ = child.wait();
                    last_err = Some("camera stdout unavailable".into());
                    break;
                };
                let stderr = child.stderr.take();
                if let Ok(mut slot) = child_slot.lock() {
                    *slot = Some(child);
                }

                let mut buf = vec![0u8; frame_bytes];
                let mut frames_in_window: u32 = 0;
                let mut window_start = Instant::now();
                let mut current_fps = 0.0f64;

                loop {
                    if stop_flag.load(Ordering::SeqCst) {
                        break;
                    }
                    let mut filled = 0;
                    while filled < frame_bytes && !stop_flag.load(Ordering::SeqCst) {
                        match stdout.read(&mut buf[filled..]) {
                            Ok(0) => break,
                            Ok(n) => filled += n,
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(e) => {
                                if !stop_flag.load(Ordering::SeqCst) {
                                    last_err = Some(e.to_string());
                                }
                                break;
                            }
                        }
                    }
                    if filled < frame_bytes || stop_flag.load(Ordering::SeqCst) {
                        break;
                    }
                    got_any_frame = true;
                    frames_in_window += 1;
                    let elapsed = window_start.elapsed();
                    if elapsed >= Duration::from_millis(500) {
                        current_fps = frames_in_window as f64 / elapsed.as_secs_f64();
                        frames_in_window = 0;
                        window_start = Instant::now();
                    }
                    if tx
                        .send_blocking(PreviewMsg::Frame {
                            generation,
                            rgb: buf.clone(),
                            fps: if current_fps > 0.0 {
                                current_fps
                            } else {
                                PREVIEW_FPS as f64
                            },
                        })
                        .is_err()
                    {
                        break;
                    }
                }

                kill_child_slot(&child_slot);
                if let Some(mut se) = stderr {
                    let mut err_str = String::new();
                    let _ = se.read_to_string(&mut err_str);
                    let trimmed = err_str.trim();
                    if !trimmed.is_empty() && !got_any_frame {
                        last_err = Some(trimmed.chars().take(180).collect());
                    }
                }
                if got_any_frame || stop_flag.load(Ordering::SeqCst) {
                    break;
                }
            }

            let _ = tx.send_blocking(PreviewMsg::Stopped {
                generation,
                error: if stop_flag.load(Ordering::SeqCst) {
                    None
                } else {
                    last_err.or_else(|| {
                        if got_any_frame {
                            None
                        } else {
                            Some(i18n::t("camera_open_failed"))
                        }
                    })
                },
            });
        });
    }

    pub fn stop(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.running.set(false);
        if let Some(flag) = self.active_stop.borrow_mut().take() {
            flag.store(true, Ordering::SeqCst);
        }
        kill_child_slot(&self.child);
    }
}

impl Drop for CameraPreviewHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct CameraTestController {
    preview: Rc<CameraPreviewHandle>,
    config: Rc<RefCell<AppConfig>>,
    toggle_btn: gtk::Button,
    status_label: gtk::Label,
    placeholder_box: gtk::Box,
}

impl CameraTestController {
    pub fn on_device_changed(&self) {
        if self.preview.is_running() {
            let (dev, mirror) = {
                let c = self.config.borrow();
                (c.video_input.clone(), c.call_video_mirror)
            };
            self.preview.start(&dev, mirror);
        } else {
            self.refresh_idle_status();
        }
    }

    pub fn stop(&self) {
        self.preview.stop();
        self.toggle_btn.set_label(&i18n::t("camera_test_start"));
        self.toggle_btn.remove_css_class("destructive-action");
        self.toggle_btn.add_css_class("suggested-action");
        self.placeholder_box.set_visible(true);
        self.refresh_idle_status();
    }

    fn refresh_idle_status(&self) {
        let configured = self.config.borrow().video_input.clone();
        if let Some(dev) = resolve_camera_device(&configured) {
            self.status_label
                .set_text(&i18n::tf("camera_test_ready", &[("device", &dev)]));
        } else {
            self.status_label.set_text(&i18n::t("camera_not_found"));
        }
    }
}

pub fn attach_camera_test_ui(
    group: &libadwaita::PreferencesGroup,
    win: &libadwaita::PreferencesWindow,
    config: Rc<RefCell<AppConfig>>,
    data_dir: PathBuf,
) -> Rc<CameraTestController> {
    let mirror_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("camera_mirror"))
        .subtitle(i18n::t("camera_mirror_subtitle"))
        .active(config.borrow().call_video_mirror)
        .build();
    group.add(&mirror_row);

    let test_row = libadwaita::PreferencesRow::builder()
        .activatable(false)
        .selectable(false)
        .build();

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(14)
        .margin_end(14)
        .build();

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let title_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    let title = gtk::Label::builder()
        .label(i18n::t("camera_test_title"))
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let status_label = gtk::Label::builder()
        .label(i18n::t("camera_test_idle"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    title_box.append(&title);
    title_box.append(&status_label);

    let toggle_btn = gtk::Button::builder()
        .label(i18n::t("camera_test_start"))
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    header.append(&title_box);
    header.append(&toggle_btn);
    outer.append(&header);

    let preview_overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .css_classes(["line-camera-preview"])
        .build();
    preview_overlay.set_size_request(PREVIEW_WIDTH, PREVIEW_HEIGHT);

    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .build();
    picture.set_size_request(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    preview_overlay.set_child(Some(&picture));

    let placeholder_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let cam_icon = gtk::Image::builder()
        .icon_name("camera-video-symbolic")
        .pixel_size(36)
        .css_classes(["dim-label"])
        .build();
    let cam_hint = gtk::Label::builder()
        .label(i18n::t("camera_test_hint"))
        .css_classes(["dim-label", "caption"])
        .build();
    placeholder_box.append(&cam_icon);
    placeholder_box.append(&cam_hint);
    preview_overlay.add_overlay(&placeholder_box);
    outer.append(&preview_overlay);

    test_row.set_child(Some(&outer));
    group.add(&test_row);

    let status_cb = status_label.clone();
    let btn_cb = toggle_btn.clone();
    let placeholder_cb = placeholder_box.clone();
    let preview = CameraPreviewHandle::new(
        picture,
        PREVIEW_WIDTH,
        PREVIEW_HEIGHT,
        move |fps, err, active| {
            if active {
                placeholder_cb.set_visible(false);
                if let Some(f) = fps {
                    status_cb.set_text(&i18n::tf(
                        "camera_test_streaming",
                        &[("fps", &format!("{f:.0}"))],
                    ));
                }
            } else {
                placeholder_cb.set_visible(true);
                btn_cb.set_label(&i18n::t("camera_test_start"));
                btn_cb.remove_css_class("destructive-action");
                btn_cb.add_css_class("suggested-action");
                if let Some(e) = err {
                    status_cb.set_text(&i18n::tf("camera_test_error", &[("error", &e)]));
                } else {
                    status_cb.set_text(&i18n::t("camera_test_idle"));
                }
            }
        },
    );

    let ctrl = Rc::new(CameraTestController {
        preview,
        config: config.clone(),
        toggle_btn: toggle_btn.clone(),
        status_label: status_label.clone(),
        placeholder_box,
    });
    ctrl.refresh_idle_status();

    {
        let ctrl = ctrl.clone();
        toggle_btn.connect_clicked(move |btn| {
            if ctrl.preview.is_running() {
                ctrl.stop();
            } else {
                let (dev, mirror) = {
                    let c = ctrl.config.borrow();
                    (c.video_input.clone(), c.call_video_mirror)
                };
                btn.set_label(&i18n::t("camera_test_stop"));
                btn.remove_css_class("suggested-action");
                btn.add_css_class("destructive-action");
                ctrl.status_label
                    .set_text(&i18n::t("camera_test_starting"));
                ctrl.preview.start(&dev, mirror);
            }
        });
    }

    {
        let ctrl = ctrl.clone();
        let cfg = config;
        mirror_row.connect_active_notify(move |row| {
            cfg.borrow_mut().call_video_mirror = row.is_active();
            cfg.borrow().save(&data_dir);
            ctrl.on_device_changed();
        });
    }

    {
        let ctrl = ctrl.clone();
        win.connect_close_request(move |_| {
            ctrl.stop();
            glib::Propagation::Proceed
        });
    }

    ctrl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v4l2_list_devices_extracts_cards_and_video_nodes() {
        let sample = "\
HD Webcam C615 (usb-0000:01:00.0-5):
\t/dev/video0
\t/dev/video1
\t/dev/media0

Integrated Camera (usb-0000:00:14.0-2):
\t/dev/video2
";
        let parsed = parse_v4l2_list_devices(sample);
        assert_eq!(
            parsed,
            vec![
                ("/dev/video0".into(), "HD Webcam C615".into()),
                ("/dev/video1".into(), "HD Webcam C615".into()),
                ("/dev/video2".into(), "Integrated Camera".into()),
            ]
        );
    }

    #[test]
    fn v4l2_info_distinguishes_capture_node_from_metadata_node() {
        let capture = "\
Driver Info:
\tCapabilities     : 0x84a00001
\t\tVideo Capture
\t\tMetadata Capture
\tDevice Caps      : 0x04200001
\t\tVideo Capture
\t\tStreaming
";
        let metadata = "\
Driver Info:
\tCapabilities     : 0x84a00001
\t\tVideo Capture
\t\tMetadata Capture
\tDevice Caps      : 0x04a00000
\t\tMetadata Capture
\t\tStreaming
";
        assert!(v4l2_info_supports_capture(capture));
        assert!(!v4l2_info_supports_capture(metadata));
    }

    #[test]
    fn preview_frame_bytes_matches_rgb24_dimensions() {
        assert_eq!(PREVIEW_FRAME_BYTES, 320 * 180 * 3);
    }
}
