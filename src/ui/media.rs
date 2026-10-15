use super::*;

pub(super) fn meaningful_file_name(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("[File]") {
        None
    } else {
        Some(name)
    }
}

pub(super) fn original_file_name(msg: &MessageInfo) -> Option<String> {
    msg.file_name
        .as_deref()
        .and_then(meaningful_file_name)
        .or_else(|| meaningful_file_name(&msg.text))
        .map(str::to_string)
        .or_else(|| {
            let name = std::path::Path::new(msg.file_path.as_deref()?)
                .file_name()?
                .to_str()?;
            if msg.id.is_empty() {
                return None;
            }
            let suffix = name.strip_prefix(&msg.id)?;
            let original = suffix
                .strip_prefix('.')
                .or_else(|| suffix.strip_prefix('_'))?;
            meaningful_file_name(original)
                .filter(|name| std::path::Path::new(name).extension().is_some())
                .map(str::to_string)
        })
}

pub(super) fn suggest_media_name(msg: &MessageInfo) -> String {
    if msg.content_type.eq_ignore_ascii_case("FILE") {
        return original_file_name(msg).unwrap_or_else(|| {
            let extension = msg
                .file_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).extension())
                .and_then(|extension| extension.to_str())
                .map(|extension| format!(".{extension}"))
                .unwrap_or_default();
            format!("line-{}{extension}", msg.id)
        });
    }
    if let Some(n) = msg.file_name.as_deref().and_then(meaningful_file_name) {
        return n.to_string();
    }
    match msg.content_type.to_ascii_uppercase().as_str() {
        "IMAGE" => format!("{}.jpg", msg.id),
        "VIDEO" => format!("{}.mp4", msg.id),
        "AUDIO" => format!("{}.m4a", msg.id),
        _ => format!("line-{}", msg.id),
    }
}

pub(super) struct FilePresentation {
    pub name: String,
    pub detail: String,
    pub icon: &'static str,
    pub action: &'static str,
}

impl FilePresentation {
    pub fn from_message(msg: &MessageInfo) -> Self {
        let original = original_file_name(msg);
        let extension = original
            .as_deref()
            .map(file_extension)
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| file_extension(msg.file_path.as_deref().unwrap_or("")));
        let name = original.unwrap_or_else(|| crate::i18n::t("file_name_unknown"));
        let (label, icon) = match extension.as_str() {
            "doc" | "docx" | "docm" | "dot" | "dotx" | "dotm" | "odt" | "ott" | "rtf" => {
                ("Word", "x-office-document-symbolic")
            }
            "xls" | "xlsx" | "xlsm" | "xlsb" | "xlt" | "xltx" | "xltm" | "ods" | "ots" => {
                ("Excel", "x-office-spreadsheet-symbolic")
            }
            "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "ppsm" | "pot" | "potx" | "potm" | "odp"
            | "otp" => ("PowerPoint", "x-office-presentation-symbolic"),
            "pdf" => ("PDF", "application-pdf-symbolic"),
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "cab" => {
                ("Archive", "package-x-generic-symbolic")
            }
            _ => match viewer_kind_for("FILE", msg.file_path.as_deref().unwrap_or(""), &name) {
                ViewerKind::Image => ("Image", "image-x-generic-symbolic"),
                ViewerKind::Video => ("Video", "video-x-generic-symbolic"),
                ViewerKind::Audio => ("Audio", "audio-x-generic-symbolic"),
                ViewerKind::Text => ("Text", "text-x-generic-symbolic"),
                _ => ("", "text-x-generic-symbolic"),
            },
        };
        let localized = match label {
            "Archive" => crate::i18n::t("file_type_archive"),
            "Image" => crate::i18n::t("file_type_image"),
            "Video" => crate::i18n::t("file_type_video"),
            "Audio" => crate::i18n::t("file_type_audio"),
            "Text" => crate::i18n::t("file_type_text"),
            _ => label.to_string(),
        };
        let mut detail = if label.is_empty() {
            if extension.is_empty() {
                crate::i18n::t("file_type_unknown")
            } else {
                extension.to_ascii_uppercase()
            }
        } else if extension.is_empty() || label.eq_ignore_ascii_case(&extension) {
            localized
        } else {
            format!("{localized} · {}", extension.to_ascii_uppercase())
        };
        let size = msg.file_size.or_else(|| {
            std::fs::metadata(msg.file_path.as_deref()?)
                .ok()
                .map(|m| m.len())
        });
        if let Some(size) = size {
            detail.push_str(&format!(" · {}", format_bytes(size)));
        }
        let action = if viewer_kind_for("FILE", msg.file_path.as_deref().unwrap_or(""), &name)
            == ViewerKind::Generic
        {
            "open_external"
        } else {
            "open_viewer"
        };
        Self {
            name,
            detail,
            icon,
            action,
        }
    }
}

fn file_extension(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub(super) fn is_thumb_media_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains(".thumb.") || lower.ends_with(".thumb.jpg") || lower.ends_with(".thumb.png")
}

pub(super) fn is_full_media_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains(".full.")
}

pub(super) fn image_looks_low_res(path: &str) -> bool {
    if is_full_media_path(path) {
        return false;
    }
    if is_thumb_media_path(path) {
        return true;
    }
    match gdk_pixbuf::Pixbuf::from_file(path) {
        Ok(pb) => pb.width() <= 512 && pb.height() <= 512,
        Err(_) => false,
    }
}

pub(super) fn full_image_candidate(path: &str) -> Option<String> {
    if is_full_media_path(path) && std::path::Path::new(path).exists() {
        return Some(path.to_string());
    }
    if !is_thumb_media_path(path)
        && std::path::Path::new(path).exists()
        && !image_looks_low_res(path)
    {
        return Some(path.to_string());
    }
    // Try sibling .full. next to a polluted preview or thumb.
    let p = std::path::Path::new(path);
    let parent = p.parent()?;
    let name = p.file_name()?.to_str()?;
    let id = name.split('.').next().filter(|s| !s.is_empty())?;
    for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
        let cand = parent.join(format!("{id}.full.{ext}"));
        if cand.is_file() {
            return Some(cand.to_string_lossy().to_string());
        }
    }
    if is_thumb_media_path(path) {
        for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
            let cand = parent.join(format!("{id}.{ext}"));
            if cand.is_file() && !image_looks_low_res(&cand.to_string_lossy()) {
                return Some(cand.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub(super) fn local_media_path(msg: &MessageInfo, for_viewer: bool) -> Option<String> {
    let ct = msg.content_type.to_ascii_uppercase();
    if ct == "AUDIO" {
        return msg
            .audio_path
            .as_ref()
            .or(msg.file_path.as_ref())
            .filter(|p| std::path::Path::new(p).exists())
            .cloned();
    }
    if ct == "VIDEO" {
        if let Some(p) = msg.file_path.as_ref().filter(|p| {
            std::path::Path::new(p).exists() && p.to_ascii_lowercase().ends_with(".mp4")
        }) {
            return Some(p.clone());
        }
        // Viewer needs full video; thumb alone is not enough.
        if for_viewer {
            return None;
        }
    }
    if let Some(p) = msg
        .file_path
        .as_ref()
        .filter(|p| std::path::Path::new(p).exists() && !is_thumb_media_path(p))
        && !(for_viewer && ct == "IMAGE" && image_looks_low_res(p))
    {
        return Some(p.clone());
    }
    if ct == "IMAGE" {
        if for_viewer {
            // Never open the chat thumbnail / OBS preview in the viewer.
            if let Some(p) = msg.file_path.as_ref().and_then(|p| full_image_candidate(p)) {
                return Some(p);
            }
            if let Some(p) = msg
                .image_path
                .as_ref()
                .and_then(|p| full_image_candidate(p))
            {
                return Some(p);
            }
            return None;
        }
        if let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
        {
            return Some(p.clone());
        }
    } else if !for_viewer
        && let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
    {
        return Some(p.clone());
    }
    None
}

pub(super) fn request_media_download(state: &AppState, msg: &MessageInfo, action: &str) {
    request_media_action(
        state,
        msg,
        PendingMediaAction::ViewOrSave(action.to_string()),
    );
}

pub(super) fn request_media_action(
    state: &AppState,
    msg: &MessageInfo,
    action: PendingMediaAction,
) {
    let source_chat = match &action {
        PendingMediaAction::ShareFile {
            source_chat_mid, ..
        } => Some(source_chat_mid.clone()),
        PendingMediaAction::ViewOrSave(_) => state.current_chat.borrow().clone(),
    };
    let Some(chat_mid) = source_chat else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if msg.id.is_empty() {
        toast(state, &crate::i18n::t("media_download_failed"));
        return;
    }
    // Prefer freshest paths from hydrate / download cache.
    let msg = state
        .media_msgs
        .borrow()
        .get(&msg.id)
        .cloned()
        .unwrap_or_else(|| msg.clone());
    let needs_full_media = msg.content_type.eq_ignore_ascii_case("IMAGE")
        || matches!(&action, PendingMediaAction::ViewOrSave(value) if value == "open_viewer");
    if let Some(path) = local_media_path(&msg, needs_full_media) {
        finish_downloaded_media_action(
            state,
            &action,
            &path,
            &suggest_media_name(&msg),
            &msg.content_type,
        );
        return;
    }
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    state.status.set_text(&crate::i18n::t("media_downloading"));
    match state.sidecar.download_media(&chat_mid, &msg.id) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::DownloadMedia {
                    message_id: msg.id.clone(),
                    action,
                    content_type: msg.content_type.clone(),
                    suggest_name: suggest_media_name(&msg),
                },
            );
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn finish_downloaded_media_action(
    state: &AppState,
    action: &PendingMediaAction,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    match action {
        PendingMediaAction::ViewOrSave(action) => {
            finish_media_action(state, action, path, suggest_name, content_type)
        }
        PendingMediaAction::ShareFile { chat_mid, .. } => {
            file_actions::send_shared_media(state, chat_mid, path, suggest_name, content_type)
        }
    }
}

pub(super) fn finish_media_action(
    state: &AppState,
    action: &str,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    let action = if content_type.eq_ignore_ascii_case("FILE")
        && matches!(action, "open_viewer" | "open_external")
    {
        if viewer_kind_for("FILE", path, suggest_name) == ViewerKind::Generic {
            "open_external"
        } else {
            "open_viewer"
        }
    } else {
        action
    };
    match action {
        "open_viewer" => open_media_viewer(state, path, content_type, suggest_name),
        "open_external" => match prepare_external_attachment(&state.data_dir, path, suggest_name) {
            Ok(prepared) => open_path_externally(&prepared.to_string_lossy()),
            Err(error) => toast(
                state,
                &crate::i18n::tf(
                    "media_download_failed_err",
                    &[("error", &error.to_string())],
                ),
            ),
        },
        "save_dialog" => save_media_as(state, path, suggest_name, content_type),
        "file_save" => save_media_to_default(state, path, suggest_name, content_type),
        "file_save_as" => show_media_save_dialog(state, path, suggest_name, content_type),
        "play_audio" => {
            if let Err(e) = play_audio_file(path, &state.config.borrow().audio_output) {
                toast(
                    state,
                    &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
                );
            }
        }
        _ => save_media_as(state, path, suggest_name, content_type),
    }
}

pub(super) fn copy_media_to_dest(state: &AppState, src: &str, dest: &std::path::Path) {
    match std::fs::copy(src, dest) {
        Ok(_) => {
            toast(
                state,
                &crate::i18n::tf("media_saved", &[("path", &dest.display().to_string())]),
            );
            state.status.set_text(&crate::i18n::t("media_saved_status"));
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn save_media_as(state: &AppState, path: &str, suggest_name: &str, content_type: &str) {
    if state.config.borrow().download_ask_every_time {
        show_media_save_dialog(state, path, suggest_name, content_type);
    } else {
        save_media_to_default(state, path, suggest_name, content_type);
    }
}

pub(super) fn save_media_to_default(
    state: &AppState,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    let dest_dir = state.config.borrow().download_dir_for(content_type);
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        );
        return;
    }
    let dest = downloads::unique_download_dest(&dest_dir, suggest_name);
    copy_media_to_dest(state, path, &dest);
}

pub(super) fn show_media_save_dialog(
    state: &AppState,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    let configured = state.config.borrow().download_dir_for(content_type);
    let dest_dir = if configured.is_dir() {
        configured
    } else {
        AppConfig::default().download_dir_for(content_type)
    };

    let filter = gtk::FileFilter::new();
    filter.set_name(Some("All files"));
    filter.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("media_save_as"))
        .modal(true)
        .initial_name(downloads::sanitize_filename(suggest_name))
        .filters(&filters)
        .default_filter(&filter)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(&dest_dir)));
    let s = state.clone();
    let src = path.to_string();
    dialog.save(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(dest) = file.path() else {
                toast(&s, &crate::i18n::t("media_download_failed"));
                return;
            };
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            copy_media_to_dest(&s, &src, &dest);
        },
    );
}

