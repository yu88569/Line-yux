use super::*;
use anyhow::{Context, bail};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CaptureOutcome {
    Captured(PathBuf),
    Cancelled,
}

#[derive(Clone)]
struct CaptureTools {
    selector: PathBuf,
    screenshot: PathBuf,
}

impl Default for CaptureTools {
    fn default() -> Self {
        Self {
            selector: "slurp".into(),
            screenshot: "grim".into(),
        }
    }
}

fn valid_geometry(value: &str) -> bool {
    let Some((origin, size)) = value.split_once(' ') else {
        return false;
    };
    let Some((x, y)) = origin.split_once(',') else {
        return false;
    };
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    x.parse::<i32>().is_ok()
        && y.parse::<i32>().is_ok()
        && width.parse::<u32>().is_ok_and(|v| v > 0)
        && height.parse::<u32>().is_ok_and(|v| v > 0)
}

fn capture(data_dir: &Path, tools: &CaptureTools) -> anyhow::Result<CaptureOutcome> {
    let selection = Command::new(&tools.selector)
        .output()
        .context(crate::i18n::t("capture_install_tools"))?;
    if !selection.status.success() {
        if selection.status.code() == Some(1)
            && (selection.stderr.is_empty() || selection.stderr == b"selection cancelled\n")
        {
            return Ok(CaptureOutcome::Cancelled);
        }
        bail!("{}", crate::i18n::t("capture_select_failed"));
    }
    let geometry = std::str::from_utf8(&selection.stdout)?.trim();
    if !valid_geometry(geometry) {
        bail!("{}", crate::i18n::t("capture_select_failed"));
    }
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let dir = data_dir.join("cache/captures");
    std::fs::create_dir_all(&dir)?;
    let private = dir.join(format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&private)?;
    let path = private.join("capture.png");
    let result = (|| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let output = Command::new(&tools.screenshot)
            .arg("-g")
            .arg(geometry)
            .arg(&path)
            .output()
            .context(crate::i18n::t("capture_install_tools"))?;
        if !output.status.success() {
            bail!("{}", crate::i18n::t("capture_grim_failed"));
        }
        image::ImageReader::open(&path)?.decode()?;
        Ok(CaptureOutcome::Captured(path))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(private);
    }
    result
}

pub(super) fn start(state: &AppState) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        toast(state, &crate::i18n::t("capture_wayland_only"));
        return;
    }
    start_with_tools(state, CaptureTools::default());
}