pub(super) fn wire_media_open_click(
    state: &AppState,
    pic: &gtk::Picture,
    msg: &MessageInfo,
    _kind: &str,
) {
    wire_media_open_click_widget(state, pic.upcast_ref::<gtk::Widget>(), msg);
}

#[derive(Clone)]
pub(super) struct MediaViewer {
    pub window: gtk::Window,
    copy_keys: Rc<RefCell<Option<gtk::EventControllerKey>>>,
}

impl MediaViewer {
    pub fn new(state: &AppState) -> Self {
        let window = gtk::Window::builder()
            .application(&state.app)
            .modal(false)
            .title(crate::i18n::t("media_viewer_title"))
            .default_width(1080)
            .default_height(760)
            .css_classes(["line-media-viewer"])
            .build();
        let controller = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
        Self {
            window,
            copy_keys: Rc::new(RefCell::new(None)),
        }
    }

    pub fn clear(&self) {
        if let Some(controller) = self.copy_keys.borrow_mut().take() {
            self.window.remove_controller(&controller);
        }
        self.window.set_child(None::<&gtk::Widget>);
    }

    pub fn show(
        &self,
        state: &AppState,
        path: &str,
        content_type: &str,
        suggest_name: &str,
        navigation: Option<&gtk::Box>,
    ) {
        self.clear();
        let win = &self.window;
        let kind = viewer_kind_for(content_type, path, suggest_name);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["line-media-viewer-root"])
            .build();

        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .css_classes(["line-media-viewer-bar"])
            .build();
        let title = gtk::Label::builder()
            .label(suggest_name)
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();

        let open_ext = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(crate::i18n::t("media_open_externally"))
            .css_classes(["flat", "circular"])
            .build();
        let dl = gtk::Button::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text(crate::i18n::t("media_download"))
            .css_classes(["flat", "circular"])
            .build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(crate::i18n::t("media_close"))
            .css_classes(["flat", "circular"])
            .build();
        bar.append(&title);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .vexpand(true)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .css_classes(["line-media-viewer-body"])
            .build();

        match kind {
            ViewerKind::Video => {
                append_gpu_video_viewer(&body, path);
            }
            ViewerKind::Image => {
                *self.copy_keys.borrow_mut() = append_image_viewer(state, &bar, &body, path, win);
            }
            ViewerKind::Pdf => {
                append_pdf_viewer(&body, path, suggest_name);
            }
            ViewerKind::Text => {
                append_text_viewer(&body, path);
            }
            ViewerKind::Audio => {
                append_audio_viewer(state, &body, path);
            }
            ViewerKind::Generic => {
                append_generic_file_viewer(&body, path, suggest_name);
            }
        }

        bar.append(&open_ext);
        bar.append(&dl);
        bar.append(&close);
        root.append(&bar);
        if let Some(navigation) = navigation {
            root.append(navigation);
        }
        root.append(&body);
        win.set_child(Some(&root));

        {
            let w = win.clone();
            close.connect_clicked(move |_| w.close());
        }
        {
            let path = path.to_string();
            open_ext.connect_clicked(move |_| {
                open_path_externally(&path);
            });
        }
        {
            let s = state.clone();
            let path = path.to_string();
            let name = suggest_name.to_string();
            let ct = content_type.to_string();
            dl.connect_clicked(move |_| {
                save_media_as(&s, &path, &name, &ct);
            });
        }
    }
}

pub(super) fn open_media_viewer(
    state: &AppState,
    path: &str,
    content_type: &str,
    suggest_name: &str,
) {
    let viewer = MediaViewer::new(state);
    let (width, height) = match viewer_kind_for(content_type, path, suggest_name) {
        ViewerKind::Video => (1100, 720),
        ViewerKind::Image => (1080, 760),
        ViewerKind::Pdf | ViewerKind::Text => (920, 700),
        _ => (720, 480),
    };
    viewer.window.set_default_size(width, height);
    viewer.show(state, path, content_type, suggest_name, None);
    viewer.window.present();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewerKind {
    Image,
    Video,
    Pdf,
    Text,
    Audio,
    Generic,
}

pub(super) fn viewer_kind_for(content_type: &str, path: &str, name: &str) -> ViewerKind {
    let ct = content_type.to_ascii_uppercase();
    let extension = file_extension(name);
    let extension = if extension.is_empty() {
        file_extension(path)
    } else {
        extension
    };
    if ct == "VIDEO" || matches!(extension.as_str(), "mp4" | "webm" | "mkv" | "mov" | "m4v") {
        return ViewerKind::Video;
    }
    if ct == "IMAGE"
        || matches!(
            extension.as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
        )
    {
        return ViewerKind::Image;
    }
    if ct == "AUDIO"
        || matches!(
            extension.as_str(),
            "m4a" | "mp3" | "aac" | "ogg" | "wav" | "flac"
        )
    {
        return ViewerKind::Audio;
    }
    if extension == "pdf" {
        return ViewerKind::Pdf;
    }
    if matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "log"
            | "json"
            | "csv"
            | "xml"
            | "yaml"
            | "yml"
            | "toml"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "css"
            | "html"
            | "c"
            | "h"
            | "go"
    ) {
        return ViewerKind::Text;
    }
    // Sniff text files without extension.
    if extension.is_empty() && (ct == "FILE" || ct.is_empty()) && looks_like_text_file(path) {
        return ViewerKind::Text;
    }
    ViewerKind::Generic
}

pub(super) fn looks_like_text_file(path: &str) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 512];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    if n == 0 {
        return false;
    }
    let sample = &buf[..n];
    if sample.contains(&0) {
        return false;
    }
    let textish = sample
        .iter()
        .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    textish * 100 / n >= 90
}

pub(super) fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.1} GB", v / GB)
    } else if v >= MB {
        format!("{:.1} MB", v / MB)
    } else if v >= KB {
        format!("{:.0} KB", v / KB)
    } else {
        format!("{n} B")
    }
}

pub(super) fn prepare_external_attachment(
    data_dir: &std::path::Path,
    path: &str,
    name: &str,
) -> std::io::Result<PathBuf> {
    let source = std::path::Path::new(path);
    let extension = meaningful_file_name(name)
        .map(file_extension)
        .unwrap_or_default();
    if extension.is_empty() || extension == file_extension(path) {
        return Ok(source.to_path_buf());
    }
    let basename = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment");
    let directory = data_dir
        .join("cache/media")
        .join(format!("{}.open", downloads::sanitize_filename(basename)));
    std::fs::create_dir_all(&directory)?;
    let destination = downloads::unique_download_dest(&directory, name);
    let mut input = std::fs::File::open(source)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    if let Err(error) = std::io::copy(&mut input, &mut output) {
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    Ok(destination)
}

pub(super) fn open_path_externally(path: &str) {
    let uri = gio::File::for_path(path).uri();
    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_ok() {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Prefer GPU decode + offloaded composition for in-app video.
pub(super) fn append_gpu_video_viewer(body: &gtk::Box, path: &str) {
    let file = gio::File::for_path(path);
    let stream = gtk::MediaFile::for_file(&file);
    stream.set_loop(false);

    let video = gtk::Video::for_media_stream(Some(&stream));
    video.set_autoplay(true);
    video.set_loop(false);
    video.set_hexpand(true);
    video.set_vexpand(true);
    video.add_css_class("line-media-viewer-video");
    // GTK 4.14+: keep decoded frames on the GPU instead of round-tripping through CPU.
    video.set_graphics_offload(gtk::GraphicsOffloadEnabled::Enabled);

    let offload = gtk::GraphicsOffload::new(Some(&video));
    offload.set_hexpand(true);
    offload.set_vexpand(true);
    offload.add_css_class("line-media-viewer-video");

    body.append(&offload);
    stream.play();
}

fn render_image_viewport(
    source: &gdk_pixbuf::Pixbuf,
    dest: &gdk_pixbuf::Pixbuf,
    offset: (f64, f64),
    zoom: f64,
    interp: gdk_pixbuf::InterpType,
) {
    dest.fill(0);
    // GdkPixbuf rounds offsets and extends edge pixels outside the source image.
    let ox = (offset.0 + 0.5).floor();
    let oy = (offset.1 + 0.5).floor();
    let left = ox.clamp(0.0, dest.width() as f64) as i32;
    let top = oy.clamp(0.0, dest.height() as f64) as i32;
    let right = (ox + source.width() as f64 * zoom)
        .ceil()
        .clamp(0.0, dest.width() as f64) as i32;
    let bottom = (oy + source.height() as f64 * zoom)
        .ceil()
        .clamp(0.0, dest.height() as f64) as i32;
    if right > left && bottom > top {
        source.scale(
            dest,
            left,
            top,
            right - left,
            bottom - top,
            ox,
            oy,
            zoom,
            zoom,
            interp,
        );
    }
}

pub(super) fn append_image_viewer(
    state: &AppState,
    bar: &gtk::Box,
    body: &gtk::Box,
    path: &str,
    win: &gtk::Window,
) -> Option<gtk::EventControllerKey> {
    let Ok(base_pixbuf) = gdk_pixbuf::Pixbuf::from_file(path) else {
        let label = gtk::Label::builder()
            .label(crate::i18n::t("media_unsupported_preview"))
            .css_classes(["dim-label"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        body.append(&label);
        return None;
    };
    super::qr::append_button(bar, path, win);
    let nat_w = base_pixbuf.width().max(1);
    let nat_h = base_pixbuf.height().max(1);

    // View transform: image pixel (ix,iy) -> widget (ox + ix*zoom, oy + iy*zoom).
    let zoom = Rc::new(RefCell::new(1.0_f64));
    let offset = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let fitted = Rc::new(RefCell::new(false));
    let fast_paint = Rc::new(RefCell::new(false));
    let pixbuf = Rc::new(base_pixbuf);
    // Reused viewport-sized buffer so pan/zoom never reallocates every frame.
    let scratch = Rc::new(RefCell::new(None::<gdk_pixbuf::Pixbuf>));
    let strokes = Rc::new(RefCell::new(Vec::<Annotation>::new()));
    let tool = Rc::new(RefCell::new(ViewerTool::Pan));
    let brush_color = Rc::new(RefCell::new((0.90_f64, 0.22, 0.21, 1.0)));
    let brush_width = Rc::new(RefCell::new(6.0_f64));
    let active_stroke = Rc::new(RefCell::new(None::<Annotation>));
    let active_drag = Rc::new(RefCell::new(None::<ViewerDrag>));

    let zoom_out = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_out"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_reset = gtk::Button::builder()
        .icon_name("zoom-fit-best-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_reset"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_in = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_in"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&zoom_out);
    bar.append(&zoom_reset);
    bar.append(&zoom_in);

    let draw_btn = gtk::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(crate::i18n::t("media_draw"))
        .css_classes(["flat", "circular"])
        .build();
    let rectangle_btn = gtk::ToggleButton::builder()
        .label("□")
        .tooltip_text(crate::i18n::t("media_draw_rectangle"))
        .css_classes(["flat", "circular"])
        .build();
    let circle_btn = gtk::ToggleButton::builder()
        .label("○")
        .tooltip_text(crate::i18n::t("media_draw_circle"))
        .css_classes(["flat", "circular"])
        .build();
    let undo_btn = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_undo"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let clear_btn = gtk::Button::builder()
        .icon_name("edit-clear-all-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_clear"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let send_btn = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text(crate::i18n::t("attachment_image_queue"))
        .css_classes(["flat", "circular", "suggested-action"])
        .build();
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&draw_btn);
    bar.append(&rectangle_btn);
    bar.append(&circle_btn);
    for (r, g, b, class, tip) in [
        (0.90, 0.22, 0.21, "line-draw-c-red", "Red"),
        (0.26, 0.63, 0.28, "line-draw-c-green", "Green"),
        (0.12, 0.53, 0.90, "line-draw-c-blue", "Blue"),
        (0.99, 0.85, 0.21, "line-draw-c-yellow", "Yellow"),
        (1.0, 1.0, 1.0, "line-draw-c-white", "White"),
        (0.13, 0.13, 0.13, "line-draw-c-black", "Black"),
    ] {
        let btn = gtk::Button::builder()
            .label("●")
            .tooltip_text(tip)
            .css_classes(["flat", "circular", "line-draw-swatch", class])
            .build();
        let brush_color = brush_color.clone();
        let draw_btn = draw_btn.clone();
        let tool = tool.clone();
        btn.connect_clicked(move |_| {
            *brush_color.borrow_mut() = (r, g, b, 1.0);
            if *tool.borrow() == ViewerTool::Pan {
                draw_btn.set_active(true);
            }
        });
        bar.append(&btn);
    }
    bar.append(&undo_btn);
    bar.append(&clear_btn);
    bar.append(&send_btn);
    let copy_btn = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(crate::i18n::t("media_copy_image"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&copy_btn);
    let copy_image = {
        let s = state.clone();
        let clipboard = win.clipboard();
        let pixbuf = pixbuf.clone();
        let strokes = strokes.clone();
        let active_stroke = active_stroke.clone();
        Rc::new(move || {
            let packed: Vec<_> = strokes
                .borrow()
                .iter()
                .chain(active_stroke.borrow().iter())
                .cloned()
                .collect();
            match image_clipboard_texture(&pixbuf, &packed) {
                Ok(texture) => {
                    clipboard.set_texture(&texture);
                    toast(&s, &crate::i18n::t("media_image_copied"));
                }
                Err(error) => toast(
                    &s,
                    &crate::i18n::tf("media_image_copy_failed", &[("error", &error.to_string())]),
                ),
            }
        })
    };
    {
        let copy_image = copy_image.clone();
        copy_btn.connect_clicked(move |_| copy_image());
    }
    let copy_keys = gtk::EventControllerKey::new();
    copy_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && !modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK)
            && matches!(key, gdk::Key::c | gdk::Key::C)
        {
            copy_image();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(copy_keys.clone());

    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-image"])
        .build();
    area.set_cursor_from_name(Some("grab"));
    area.set_draw_func({
        let pixbuf = pixbuf.clone();
        let scratch = scratch.clone();
        let zoom = zoom.clone();
        let offset = offset.clone();
        let fast_paint = fast_paint.clone();
        let strokes = strokes.clone();
        let active_stroke = active_stroke.clone();
        move |_area, cr, width, height| {
            if width <= 0 || height <= 0 {
                return;
            }
            // Clear is handled by GTK/CSS background; only composite the scaled viewport.
            let z = (*zoom.borrow()).max(0.01);
            let (ox, oy) = *offset.borrow();
            let interp = if *fast_paint.borrow() {
                gdk_pixbuf::InterpType::Nearest
            } else {
                gdk_pixbuf::InterpType::Bilinear
            };

            {
                let mut slot = scratch.borrow_mut();
                let needs = slot
                    .as_ref()
                    .map(|p| p.width() != width || p.height() != height)
                    .unwrap_or(true);
                if needs {
                    *slot = gdk_pixbuf::Pixbuf::new(
                        gdk_pixbuf::Colorspace::Rgb,
                        true,
                        8,
                        width,
                        height,
                    );
                }
                if let Some(dest) = slot.as_ref() {
                    render_image_viewport(&pixbuf, dest, (ox, oy), z, interp);
                    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, dest, 0.0, 0.0);
                    let _ = cr.paint();
                }
            }

            let _ = cr.save();
            cr.translate(ox, oy);
            cr.scale(z, z);
            for stroke in strokes.borrow().iter().chain(active_stroke.borrow().iter()) {
                let _ = paint_annotation(cr, stroke);
            }
            let _ = cr.restore();
        }
    });

    let fit_to_area = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        Rc::new(move || {
            let aw = area.width().max(1) as f64;
            let ah = area.height().max(1) as f64;
            let z = (aw / nat_w as f64).min(ah / nat_h as f64).clamp(0.05, 1.0);
            *zoom.borrow_mut() = z;
            *offset.borrow_mut() = ((aw - nat_w as f64 * z) * 0.5, (ah - nat_h as f64 * z) * 0.5);
            area.queue_draw();
        })
    };

    let zoom_at = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        Rc::new(move |factor: f64, pivot_x: f64, pivot_y: f64, fast: bool| {
            let old = (*zoom.borrow()).max(0.01);
            let new_z = (old * factor).clamp(0.05, 16.0);
            if (new_z - old).abs() < f64::EPSILON {
                return;
            }
            let (ox, oy) = *offset.borrow();
            // Keep the image point under the pivot fixed.
            let ox2 = pivot_x - (pivot_x - ox) * (new_z / old);
            let oy2 = pivot_y - (pivot_y - oy) * (new_z / old);
            *zoom.borrow_mut() = new_z;
            *offset.borrow_mut() = (ox2, oy2);
            *fast_paint.borrow_mut() = fast;
            area.queue_draw();
        })
    };

    {
        let area = area.clone();
        let fitted = fitted.clone();
        let fit_to_area = fit_to_area.clone();
        area.connect_resize(move |_, _, _| {
            if !*fitted.borrow() {
                *fitted.borrow_mut() = true;
                fit_to_area();
            }
        });
    }

    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_in.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_out.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.0 / 1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let fit_to_area = fit_to_area.clone();
        let fast_paint = fast_paint.clone();
        zoom_reset.connect_clicked(move |_| {
            *fast_paint.borrow_mut() = false;
            fit_to_area();
        });
    }

    let refresh_edit_btns = {
        let undo_btn = undo_btn.clone();
        let clear_btn = clear_btn.clone();
        let strokes = strokes.clone();
        Rc::new(move || {
            let n = strokes.borrow().len();
            undo_btn.set_sensitive(n > 0);
            clear_btn.set_sensitive(n > 0);
        })
    };

    let tool_buttons = [
        (draw_btn, ViewerTool::Freehand),
        (rectangle_btn, ViewerTool::Rectangle),
        (circle_btn, ViewerTool::Circle),
    ];
    for (button, selected) in &tool_buttons {
        let selected = *selected;
        let others: Vec<_> = tool_buttons
            .iter()
            .filter(|(_, candidate)| *candidate != selected)
            .map(|(button, _)| button.downgrade())
            .collect();
        let tool = tool.clone();
        let active_stroke = active_stroke.clone();
        let active_drag = active_drag.clone();
        let fast_paint = fast_paint.clone();
        let area = area.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                for other in &others {
                    if let Some(other) = other.upgrade() {
                        other.set_active(false);
                    }
                }
                *tool.borrow_mut() = selected;
            } else if *tool.borrow() == selected {
                *tool.borrow_mut() = ViewerTool::Pan;
            }
            active_stroke.borrow_mut().take();
            active_drag.borrow_mut().take();
            *fast_paint.borrow_mut() = false;
            area.set_cursor_from_name(Some(if *tool.borrow() == ViewerTool::Pan {
                "grab"
            } else {
                "crosshair"
            }));
            area.queue_draw();
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        undo_btn.connect_clicked(move |_| {
            strokes.borrow_mut().pop();
            refresh();
            area.queue_draw();
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        let active_stroke = active_stroke.clone();
        let active_drag = active_drag.clone();
        clear_btn.connect_clicked(move |_| {
            active_stroke.borrow_mut().take();
            active_drag.borrow_mut().take();
            strokes.borrow_mut().clear();
            refresh();
            area.queue_draw();
        });
    }
    {
        let s = state.clone();
        let win = win.clone();
        let pixbuf = pixbuf.clone();
        let strokes = strokes.clone();
        let src_path = path.to_string();
        send_btn.connect_clicked(move |_| {
            let out = if strokes.borrow().is_empty() {
                std::path::PathBuf::from(&src_path)
            } else {
                match export_annotated_image(pixbuf.as_ref(), &strokes.borrow(), &s.data_dir) {
                    Ok(p) => p,
                    Err(e) => {
                        toast(
                            &s,
                            &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                        );
                        return;
                    }
                }
            };
            if send_local_image_file(&s, &out) {
                win.close();
            }
        });
    }

    // Scroll: wheel pans; Ctrl+wheel zooms toward pointer.
    let scroll_ctl = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    {
        let zoom_at = zoom_at.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        scroll_ctl.connect_scroll(move |ctl, dx, dy| {
            let state = ctl.current_event_state();
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                let px = area.width() as f64 * 0.5;
                let py = area.height() as f64 * 0.5;
                let factor = if dy < 0.0 {
                    1.15
                } else if dy > 0.0 {
                    1.0 / 1.15
                } else {
                    return glib::Propagation::Stop;
                };
                zoom_at(factor, px, py, true);
                let fast_paint = fast_paint.clone();
                let area = area.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                    *fast_paint.borrow_mut() = false;
                    area.queue_draw();
                });
                return glib::Propagation::Stop;
            }
            // Pan with wheel.
            let mut off = offset.borrow_mut();
            off.0 -= dx * 48.0;
            off.1 -= dy * 48.0;
            drop(off);
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll_ctl);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    {
        let tool = tool.clone();
        let active_drag = active_drag.clone();
        let offset = offset.clone();
        let active_stroke = active_stroke.clone();
        let brush_color = brush_color.clone();
        let brush_width = brush_width.clone();
        let zoom = zoom.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_begin(move |gesture, x, y| {
            let selected = *tool.borrow();
            let origin = *offset.borrow();
            if selected == ViewerTool::Pan {
                *active_drag.borrow_mut() = Some(ViewerDrag::Pan { origin });
                *fast_paint.borrow_mut() = true;
                area.set_cursor_from_name(Some("grabbing"));
            } else {
                let z = (*zoom.borrow()).max(0.01);
                let start = ((x - origin.0) / z, (y - origin.1) / z);
                *active_stroke.borrow_mut() = Some(Annotation {
                    color: *brush_color.borrow(),
                    width: (*brush_width.borrow() / z).max(1.0),
                    geometry: match selected {
                        ViewerTool::Freehand => AnnotationGeometry::Freehand(vec![start]),
                        ViewerTool::Rectangle => {
                            AnnotationGeometry::Rectangle { start, end: start }
                        }
                        ViewerTool::Circle => AnnotationGeometry::Circle { start, end: start },
                        ViewerTool::Pan => unreachable!(),
                    },
                });
                *active_drag.borrow_mut() = Some(ViewerDrag::Draw { start, zoom: z });
            }
            area.queue_draw();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    let update_drag = {
        let active_drag = active_drag.clone();
        let active_stroke = active_stroke.clone();
        let offset = offset.clone();
        let area = area.clone();
        Rc::new(move |dx: f64, dy: f64| {
            match *active_drag.borrow() {
                Some(ViewerDrag::Pan { origin }) => {
                    *offset.borrow_mut() = (origin.0 + dx, origin.1 + dy)
                }
                Some(ViewerDrag::Draw { start, zoom }) => {
                    if let Some(annotation) = active_stroke.borrow_mut().as_mut() {
                        annotation.update_end((start.0 + dx / zoom, start.1 + dy / zoom));
                    }
                }
                None => {}
            }
            area.queue_draw();
        })
    };
    {
        let update_drag = update_drag.clone();
        drag.connect_drag_update(move |_, dx, dy| update_drag(dx, dy));
    }
    {
        let active_drag = active_drag.clone();
        let active_stroke = active_stroke.clone();
        let strokes = strokes.clone();
        let area = area.clone();
        let tool = tool.clone();
        let refresh = refresh_edit_btns.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            update_drag(dx, dy);
            active_drag.borrow_mut().take();
            if let Some(annotation) = active_stroke.borrow_mut().take()
                && annotation.is_drawable()
            {
                strokes.borrow_mut().push(annotation);
                refresh();
            }
            *fast_paint.borrow_mut() = false;
            area.set_cursor_from_name(Some(if *tool.borrow() == ViewerTool::Pan {
                "grab"
            } else {
                "crosshair"
            }));
            area.queue_draw();
        });
    }
    {
        let active_drag = active_drag.clone();
        let active_stroke = active_stroke.clone();
        let area = area.clone();
        let tool = tool.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_cancel(move |_, _| {
            active_drag.borrow_mut().take();
            active_stroke.borrow_mut().take();
            *fast_paint.borrow_mut() = false;
            area.set_cursor_from_name(Some(if *tool.borrow() == ViewerTool::Pan {
                "grab"
            } else {
                "crosshair"
            }));
            area.queue_draw();
        });
    }
    area.add_controller(drag);

    body.append(&area);
    Some(copy_keys)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewerTool {
    Pan,
    Freehand,
    Rectangle,
    Circle,
}

#[derive(Clone, Copy)]
enum ViewerDrag {
    Pan { origin: (f64, f64) },
    Draw { start: (f64, f64), zoom: f64 },
}

#[derive(Clone)]
pub(super) struct Annotation {
    color: (f64, f64, f64, f64),
    width: f64,
    geometry: AnnotationGeometry,
}

#[derive(Clone)]
enum AnnotationGeometry {
    Freehand(Vec<(f64, f64)>),
    Rectangle { start: (f64, f64), end: (f64, f64) },
    Circle { start: (f64, f64), end: (f64, f64) },
}

impl Annotation {
    fn update_end(&mut self, point: (f64, f64)) {
        match &mut self.geometry {
            AnnotationGeometry::Freehand(points) => {
                if points.last() != Some(&point) {
                    points.push(point);
                }
            }
            AnnotationGeometry::Rectangle { end, .. } | AnnotationGeometry::Circle { end, .. } => {
                *end = point
            }
        }
    }

    fn is_drawable(&self) -> bool {
        match &self.geometry {
            AnnotationGeometry::Freehand(points) => !points.is_empty(),
            AnnotationGeometry::Rectangle { start, end } => start.0 != end.0 && start.1 != end.1,
            AnnotationGeometry::Circle { start, end } => start != end,
        }
    }
}

fn paint_annotation(cr: &cairo::Context, annotation: &Annotation) -> Result<(), cairo::Error> {
    if !annotation.is_drawable() {
        return Ok(());
    }
    let (r, g, b, a) = annotation.color;
    cr.set_source_rgba(r, g, b, a);
    cr.set_line_width(annotation.width);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.new_path();
    match &annotation.geometry {
        AnnotationGeometry::Freehand(points) => {
            cr.move_to(points[0].0, points[0].1);
            for &(x, y) in points.iter().skip(1) {
                cr.line_to(x, y);
            }
            if points.len() == 1 {
                cr.line_to(points[0].0 + 0.01, points[0].1);
            }
        }
        AnnotationGeometry::Rectangle { start, end } => {
            cr.rectangle(
                start.0.min(end.0),
                start.1.min(end.1),
                (end.0 - start.0).abs(),
                (end.1 - start.1).abs(),
            );
        }
        AnnotationGeometry::Circle { start, end } => {
            let dx = end.0 - start.0;
            let dy = end.1 - start.1;
            let side = dx.abs().max(dy.abs());
            let x = if dx < 0.0 { start.0 - side } else { start.0 };
            let y = if dy < 0.0 { start.1 - side } else { start.1 };
            cr.arc(
                x + side / 2.0,
                y + side / 2.0,
                side / 2.0,
                0.0,
                std::f64::consts::TAU,
            );
        }
    }
    cr.stroke()
}

fn render_annotated_image(
    pixbuf: &gdk_pixbuf::Pixbuf,
    strokes: &[Annotation],
) -> anyhow::Result<cairo::ImageSurface> {
    let w = pixbuf.width().max(1);
    let h = pixbuf.height().max(1);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| anyhow::anyhow!("cairo surface: {e}"))?;
    let cr = cairo::Context::new(&surface).map_err(|e| anyhow::anyhow!("cairo: {e}"))?;
    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, pixbuf, 0.0, 0.0);
    cr.paint().map_err(|e| anyhow::anyhow!("paint: {e}"))?;

    for annotation in strokes {
        paint_annotation(&cr, annotation)?;
    }

    Ok(surface)
}

fn image_clipboard_texture(
    pixbuf: &gdk_pixbuf::Pixbuf,
    strokes: &[Annotation],
) -> anyhow::Result<gdk::Texture> {
    if strokes.is_empty() {
        return Ok(gdk::Texture::for_pixbuf(pixbuf));
    }
    let surface = render_annotated_image(pixbuf, strokes)?;
    let mut png = Vec::new();
    surface.write_to_png(&mut png)?;
    Ok(gdk::Texture::from_bytes(&glib::Bytes::from_owned(png))?)
}

pub(super) fn export_annotated_image(
    pixbuf: &gdk_pixbuf::Pixbuf,
    strokes: &[Annotation],
    data_dir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let surface = render_annotated_image(pixbuf, strokes)?;
    let dir = data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!(
        "annotated-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut file = std::fs::File::create(&dest)?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| anyhow::anyhow!("png: {e}"))?;
    Ok(dest)
}

pub(super) fn send_local_image_file(state: &AppState, path: &std::path::Path) -> bool {
    let Some(chat) = state.current_chat.borrow().clone() else {
        return false;
    };
    stage_attachment(state, &chat, path.to_path_buf())
}

pub(super) fn append_pdf_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(12)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-doc"])
        .build();

    let preview_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-pdf-preview"])
        .build();

    // Render first page with pdftoppm when available.
    let cache = std::env::temp_dir().join(format!(
        "line-gtk-pdf-{}-preview",
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
    ));
    let _ = std::fs::create_dir_all(&cache);
    let out_prefix = cache.join("page");
    let rendered = std::process::Command::new("pdftoppm")
        .args([
            "-jpeg",
            "-f",
            "1",
            "-l",
            "1",
            "-r",
            "120",
            path,
            out_prefix.to_str().unwrap_or("/tmp/line-gtk-pdf"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let preview_path = cache.join("page-1.jpg");
    if rendered && preview_path.is_file() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let pic = gtk::Picture::for_filename(&preview_path);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        scroll.set_child(Some(&pic));
        preview_box.append(&scroll);
    } else {
        let hint = gtk::Label::builder()
            .label(crate::i18n::t("media_pdf_preview"))
            .css_classes(["title-3"])
            .build();
        preview_box.append(&hint);
    }

    let meta = gtk::Label::builder()
        .label({
            let size = std::fs::metadata(path)
                .map(|m| format_bytes(m.len()))
                .unwrap_or_else(|_| "?".into());
            format!(
                "{suggest_name} · {}",
                crate::i18n::tf("media_file_size", &[("size", &size)])
            )
        })
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();

    // Optional text extract for searchable peek.
    let text_out = std::process::Command::new("pdftotext")
        .args(["-f", "1", "-l", "2", "-layout", path, "-"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_default();
    if !text_out.trim().is_empty() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .min_content_height(140)
            .css_classes(["line-media-viewer-text-scroll"])
            .build();
        let tv = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .css_classes(["line-media-viewer-text"])
            .build();
        let buf = tv.buffer();
        let snippet: String = text_out.chars().take(4_000).collect();
        buf.set_text(&snippet);
        scroll.set_child(Some(&tv));
        wrap.append(&preview_box);
        wrap.append(&meta);
        wrap.append(&scroll);
    } else {
        wrap.append(&preview_box);
        wrap.append(&meta);
    }

    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .halign(gtk::Align::Start)
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&open_btn);
    body.append(&wrap);
}

pub(super) fn append_text_viewer(body: &gtk::Box, path: &str) {
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-text-scroll"])
        .build();
    let tv = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .monospace(true)
        .left_margin(14)
        .right_margin(14)
        .top_margin(12)
        .bottom_margin(12)
        .css_classes(["line-media-viewer-text"])
        .build();
    let buf = tv.buffer();
    const LIMIT: u64 = 1_500_000;
    let meta = std::fs::metadata(path).ok();
    let truncated = meta.map(|m| m.len() > LIMIT).unwrap_or(false);
    match std::fs::read(path) {
        Ok(bytes) => {
            let slice = if bytes.len() as u64 > LIMIT {
                &bytes[..LIMIT as usize]
            } else {
                &bytes[..]
            };
            let text = String::from_utf8_lossy(slice);
            if truncated {
                buf.set_text(&format!(
                    "{}\n\n{}",
                    crate::i18n::t("media_text_truncated"),
                    text
                ));
            } else {
                buf.set_text(&text);
            }
        }
        Err(e) => {
            buf.set_text(&crate::i18n::tf(
                "file_read_failed",
                &[("error", &e.to_string())],
            ));
        }
    }
    scroll.set_child(Some(&tv));
    body.append(&scroll);
}

pub(super) fn append_audio_viewer(state: &AppState, body: &gtk::Box, path: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-audio"])
        .build();
    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.set_pixel_size(72);
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let label = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let play = gtk::Button::builder()
        .label(crate::i18n::t("play_voice"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let cfg = state.config.clone();
    let toast_state = state.clone();
    let path = path.to_string();
    play.connect_clicked(move |_| {
        if let Err(e) = play_audio_file(&path, &cfg.borrow().audio_output) {
            toast(
                &toast_state,
                &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
            );
        }
    });
    wrap.append(&icon);
    wrap.append(&label);
    wrap.append(&play);
    body.append(&wrap);
}

pub(super) fn append_generic_file_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_start(24)
        .margin_end(24)
        .css_classes(["line-media-viewer-generic"])
        .build();
    let icon = gtk::Image::from_icon_name("folder-download-symbolic");
    icon.set_pixel_size(64);
    let name = gtk::Label::builder()
        .label(suggest_name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-3"])
        .build();
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let meta = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("media_unsupported_preview"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&icon);
    wrap.append(&name);
    wrap.append(&meta);
    wrap.append(&hint);
    wrap.append(&open_btn);
    body.append(&wrap);
}

pub(super) fn pulse_device_list(kind: &str) -> Vec<(String, String)> {
    // kind: "sources" | "sinks"
    let out = std::process::Command::new("pactl")
        .args(["list", "short", kind])
        .output()
        .ok();
    let Some(out) = out else {
        return vec![("default".into(), "Default".into())];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = vec![("default".into(), "Default".into())];
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[1].to_string();
        // Skip monitors as record sources unless nothing else.
        if kind == "sources" && name.contains(".monitor") {
            continue;
        }
        let label = name.clone();
        list.push((name, label));
    }
    list
}

pub fn pulse_sources() -> Vec<(String, String)> {
    pulse_device_list("sources")
}

pub fn pulse_sinks() -> Vec<(String, String)> {
    pulse_device_list("sinks")
}

pub(super) fn guess_media_o_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "gif" => "gif",
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "heic" => "image",
        "mp4" | "webm" | "mov" | "mkv" | "m4v" => "video",
        "m4a" | "mp3" | "wav" | "ogg" | "oga" | "aac" | "flac" => "audio",
        _ => "file",
    }
}

pub(super) fn ffprobe_duration_ms(path: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str()?,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let secs: f64 = text.trim().parse().ok()?;
    if !secs.is_finite() || secs <= 0.0 {
        return None;
    }
    Some((secs * 1000.0).round() as u64)
}

pub(super) fn mark_media_failed(state: &AppState, message_id: &str) {
    let Some(bubble) = state.media_slots.borrow().get(message_id).cloned() else {
        return;
    };
    let msg = state.media_msgs.borrow().get(message_id).cloned();
    let is_video = msg
        .as_ref()
        .map(|m| m.content_type.eq_ignore_ascii_case("video"))
        .unwrap_or(false);
    // Don't clobber a voice card with "image unavailable".
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if w.css_classes().iter().any(|c| c == "line-voice-card") {
            // Keep voice UI; just mark play unavailable text if needed.
            let fail = gtk::Label::builder()
                .label(crate::i18n::t("voice_unavailable"))
                .xalign(0.0)
                .css_classes(["dim-label", "line-media-failed"])
                .build();
            bubble.append(&fail);
            return;
        }
        child = next;
    }
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        let drop = w.css_classes().iter().any(|c| {
            c == "line-media-placeholder"
                || c == "line-bubble-image"
                || c == "line-media-failed"
                || c == "line-video-wrap"
                || c == "line-video-placeholder"
        }) || w.downcast_ref::<gtk::Picture>().is_some()
            || w.downcast_ref::<gtk::Overlay>().is_some();
        if drop {
            bubble.remove(&w);
        }
        child = next;
    }
    if is_video && let Some(msg) = msg.as_ref() {
        append_video_placeholder(state, &bubble, msg, true);
        return;
    }
    let label = gtk::Label::builder()
        .label(crate::i18n::t("image_unavailable"))
        .xalign(0.0)
        .css_classes(["dim-label", "line-media-failed"])
        .build();
    append_media_visual(&bubble, &label);
}

pub(super) fn append_media_visual(bubble: &gtk::Box, visual: &impl IsA<gtk::Widget>) {
    let mut child = bubble.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("line-file-actions") {
            bubble.insert_child_after(visual, widget.prev_sibling().as_ref());
            return;
        }
        child = widget.next_sibling();
    }
    bubble.append(visual);
}

pub(super) fn make_media_picture_placeholder(sticker: bool) -> gtk::Picture {
    let (w, h) = if sticker { (128, 128) } else { (220, 160) };
    gtk::Picture::builder()
        .content_fit(if sticker {
            gtk::ContentFit::Contain
        } else {
            gtk::ContentFit::Cover
        })
        .can_shrink(true)
        .width_request(w)
        .height_request(h)
        .css_classes(if sticker {
            ["line-sticker-image"]
        } else {
            ["line-bubble-image"]
        })
        .build()
}

pub(super) fn wrap_video_thumb(
    state: &AppState,
    pic: &gtk::Picture,
    msg: &MessageInfo,
) -> gtk::Overlay {
    pic.add_css_class("line-video-thumb");
    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_video")));
    let overlay = gtk::Overlay::builder()
        .child(pic)
        .css_classes(["line-video-wrap"])
        .build();
    let badge = gtk::Image::builder()
        .icon_name("media-playback-start-symbolic")
        .pixel_size(28)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-video-play-badge"])
        .build();
    overlay.add_overlay(&badge);
    wire_media_open_click_widget(state, overlay.upcast_ref::<gtk::Widget>(), msg);
    overlay
}

pub(super) fn append_video_placeholder(
    state: &AppState,
    bubble: &gtk::Box,
    msg: &MessageInfo,
    failed: bool,
) {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(180)
        .height_request(110)
        .css_classes(if failed {
            vec!["line-video-placeholder", "line-media-failed"]
        } else {
            vec!["line-video-placeholder", "line-media-placeholder"]
        })
        .build();
    let icon = gtk::Image::builder()
        .icon_name(if failed {
            "media-playback-start-symbolic"
        } else {
            "content-loading-symbolic"
        })
        .pixel_size(28)
        .css_classes(["line-video-play-badge"])
        .build();
    let label = gtk::Label::builder()
        .label(if failed {
            crate::i18n::t("video_unavailable")
        } else {
            crate::i18n::t("media_open_video")
        })
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    box_.append(&icon);
    box_.append(&label);
    wire_media_open_click_widget(state, box_.upcast_ref::<gtk::Widget>(), msg);
    bubble.append(&box_);
}

pub(super) fn show_upload_progress(state: &AppState, progress: f64, label: &str) {
    let frac = progress.clamp(0.0, 1.0);
    state.upload_bar.set_fraction(frac);
    let text = if label.is_empty() {
        crate::i18n::t("media_uploading")
    } else {
        label.to_string()
    };
    state.upload_label.set_text(&text);
    state.upload_revealer.set_reveal_child(true);
    state.status.set_text(&text);
}

pub(super) fn hide_upload_progress(state: &AppState) {
    state.upload_revealer.set_reveal_child(false);
    state.upload_bar.set_fraction(0.0);
    state.upload_label.set_text("");
}

pub(super) fn wire_media_open_click_widget(
    state: &AppState,
    widget: &gtk::Widget,
    msg: &MessageInfo,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let s = state.clone();
    let msg = msg.clone();
    click.connect_released(move |_, _n, _x, _y| {
        request_media_download(&s, &msg, "open_viewer");
    });
    widget.add_controller(click);
    widget.set_cursor_from_name(Some("pointer"));
}

pub(super) fn raw_frame_to_pixbuf(frame: &crate::sticker_anim::RawFrame) -> Option<Pixbuf> {
    let bytes = glib::Bytes::from_owned(frame.rgba.clone());
    Some(Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        frame.width,
        frame.height,
        frame.width * 4,
    ))
}

pub(super) fn apply_frames_to_picture(
    picture: &gtk::Picture,
    frames: crate::sticker_anim::AnimFrames,
    animate: bool,
) {
    let Some(first) = frames.frames.first().and_then(raw_frame_to_pixbuf) else {
        return;
    };
    picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&first)));
    if !animate || frames.frames.len() <= 1 {
        return;
    }

    let max_plays = frames.plays;
    let pixbufs: Vec<(Pixbuf, u32)> = frames
        .frames
        .iter()
        .filter_map(|f| {
            let pb = raw_frame_to_pixbuf(f)?;
            Some((pb, f.delay_ms))
        })
        .collect();
    if pixbufs.len() <= 1 {
        return;
    }

    let frames = std::rc::Rc::new(pixbufs);
    let idx = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let completed_plays = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let picture = picture.clone();

    fn schedule(
        picture: gtk::Picture,
        frames: std::rc::Rc<Vec<(Pixbuf, u32)>>,
        idx: std::rc::Rc<std::cell::Cell<usize>>,
        completed_plays: std::rc::Rc<std::cell::Cell<u32>>,
        max_plays: u32,
    ) {
        let delay = frames
            .get(idx.get())
            .map(|(_, d)| (*d as u64).max(20))
            .unwrap_or(50);
        let picture_weak = picture.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            let Some(picture) = picture_weak.upgrade() else {
                return;
            };
            if picture.parent().is_none() {
                return;
            }
            let next = (idx.get() + 1) % frames.len();
            if next == 0 {
                let completed = completed_plays.get().saturating_add(1);
                completed_plays.set(completed);
                if max_plays > 0 && completed >= max_plays {
                    return;
                }
            }
            idx.set(next);
            if let Some((pb, _)) = frames.get(next) {
                picture.set_paintable(Some(&gdk::Texture::for_pixbuf(pb)));
            }
            schedule(picture, frames, idx, completed_plays, max_plays);
        });
    }

    schedule(picture, frames, idx, completed_plays, max_plays);
}