fn start_with_tools(state: &AppState, tools: CaptureTools) {
    if !state.capture_btn.is_sensitive() {
        return;
    }
    state.capture_btn.set_sensitive(false);
    let s = state.clone();
    glib::spawn_future_local(async move {
        let dir = s.data_dir.clone();
        let result = gio::spawn_blocking(move || capture(&dir, &tools)).await;
        s.capture_btn.set_sensitive(true);
        match result {
            Ok(Ok(CaptureOutcome::Captured(path))) => {
                open_media_viewer(&s, &path.to_string_lossy(), "IMAGE", "capture.png");
            }
            Ok(Ok(CaptureOutcome::Cancelled)) => {
                s.status.set_text(&crate::i18n::t("capture_cancelled"));
            }
            outcome => {
                let error = match outcome {
                    Ok(Err(error)) => error.to_string(),
                    _ => crate::i18n::t("capture_failed"),
                };
                toast(
                    &s,
                    &crate::i18n::tf("capture_failed_detail", &[("error", &error)]),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt;

    fn script(path: &Path, content: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{content}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn fixture(name: &str) -> (PathBuf, CaptureTools) {
        let dir = std::env::temp_dir().join(format!("line-capture-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tools = CaptureTools {
            selector: dir.join("selector"),
            screenshot: dir.join("screenshot"),
        };
        (dir, tools)
    }

    fn success_scripts(dir: &Path, tools: &CaptureTools) {
        script(&tools.selector, "printf '%s\\n' '-10,20 240x240'");
        script(
            &tools.screenshot,
            &format!(
                "[ \"$1\" = '-g' ] && [ \"$2\" = '-10,20 240x240' ] || exit 2\ncp '{}' \"$3\"",
                dir.join("source.png").display()
            ),
        );
    }

    #[test]
    fn capture_commands_outcomes_geometry_and_private_output() {
        let (dir, tools) = fixture("commands");
        super::super::qr::tests::qr_image("https://example.com")
            .save(dir.join("source.png"))
            .unwrap();
        success_scripts(&dir, &tools);
        let CaptureOutcome::Captured(path) = capture(&dir, &tools).unwrap() else {
            panic!()
        };
        assert!(path.is_file());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(super::super::qr::decode(&path).unwrap().len(), 1);
        script(&tools.selector, "exit 1");
        assert_eq!(capture(&dir, &tools).unwrap(), CaptureOutcome::Cancelled);
        script(
            &tools.selector,
            "printf 'selection cancelled\\n' >&2\nexit 1",
        );
        assert_eq!(capture(&dir, &tools).unwrap(), CaptureOutcome::Cancelled);
        script(&tools.selector, "echo unsupported >&2\nexit 1");
        assert!(capture(&dir, &tools).is_err());
        script(&tools.selector, "printf '%s\\n' '0,0 1x1;touch injected'");
        assert!(capture(&dir, &tools).is_err());
        assert!(!dir.join("injected").exists());
        success_scripts(&dir, &tools);
        script(&tools.screenshot, "exit 2");
        assert!(capture(&dir, &tools).is_err());
        let children: Vec<_> = std::fs::read_dir(dir.join("cache/captures"))
            .unwrap()
            .collect();
        assert_eq!(
            children.len(),
            1,
            "failed captures remove their private directory"
        );
        script(&tools.screenshot, "printf invalid > \"$3\"");
        assert!(capture(&dir, &tools).is_err());
        let missing = CaptureTools {
            selector: dir.join("missing"),
            ..tools
        };
        assert!(
            capture(&dir, &missing)
                .unwrap_err()
                .to_string()
                .contains("capture_install_tools")
        );
        for invalid in [
            "",
            "0 1x1",
            "0,0 0x1",
            "0,0 1x0",
            "0,0 1x1 extra",
            "$(id),0 1x1",
        ] {
            assert!(!valid_geometry(invalid));
        }
        assert!(valid_geometry("-10,-200 300x400"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn pump_until(check: impl Fn() -> bool) {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !check() {
            assert!(
                std::time::Instant::now() < deadline,
                "GTK condition timed out"
            );
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn windows() -> Vec<gtk::Window> {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .collect()
    }

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut items = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            items.extend(descendants(&current));
        }
        items
    }

    fn qr_button(win: &gtk::Window) -> gtk::Button {
        descendants(win.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.tooltip_text().as_deref() == Some(crate::i18n::t("qr_read").as_str()))
            .unwrap()
    }

    fn qr_dialog() -> Option<gtk::Window> {
        windows().into_iter().find(|w| {
            w.is_visible() && w.title().as_deref() == Some(crate::i18n::t("qr_results").as_str())
        })
    }

    #[test]
    #[ignore = "requires a native GTK display"]
    fn gtk_capture_and_qr_existing_viewer_lifetimes() {
        libadwaita::init().unwrap();
        let (dir, tools) = fixture("gtk");
        let url = "https://example.com/captured?test=1";
        super::super::qr::tests::qr_image(url)
            .save(dir.join("source.png"))
            .unwrap();
        success_scripts(&dir, &tools);
        std::fs::create_dir_all(dir.join("protocol")).unwrap();
        script(
            &dir.join("protocol/line-gtk-protocol"),
            "while read -r line; do :; done",
        );
        let data = dir.join("data");
        AppConfig {
            tray_enabled: false,
            animations: false,
            notifications: false,
            ..AppConfig::default()
        }
        .save(&data);
        let app = Application::builder()
            .application_id("dev.linegtk.CaptureQrFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let sidecar = Rc::new(Sidecar::spawn(&dir, &data).unwrap());
        let (state, _) = build_state(
            &app,
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            data,
            sidecar,
        );
        assert_eq!(
            state.mic_btn.next_sibling(),
            Some(state.capture_btn.clone().upcast())
        );
        state.stack.set_visible_child_name("shell");
        state.window.present();
        pump_until(|| state.window.is_mapped());
        let s = state.clone();
        let injected = tools.clone();
        state
            .capture_btn
            .connect_clicked(move |_| start_with_tools(&s, injected.clone()));
        let visibility_changes = Rc::new(Cell::new(0));
        let changes = visibility_changes.clone();
        state.window.connect_visible_notify(move |_| {
            changes.set(changes.get() + 1);
        });
        state.capture_btn.emit_clicked();
        assert!(!state.capture_btn.is_sensitive());
        assert!(state.window.is_visible());
        state.capture_btn.emit_clicked();
        pump_until(|| state.capture_btn.is_sensitive());
        assert_eq!(visibility_changes.get(), 0);
        assert!(state.window.is_visible());
        let viewer = windows()
            .into_iter()
            .find(|w| w.has_css_class("line-media-viewer") && w.is_visible())
            .unwrap();
        assert!(!viewer.is_modal());
        assert!(viewer.transient_for().is_none());
        assert_eq!(viewer.application().as_ref(), Some(app.upcast_ref()));
        state.window.present();
        assert!(state.composer.grab_focus());
        pump_until(|| {
            gtk::prelude::GtkWindowExt::focus(&state.window)
                .is_some_and(|focus| focus == state.composer || focus.is_ancestor(&state.composer))
        });
        let mut position = 0;
        state
            .composer
            .insert_text("chat while viewing", &mut position);
        assert_eq!(state.composer.text(), "chat while viewing");
        assert!(viewer.is_visible());
        assert!(state.window.is_visible());
        let children = descendants(viewer.upcast_ref());
        assert!(children.iter().any(|w| w.is::<gtk::DrawingArea>()));
        assert!(
            children
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Button>())
                .any(|b| b.icon_name().as_deref() == Some("mail-send-symbolic"))
        );
        assert_eq!(
            std::fs::read_dir(dir.join("data/cache/captures"))
                .unwrap()
                .count(),
            1
        );
        let read = qr_button(&viewer);
        read.emit_clicked();
        assert!(!read.is_sensitive());
        pump_until(|| qr_dialog().is_some());
        assert!(read.is_sensitive());
        let result = qr_dialog().unwrap();
        let children = descendants(result.upcast_ref());
        let label = children
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Label>())
            .find(|l| l.text() == url)
            .unwrap();
        assert!(label.is_selectable());
        assert!(!label.uses_markup());
        let copy = children
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Button>())
            .find(|b| b.label().as_deref() == Some(crate::i18n::t("qr_copy").as_str()))
            .unwrap();
        assert!(
            children
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Button>())
                .any(|b| b.label().as_deref() == Some(crate::i18n::t("qr_open").as_str()))
        );
        copy.emit_clicked();
        let copied = glib::MainContext::default()
            .block_on(copy.clipboard().read_text_future())
            .unwrap()
            .unwrap();
        assert_eq!(copied, url);
        let unrelated = gtk::Window::new();
        unrelated.present();
        let hidden_viewer = MediaViewer::new(&state);
        for window in [&viewer, &result, &unrelated, &hidden_viewer.window] {
            let changes = visibility_changes.clone();
            window.connect_visible_notify(move |_| {
                changes.set(changes.get() + 1);
            });
        }
        script(
            &tools.selector,
            "printf 'selection cancelled\\n' >&2\nexit 1",
        );
        state.capture_btn.emit_clicked();
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(unrelated.is_visible());
        pump_until(|| state.capture_btn.is_sensitive());
        assert_eq!(visibility_changes.get(), 0);
        assert!(state.window.is_visible());
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(!hidden_viewer.window.is_visible());
        assert_eq!(state.status.text(), crate::i18n::t("capture_cancelled"));
        success_scripts(&dir, &tools);
        script(&tools.screenshot, "exit 2");
        state.capture_btn.emit_clicked();
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(unrelated.is_visible());
        pump_until(|| state.capture_btn.is_sensitive());
        assert_eq!(visibility_changes.get(), 0);
        assert!(state.window.is_visible());
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(!hidden_viewer.window.is_visible());
        success_scripts(&dir, &tools);
        state.capture_btn.emit_clicked();
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(unrelated.is_visible());
        pump_until(|| state.capture_btn.is_sensitive());
        assert_eq!(visibility_changes.get(), 0);
        assert!(state.window.is_visible());
        assert!(viewer.is_visible());
        assert!(result.is_visible());
        assert!(!hidden_viewer.window.is_visible());
        let captured = windows()
            .into_iter()
            .find(|w| *w != viewer && w.has_css_class("line-media-viewer") && w.is_visible())
            .unwrap();
        assert_eq!(captured.application().as_ref(), Some(app.upcast_ref()));
        captured.close();
        result.close();
        viewer.close();
        assert!(state.window.is_visible());
        hidden_viewer.window.close();
        unrelated.close();

        let viewer = MediaViewer::new(&state);
        let path = dir.join("source.png");
        let path = path.to_str().unwrap();
        viewer.show(&state, path, "IMAGE", "qr.png", None);
        viewer.window.present();
        pump_until(|| viewer.window.is_mapped());
        let old = qr_button(&viewer.window);
        old.emit_clicked();
        viewer.show(&state, path, "IMAGE", "replacement.png", None);
        pump_until(|| old.is_sensitive());
        assert!(
            qr_dialog().is_none(),
            "late old-image result must be discarded"
        );
        let old = qr_button(&viewer.window);
        old.emit_clicked();
        viewer.window.close();
        pump_until(|| old.is_sensitive());
        assert!(
            qr_dialog().is_none(),
            "late closed-viewer result must be discarded"
        );

        let blank = dir.join("blank.png");
        image::GrayImage::from_pixel(240, 240, image::Luma([255]))
            .save(&blank)
            .unwrap();
        let viewer = MediaViewer::new(&state);
        viewer.show(&state, blank.to_str().unwrap(), "IMAGE", "blank.png", None);
        viewer.window.present();
        pump_until(|| viewer.window.is_mapped());
        qr_button(&viewer.window).emit_clicked();
        pump_until(|| qr_dialog().is_some());
        let result = qr_dialog().unwrap();
        assert!(
            descendants(result.upcast_ref())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Label>())
                .any(|l| l.text() == crate::i18n::t("qr_not_found"))
        );
        viewer.show(&state, path, "IMAGE", "changed.png", None);
        pump_until(|| !result.is_visible());
        let plain = dir.join("plain.png");
        let payload = "<b>สวัสดี</b> & text";
        super::super::qr::tests::qr_image(payload)
            .save(&plain)
            .unwrap();
        viewer.show(&state, plain.to_str().unwrap(), "IMAGE", "plain.png", None);
        qr_button(&viewer.window).emit_clicked();
        pump_until(|| qr_dialog().is_some());
        let result = qr_dialog().unwrap();
        let children = descendants(result.upcast_ref());
        let label = children
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Label>())
            .find(|l| l.text() == payload)
            .unwrap();
        assert!(label.is_selectable());
        assert!(!label.uses_markup());
        assert!(
            !children
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Button>())
                .any(|b| b.label().as_deref() == Some(crate::i18n::t("qr_open").as_str()))
        );
        result.close();
        std::fs::remove_file(&plain).unwrap();
        qr_button(&viewer.window).emit_clicked();
        pump_until(|| qr_dialog().is_some());
        let result = qr_dialog().unwrap();
        assert!(
            descendants(result.upcast_ref())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Label>())
                .any(|l| l.text() == crate::i18n::t("qr_decode_failed"))
        );
        result.close();
        viewer.window.close();
        state.window.close();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