pub(super) fn attach_texture_async(picture: gtk::Picture, path: String, max_px: i32) {
    attach_texture_async_anim(picture, path, max_px, false);
}

pub(super) fn attach_texture_async_anim(
    picture: gtk::Picture,
    path: String,
    max_px: i32,
    animate: bool,
) {
    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::sticker_anim::load_scaled(&path, max_px, animate));
    });
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Some(frames)) => apply_frames_to_picture(&picture, frames, animate),
            _ => {
                if let Some(parent) = picture.parent()
                    && let Some(box_) = parent.downcast_ref::<gtk::Box>()
                {
                    box_.remove(&picture);
                    let label = gtk::Label::builder()
                        .label(crate::i18n::t("image_unavailable"))
                        .xalign(0.0)
                        .css_classes(["dim-label", "line-media-failed"])
                        .build();
                    append_media_visual(box_, &label);
                }
            }
        }
    });
}

pub(super) fn pump_media_queue(state: &AppState) {
    if *state.media_pumping.borrow() {
        return;
    }
    let next = state.media_queue.borrow_mut().pop_front();
    let Some((message_id, image_path)) = next else {
        return;
    };
    if !std::path::Path::new(&image_path).exists() {
        mark_media_failed(state, &message_id);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    }

    let Some(bubble) = state.media_slots.borrow().get(&message_id).cloned() else {
        // Bubble not mounted yet (list rebuild / idle chunk). Keep the path so
        // append_message can attach it when the row appears; do not spin the queue.
        state
            .media_ready_paths
            .borrow_mut()
            .insert(message_id, image_path);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    };

    *state.media_pumping.borrow_mut() = true;
    let is_sticker = bubble
        .css_classes()
        .iter()
        .any(|c| c == "line-bubble-sticker");

    // Keep the "Loading…" label until decode succeeds.
    let max_px = if is_sticker { 128 } else { 320 };
    let animate = is_sticker && state.config.borrow().animations;
    let state2 = state.clone();
    let bubble2 = bubble.clone();
    let message_id2 = message_id.clone();

    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    let image_path2 = image_path.clone();
    std::thread::spawn(move || {
        let loaded =
            crate::sticker_anim::load_scaled(&image_path2, max_px, animate).or_else(|| {
                // Brief retry — avoids rare races right after atomic rename.
                std::thread::sleep(std::time::Duration::from_millis(30));
                crate::sticker_anim::load_scaled(&image_path2, max_px, animate)
            });
        let _ = tx.send_blocking(loaded);
    });
    glib::spawn_future_local(async move {
        let frames = rx.recv().await.ok().flatten();
        if let Some(frames) = frames {
            let mut child = bubble2.first_child();
            while let Some(w) = child {
                let next_w = w.next_sibling();
                if w.css_classes().iter().any(|c| {
                    c == "line-media-placeholder"
                        || c == "line-media-failed"
                        || c == "line-bubble-image"
                        || c == "line-video-wrap"
                        || c == "line-video-placeholder"
                }) || w.downcast_ref::<gtk::Picture>().is_some()
                    || w.downcast_ref::<gtk::Overlay>().is_some()
                {
                    bubble2.remove(&w);
                }
                child = next_w;
            }
            let pic = make_media_picture_placeholder(is_sticker);
            apply_frames_to_picture(&pic, frames, animate);
            let msg = state2.media_msgs.borrow().get(&message_id2).cloned();
            let is_video = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("video"))
                .unwrap_or(false);
            let is_image = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("image"))
                .unwrap_or(false);
            if !is_sticker && is_video {
                if let Some(msg) = msg.as_ref() {
                    let overlay = wrap_video_thumb(&state2, &pic, msg);
                    bubble2.append(&overlay);
                } else {
                    append_media_visual(&bubble2, &pic);
                }
            } else {
                append_media_visual(&bubble2, &pic);
                if !is_sticker
                    && is_image
                    && let Some(msg) = msg.as_ref()
                {
                    wire_media_open_click(&state2, &pic, msg, "image");
                    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_image")));
                }
            }
            if *state2.stick_bottom.borrow() {
                scroll_messages_to_end(&state2);
            }
        } else {
            mark_media_failed(&state2, &message_id2);
        }
        *state2.media_pumping.borrow_mut() = false;
        let state3 = state2.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(12), move || {
            pump_media_queue(&state3);
        });
    });
}

#[cfg(test)]
mod image_viewport_tests {
    use super::render_image_viewport;
    use gdk_pixbuf::{Colorspace, InterpType, Pixbuf};

    fn buffer(width: i32, height: i32) -> Pixbuf {
        Pixbuf::new(Colorspace::Rgb, true, 8, width, height).unwrap()
    }

    fn assert_pixels(dest: &Pixbuf, bounds: Option<(i32, i32, i32, i32)>) {
        let bytes = dest.read_pixel_bytes();
        let stride = dest.rowstride() as usize;
        for y in 0..dest.height() {
            for x in 0..dest.width() {
                let visible = bounds.is_some_and(|(left, top, right, bottom)| {
                    x >= left && x < right && y >= top && y < bottom
                });
                let expected = if visible {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                };
                let index = y as usize * stride + x as usize * 4;
                assert_eq!(&bytes[index..index + 4], &expected, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn centered_image_leaves_transparent_margins() {
        let source = buffer(4, 2);
        source.fill(0xff0000ff);
        let dest = buffer(10, 8);
        for interp in [InterpType::Nearest, InterpType::Bilinear] {
            render_image_viewport(&source, &dest, (3.0, 3.0), 1.0, interp);
            assert_pixels(&dest, Some((3, 3, 7, 5)));
        }
    }

    #[test]
    fn zoomed_panned_image_clips_to_viewport() {
        let source = buffer(4, 2);
        source.fill(0xff0000ff);
        let dest = buffer(10, 8);
        for interp in [InterpType::Nearest, InterpType::Bilinear] {
            render_image_viewport(&source, &dest, (-2.0, 1.0), 2.0, interp);
            assert_pixels(&dest, Some((0, 1, 6, 5)));
            render_image_viewport(&source, &dest, (2.5, -0.5), 0.5, interp);
            assert_pixels(&dest, Some((3, 0, 5, 1)));
        }
    }

    #[test]
    fn reused_buffer_clears_pixels_when_image_moves() {
        let source = buffer(4, 2);
        source.fill(0xff0000ff);
        let dest = buffer(10, 8);
        render_image_viewport(&source, &dest, (0.0, 0.0), 1.0, InterpType::Nearest);
        assert_pixels(&dest, Some((0, 0, 4, 2)));
        render_image_viewport(&source, &dest, (6.0, 6.0), 1.0, InterpType::Nearest);
        assert_pixels(&dest, Some((6, 6, 10, 8)));
        for offset in [(-10.0, 0.0), (10.0, 0.0), (0.0, -8.0), (0.0, 8.0)] {
            render_image_viewport(&source, &dest, offset, 1.0, InterpType::Nearest);
            assert_pixels(&dest, None);
        }
    }

    #[test]
    #[ignore = "requires a local image and output path"]
    fn render_local_image() {
        let source = Pixbuf::from_file(std::env::var("LINE_GTK_TEST_IMAGE").unwrap()).unwrap();
        let dest = buffer(1916, 990);
        render_image_viewport(&source, &dest, (292.0, 153.0), 1.0, InterpType::Bilinear);
        dest.savev(std::env::var("LINE_GTK_TEST_RENDER").unwrap(), "png", &[])
            .unwrap();
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn viewer_uses_final_extension_not_path_substrings() {
        assert!(matches!(
            viewer_kind_for("FILE", "/tmp/pdfs.pdf/report.docx", "report.docx"),
            ViewerKind::Generic
        ));
        assert!(matches!(
            viewer_kind_for("FILE", "", "report.pdf.docx"),
            ViewerKind::Generic
        ));
        assert!(matches!(
            viewer_kind_for("FILE", "", "file.CAB"),
            ViewerKind::Generic
        ));
        assert!(matches!(
            viewer_kind_for("FILE", "", "report.PDF"),
            ViewerKind::Pdf
        ));
    }

    #[test]
    fn cached_file_recovers_original_name_without_placeholder() {
        let msg: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "42", "contentType": "FILE", "text": "[File]", "fileName": "[File]", "filePath": "/tmp/cache/42_report.docx"
        })).unwrap();
        assert_eq!(suggest_media_name(&msg), "report.docx");
    }
    #[test]
    fn generated_cache_extension_does_not_become_filename() {
        let msg: MessageInfo = serde_json::from_value(serde_json::json!({"id":"42","text":"[File]","filePath":"/tmp/cache/42.pdf","contentType":"FILE"})).unwrap();
        assert!(original_file_name(&msg).is_none());
        assert_eq!(suggest_media_name(&msg), "line-42.pdf");
        let card = FilePresentation::from_message(&msg);
        assert_eq!(card.detail, "PDF");
        assert_eq!(card.action, "open_viewer");
    }

    #[test]
    fn file_presentation_uses_types_sizes_and_open_routes() {
        for (name, label, icon, action) in [
            (
                "รายงาน.docx",
                "Word · DOCX",
                "x-office-document-symbolic",
                "open_external",
            ),
            (
                "budget.xlsx",
                "Excel · XLSX",
                "x-office-spreadsheet-symbolic",
                "open_external",
            ),
            (
                "slides.odp",
                "PowerPoint · ODP",
                "x-office-presentation-symbolic",
                "open_external",
            ),
            (
                "report.PDF",
                "PDF",
                "application-pdf-symbolic",
                "open_viewer",
            ),
            (
                "report.pdf.docx",
                "Word · DOCX",
                "x-office-document-symbolic",
                "open_external",
            ),
            (
                "backup.zip",
                "file_type_archive · ZIP",
                "package-x-generic-symbolic",
                "open_external",
            ),
            (
                "notes.txt",
                "file_type_text · TXT",
                "text-x-generic-symbolic",
                "open_viewer",
            ),
            (
                "custom.xyz",
                "XYZ",
                "text-x-generic-symbolic",
                "open_external",
            ),
        ] {
            let msg: MessageInfo = serde_json::from_value(
                serde_json::json!({"fileName":name,"fileSize":2048,"contentType":"FILE"}),
            )
            .unwrap();
            let card = FilePresentation::from_message(&msg);
            assert_eq!(card.name, name);
            assert_eq!(card.detail, format!("{label} · 2 KB"));
            assert_eq!(card.icon, icon);
            assert_eq!(card.action, action);
        }
        let msg: MessageInfo = serde_json::from_value(
            serde_json::json!({"text":"[File]","fileName":"[File]","contentType":"FILE"}),
        )
        .unwrap();
        let card = FilePresentation::from_message(&msg);
        assert_eq!(card.name, crate::i18n::t("file_name_unknown"));
        assert_eq!(card.detail, crate::i18n::t("file_type_unknown"));
        assert_eq!(card.action, "open_external");
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn gtk_file_card_shows_original_metadata() {
        gtk::init().unwrap();
        let msg: MessageInfo = serde_json::from_value(serde_json::json!({"fileName":"รายงานประจำเดือน.docx","fileSize":2048,"contentType":"FILE"})).unwrap();
        let (card, action) = build_file_card(&msg);
        assert_eq!(card.tooltip_text().as_deref(), Some("รายงานประจำเดือน.docx"));
        assert_eq!(action, "open_external");
        let row = card.child().unwrap().downcast::<gtk::Box>().unwrap();
        let icon = row.first_child().unwrap().downcast::<gtk::Image>().unwrap();
        assert_eq!(
            icon.icon_name().as_deref(),
            Some("x-office-document-symbolic")
        );
        let labels = icon.next_sibling().unwrap();
        let name = labels
            .first_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(name.text(), "รายงานประจำเดือน.docx");
        let detail = name
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(detail.text(), "Word · DOCX · 2 KB");
    }
    #[test]
    fn external_attachment_uses_original_extension_and_keeps_payload() {
        let temp =
            std::env::temp_dir().join(format!("line-external-attachment-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let source = temp.join("42.bin");
        let payload = b"PK\x03\x04synthetic-office-document";
        std::fs::write(&source, payload).unwrap();
        let opened =
            prepare_external_attachment(&temp.join("app"), source.to_str().unwrap(), "report.docx")
                .unwrap();
        assert_eq!(opened.extension().unwrap(), "docx");
        assert!(opened.starts_with(temp.join("app/cache/media")));
        assert_eq!(std::fs::read(&opened).unwrap(), payload);
        assert_eq!(std::fs::read(&source).unwrap(), payload);
        assert_eq!(
            prepare_external_attachment(&temp.join("app"), opened.to_str().unwrap(), "report.docx")
                .unwrap(),
            opened
        );
        let duplicate =
            prepare_external_attachment(&temp.join("app"), source.to_str().unwrap(), "report.docx")
                .unwrap();
        assert_ne!(duplicate, opened);
        assert_eq!(std::fs::read(duplicate).unwrap(), payload);
        let safe = prepare_external_attachment(
            &temp.join("app"),
            source.to_str().unwrap(),
            "../../report.docx",
        )
        .unwrap();
        assert_eq!(safe.parent(), opened.parent());
        assert_eq!(safe.extension().unwrap(), "docx");
        assert_eq!(
            prepare_external_attachment(&temp.join("app"), source.to_str().unwrap(), "[File]")
                .unwrap(),
            source
        );
        std::fs::remove_dir_all(temp).unwrap();
    }
}

#[cfg(test)]
mod image_copy_tests {
    use super::*;

    fn source() -> gdk_pixbuf::Pixbuf {
        let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 32, 24).unwrap();
        pixbuf.fill(0x20408080);
        pixbuf
    }

    fn rendered(pixbuf: &gdk_pixbuf::Pixbuf, strokes: &[Annotation]) -> image::RgbaImage {
        let mut png = Vec::new();
        render_annotated_image(pixbuf, strokes)
            .unwrap()
            .write_to_png(&mut png)
            .unwrap();
        image::load_from_memory(&png).unwrap().into_rgba8()
    }

    #[test]
    fn annotated_copy_render_keeps_full_dimensions_and_alpha() {
        let pixbuf = source();
        let before = pixbuf.read_pixel_bytes();
        let original = rendered(&pixbuf, &[]);
        assert_eq!(original.dimensions(), (32, 24));
        assert_eq!(original.get_pixel(0, 0).0, [32, 64, 128, 128]);
        let strokes = vec![Annotation {
            color: (1.0, 0.0, 0.0, 1.0),
            width: 4.0,
            geometry: AnnotationGeometry::Freehand(vec![(4.0, 10.0), (28.0, 10.0)]),
        }];
        let edited = rendered(&pixbuf, &strokes);
        assert_eq!(edited.dimensions(), (32, 24));
        assert_eq!(edited.get_pixel(16, 10).0, [255, 0, 0, 255]);
        assert_eq!(edited.get_pixel(0, 0), original.get_pixel(0, 0));
        assert_eq!(pixbuf.read_pixel_bytes(), before);
    }

    fn outline(geometry: AnnotationGeometry) -> Annotation {
        Annotation {
            color: (1.0, 0.0, 0.0, 1.0),
            width: 2.0,
            geometry,
        }
    }

    #[test]
    fn rectangle_and_circle_are_unfilled_exact_outlines_in_every_drag_direction() {
        let pixbuf = source();
        let before = pixbuf.read_pixel_bytes();
        let original = rendered(&pixbuf, &[]);
        let rectangle = |start, end| outline(AnnotationGeometry::Rectangle { start, end });
        let expected = rendered(&pixbuf, &[rectangle((4.0, 4.0), (20.0, 20.0))]);
        for (start, end) in [
            ((4.0, 4.0), (20.0, 20.0)),
            ((20.0, 20.0), (4.0, 4.0)),
            ((4.0, 20.0), (20.0, 4.0)),
            ((20.0, 4.0), (4.0, 20.0)),
        ] {
            assert_eq!(rendered(&pixbuf, &[rectangle(start, end)]), expected);
        }
        for (x, y) in [(4, 12), (20, 12), (12, 4), (12, 20)] {
            assert_eq!(expected.get_pixel(x, y).0, [255, 0, 0, 255]);
        }
        assert_eq!(expected.get_pixel(12, 12), original.get_pixel(12, 12));
        let circle = |start, end| outline(AnnotationGeometry::Circle { start, end });
        let round = rendered(&pixbuf, &[circle((4.0, 4.0), (20.0, 12.0))]);
        for (start, end) in [
            ((20.0, 20.0), (4.0, 12.0)),
            ((4.0, 20.0), (20.0, 12.0)),
            ((20.0, 4.0), (4.0, 12.0)),
        ] {
            assert_eq!(rendered(&pixbuf, &[circle(start, end)]), round);
        }
        for (x, y) in [(4, 12), (19, 12), (12, 4), (12, 19)] {
            assert!(round.get_pixel(x, y).0[0] > 200);
        }
        for (x, y) in [(12, 12), (4, 4), (20, 20), (0, 0)] {
            assert_eq!(round.get_pixel(x, y), original.get_pixel(x, y));
        }
        assert_eq!(round.dimensions(), (32, 24));
        for geometry in [
            AnnotationGeometry::Circle {
                start: (4.0, 4.0),
                end: (4.0, 4.0),
            },
            AnnotationGeometry::Rectangle {
                start: (4.0, 4.0),
                end: (4.0, 20.0),
            },
        ] {
            assert_eq!(rendered(&pixbuf, &[outline(geometry)]), original);
        }
        let temp = std::env::temp_dir().join(format!("line-shape-export-{}", std::process::id()));
        let annotations = [circle((4.0, 4.0), (20.0, 12.0))];
        let file = export_annotated_image(&pixbuf, &annotations, &temp).unwrap();
        assert_eq!(image::open(file).unwrap().into_rgba8(), round);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 32, 24).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        gdk::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, &pixbuf, 0.0, 0.0);
        cr.paint().unwrap();
        paint_annotation(&cr, &annotations[0]).unwrap();
        let mut preview = Vec::new();
        surface.write_to_png(&mut preview).unwrap();
        assert_eq!(
            image::load_from_memory(&preview).unwrap().into_rgba8(),
            round
        );
        assert_eq!(pixbuf.read_pixel_bytes(), before);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    #[ignore = "requires an isolated GTK display"]
    fn gtk_image_copy_toolbar_keyboard_active_undo_clear() {
        libadwaita::init().unwrap();
        let temp = std::env::temp_dir().join(format!("line-image-copy-{}", std::process::id()));
        std::fs::create_dir_all(temp.join("protocol")).unwrap();
        let fake = temp.join("protocol/line-gtk-protocol");
        let requests = temp.join("requests");
        std::fs::write(&fake, format!("#!/usr/bin/python3\nimport sys\nfor line in sys.stdin:\n with open({:?}, 'a') as f: f.write(line)\n", requests)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let data = temp.join("data");
        AppConfig {
            tray_enabled: false,
            animations: false,
            notifications: false,
            ..AppConfig::default()
        }
        .save(&data);
        let app = Application::builder()
            .application_id("dev.linegtk.ImageCopyFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let sidecar = Rc::new(Sidecar::spawn(&temp, &data).unwrap());
        let (state, _) = build_state(
            &app,
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            data.clone(),
            sidecar,
        );
        let path = temp.join("original.png");
        source().savev(&path, "png", &[]).unwrap();
        let original_file = std::fs::read(&path).unwrap();
        let win = gtk::Window::new();
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        append_image_viewer(&state, &bar, &body, path.to_str().unwrap(), &win);
        let mut buttons = Vec::new();
        let mut child = bar.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            buttons.push(widget);
        }
        let button = |name: &str| {
            buttons
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
                .find(|b| b.icon_name().as_deref() == Some(name))
                .unwrap()
        };
        let copy = button("edit-copy-symbolic");
        let clipboard = win.clipboard();
        let read = || {
            let texture = glib::MainContext::default()
                .block_on(clipboard.read_texture_future())
                .unwrap()
                .unwrap();
            image::load_from_memory(texture.save_to_png_bytes().as_ref())
                .unwrap()
                .into_rgba8()
        };
        copy.emit_clicked();
        let original = read();
        assert_eq!(original.dimensions(), (32, 24));
        assert_eq!(original.get_pixel(0, 0).0, [32, 64, 128, 128]);
        let draw = buttons
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::ToggleButton>().ok())
            .unwrap();
        draw.set_active(true);
        let area = body.first_child().unwrap();
        let controllers = area.observe_controllers();
        let drag = (0..controllers.n_items())
            .find_map(|i| {
                controllers
                    .item(i)
                    .unwrap()
                    .downcast::<gtk::GestureDrag>()
                    .ok()
            })
            .unwrap();
        drag.emit_by_name::<()>("drag-begin", &[&10.0_f64, &10.0_f64]);
        copy.emit_clicked();
        let active = read();
        assert_ne!(active.get_pixel(10, 10), original.get_pixel(10, 10));
        drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
        copy.emit_clicked();
        assert_eq!(read(), active);
        button("zoom-in-symbolic").emit_clicked();
        copy.emit_clicked();
        assert_eq!(read(), active);
        button("edit-undo-symbolic").emit_clicked();
        copy.emit_clicked();
        assert_eq!(read(), original);
        drag.emit_by_name::<()>("drag-begin", &[&15.0_f64, &10.0_f64]);
        drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
        button("edit-clear-all-symbolic").emit_clicked();
        let controllers = win.observe_controllers();
        let keys = (0..controllers.n_items())
            .find_map(|i| {
                controllers
                    .item(i)
                    .unwrap()
                    .downcast::<gtk::EventControllerKey>()
                    .ok()
            })
            .unwrap();
        let handled = keys.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::c, &0_u32, &gdk::ModifierType::CONTROL_MASK],
        );
        assert!(handled);
        assert_eq!(read(), original);
        button("zoom-out-symbolic").emit_clicked();
        let shape_button = |label: &str| {
            buttons
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk::ToggleButton>().ok())
                .find(|b| b.label().as_deref() == Some(label))
                .unwrap()
        };
        let rectangle = shape_button("□");
        let circle = shape_button("○");
        rectangle.set_active(true);
        assert!(!draw.is_active());
        drag.emit_by_name::<()>("drag-begin", &[&4.0_f64, &4.0_f64]);
        drag.emit_by_name::<()>("drag-update", &[&12.0_f64, &12.0_f64]);
        copy.emit_clicked();
        let preview = read();
        assert_ne!(preview.get_pixel(16, 10), original.get_pixel(16, 10));
        assert_eq!(preview.get_pixel(10, 10), original.get_pixel(10, 10));
        drag.emit_by_name::<()>("drag-end", &[&20.0_f64, &16.0_f64]);
        copy.emit_clicked();
        let completed = read();
        assert_ne!(completed.get_pixel(24, 10), original.get_pixel(24, 10));
        assert_ne!(completed, preview);
        button("edit-undo-symbolic").emit_clicked();
        copy.emit_clicked();
        assert_eq!(read(), original);
        circle.set_active(true);
        assert!(!rectangle.is_active());
        let green = buttons
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
            .find(|b| b.has_css_class("line-draw-c-green"))
            .unwrap();
        green.emit_clicked();
        assert!(circle.is_active());
        assert!(!draw.is_active());
        drag.emit_by_name::<()>("drag-begin", &[&4.0_f64, &4.0_f64]);
        button("zoom-in-symbolic").emit_clicked();
        drag.emit_by_name::<()>("drag-end", &[&16.0_f64, &8.0_f64]);
        copy.emit_clicked();
        let round = read();
        assert!(round.get_pixel(12, 4).0[1] > 140);
        assert_eq!(round.get_pixel(12, 12), original.get_pixel(12, 12));
        let expected = rendered(
            &source(),
            &[Annotation {
                color: (0.26, 0.63, 0.28, 1.0),
                width: 6.0,
                geometry: AnnotationGeometry::Circle {
                    start: (4.0, 4.0),
                    end: (20.0, 12.0),
                },
            }],
        );
        assert_eq!(round, expected);
        button("edit-clear-all-symbolic").emit_clicked();
        circle.set_active(false);
        drag.emit_by_name::<()>("drag-begin", &[&0.0_f64, &0.0_f64]);
        drag.emit_by_name::<()>("drag-end", &[&10.0_f64, &5.0_f64]);
        rectangle.set_active(true);
        drag.emit_by_name::<()>("drag-begin", &[&15.0_f64, &10.0_f64]);
        drag.emit_by_name::<()>("drag-end", &[&20.0_f64, &15.0_f64]);
        copy.emit_clicked();
        let expected = rendered(
            &source(),
            &[Annotation {
                color: (0.26, 0.63, 0.28, 1.0),
                width: 4.8,
                geometry: AnnotationGeometry::Rectangle {
                    start: (4.0, 4.0),
                    end: (20.0, 16.0),
                },
            }],
        );
        assert_eq!(read(), expected);
        button("edit-clear-all-symbolic").emit_clicked();
        drag.emit_by_name::<()>("drag-begin", &[&15.0_f64, &10.0_f64]);
        drag.emit_by_name::<()>("drag-update", &[&10.0_f64, &10.0_f64]);
        circle.set_active(true);
        drag.emit_by_name::<()>("drag-end", &[&20.0_f64, &15.0_f64]);
        copy.emit_clicked();
        assert_eq!(read(), original);
        assert!(!button("edit-undo-symbolic").is_sensitive());
        drag.emit_by_name::<()>("drag-begin", &[&15.0_f64, &10.0_f64]);
        drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
        copy.emit_clicked();
        assert_eq!(read(), original);
        assert!(!button("edit-undo-symbolic").is_sensitive());
        assert_eq!(std::fs::read(&path).unwrap(), original_file);
        assert!(!data.join("cache/media").exists());
        assert!(!requests.exists() || std::fs::read_to_string(&requests).unwrap().is_empty());
        clipboard
            .set_content(None::<&gdk::ContentProvider>)
            .unwrap();
        win.close();
        std::fs::remove_dir_all(temp).unwrap();
    }
}
