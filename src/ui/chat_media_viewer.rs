use super::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

#[derive(Clone, Debug, PartialEq, Eq)]
enum MediaDownload {
    Idle,
    Loading { request_id: u64 },
    Ready { path: String },
    Failed { error: String },
}

#[derive(Clone, Debug)]
struct ChatMedia {
    message: MessageInfo,
    download: MediaDownload,
}

struct ChatMedias {
    items: Vec<ChatMedia>,
    selected: usize,
}

pub(super) struct ChatMediaSession {
    media: media::MediaViewer,
    source_chat_mid: String,
    model: RefCell<ChatMedias>,
    closed: Cell<bool>,
    rendered: RefCell<Option<(usize, String)>>,
}

#[derive(Clone)]
pub(super) struct ChatMediaViewer(pub(super) Rc<ChatMediaSession>);

impl ChatMediaViewer {
    pub(super) fn is_closed(&self) -> bool {
        self.0.closed.get()
    }

    #[cfg(test)]
    pub(super) fn window(&self) -> gtk::Window {
        self.0.media.window.clone()
    }
}

pub(super) fn is_chat_media(msg: &MessageInfo) -> bool {
    (msg.content_type.eq_ignore_ascii_case("IMAGE")
        || msg.content_type.eq_ignore_ascii_case("VIDEO"))
        && !msg.id.is_empty()
}

fn resolve_freshest_message(
    base: &MessageInfo,
    clicked: &MessageInfo,
    fresh_cache: Option<&HashMap<String, MessageInfo>>,
) -> MessageInfo {
    let mut resolved = if let Some(fresh) = fresh_cache.and_then(|c| c.get(&base.id)) {
        fresh.clone()
    } else if base.id == clicked.id {
        clicked.clone()
    } else {
        base.clone()
    };
    if base.id == clicked.id {
        if clicked.image_path.is_some() && resolved.image_path.is_none() {
            resolved.image_path = clicked.image_path.clone();
        }
        if clicked.file_path.is_some() && resolved.file_path.is_none() {
            resolved.file_path = clicked.file_path.clone();
        }
    }
    resolved
}

fn build_snapshot(
    chat_media: &[MessageInfo],
    clicked: &MessageInfo,
    fresh_cache: Option<&HashMap<String, MessageInfo>>,
) -> (Vec<ChatMedia>, usize) {
    let mut messages: Vec<MessageInfo> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for msg in chat_media {
        if is_chat_media(msg) && seen.insert(msg.id.clone()) {
            messages.push(resolve_freshest_message(msg, clicked, fresh_cache));
        }
    }
    if is_chat_media(clicked) && seen.insert(clicked.id.clone()) {
        messages.push(resolve_freshest_message(clicked, clicked, fresh_cache));
    }

    messages.sort_by(|a, b| {
        a.created_time
            .cmp(&b.created_time)
            .then_with(|| a.id.cmp(&b.id))
    });

    let selected = messages
        .iter()
        .position(|m| m.id == clicked.id)
        .unwrap_or(0);

    let items = messages
        .into_iter()
        .map(|message| {
            let download = if let Some(path) = media::local_media_path(&message, true) {
                MediaDownload::Ready { path }
            } else {
                MediaDownload::Idle
            };
            ChatMedia { message, download }
        })
        .collect();

    (items, selected)
}

fn step_selected(current: usize, total: usize, next: bool) -> Option<usize> {
    if next {
        if current + 1 < total {
            Some(current + 1)
        } else {
            None
        }
    } else if current > 0 {
        Some(current - 1)
    } else {
        None
    }
}

pub(super) fn open_chat_media_viewer(state: &AppState, clicked: &MessageInfo) {
    let source_chat_mid = state
        .current_chat
        .borrow()
        .clone()
        .unwrap_or_else(|| clicked.chat_mid().to_string());
    let loaded = state.chat_media.borrow().clone();
    let is_current = state.current_chat.borrow().as_deref() == Some(&source_chat_mid);
    let fresh_cache = if is_current {
        Some(state.media_msgs.borrow())
    } else {
        None
    };
    let (items, selected) = build_snapshot(&loaded, clicked, fresh_cache.as_deref());
    let session = Rc::new(ChatMediaSession {
        media: media::MediaViewer::new(state),
        source_chat_mid,
        model: RefCell::new(ChatMedias { items, selected }),
        closed: Cell::new(false),
        rendered: RefCell::new(None),
    });
    let viewer = ChatMediaViewer(session);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = state.clone();
    let weak_session = Rc::downgrade(&viewer.0);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::SHIFT_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let Some(session) = weak_session.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let v = ChatMediaViewer(session);
        match key {
            gdk::Key::Left => {
                navigate_chat_media(&s, &v, false);
                glib::Propagation::Stop
            }
            gdk::Key::Right => {
                navigate_chat_media(&s, &v, true);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    viewer.0.media.window.add_controller(keys.clone());

    let s = state.clone();
    let weak_session = Rc::downgrade(&viewer.0);
    let keys_weak = keys.downgrade();
    viewer.0.media.window.connect_close_request(move |win| {
        if let Some(k) = keys_weak.upgrade() {
            win.remove_controller(&k);
        }
        if let Some(session) = weak_session.upgrade() {
            session.closed.set(true);
            session.media.clear();
        }
        s.chat_media_viewers.borrow_mut().retain(|v| !v.is_closed());
        glib::Propagation::Proceed
    });

    state.chat_media_viewers.borrow_mut().push(viewer.clone());
    refresh_chat_media(state, &viewer);
    viewer.0.media.window.present();
}

fn navigate_chat_media(state: &AppState, viewer: &ChatMediaViewer, next: bool) {
    if viewer.is_closed() {
        return;
    }
    let (current, total) = {
        let model = viewer.0.model.borrow();
        (model.selected, model.items.len())
    };
    if let Some(next_idx) = step_selected(current, total, next) {
        viewer.0.model.borrow_mut().selected = next_idx;
        refresh_chat_media(state, viewer);
    }
}

fn request_chat_media_download(state: &AppState, viewer: &ChatMediaViewer, index: usize) {
    if viewer.is_closed() {
        return;
    }
    let (msg_id, source_chat) = {
        let model = viewer.0.model.borrow();
        if index >= model.items.len() {
            return;
        }
        (
            model.items[index].message.id.clone(),
            viewer.0.source_chat_mid.clone(),
        )
    };
    if msg_id.is_empty() {
        return;
    }
    match state.sidecar.download_media(&source_chat, &msg_id) {
        Ok(request_id) => {
            if let Some(item) = viewer.0.model.borrow_mut().items.get_mut(index) {
                item.download = MediaDownload::Loading { request_id };
            }
            state.pending.borrow_mut().insert(
                request_id,
                Pending::ChatMediaDownload {
                    viewer: Rc::downgrade(&viewer.0),
                    message_id: msg_id,
                },
            );
        }
        Err(e) => {
            if let Some(item) = viewer.0.model.borrow_mut().items.get_mut(index) {
                item.download = MediaDownload::Failed {
                    error: e.to_string(),
                };
            }
        }
    }
}

fn retry_chat_media(state: &AppState, viewer: &ChatMediaViewer) {
    if viewer.is_closed() {
        return;
    }
    let selected = viewer.0.model.borrow().selected;
    if let Some(item) = viewer.0.model.borrow_mut().items.get_mut(selected) {
        item.download = MediaDownload::Idle;
    }
    request_chat_media_download(state, viewer, selected);
    refresh_chat_media(state, viewer);
}

fn refresh_chat_media(state: &AppState, viewer: &ChatMediaViewer) {
    if viewer.is_closed() {
        return;
    }
    let (selected, total, item) = {
        let model = viewer.0.model.borrow();
        if model.items.is_empty() {
            return;
        }
        let selected = model.selected.min(model.items.len() - 1);
        let total = model.items.len();
        let item = model.items[selected].clone();
        (selected, total, item)
    };

    if matches!(item.download, MediaDownload::Idle) {
        if let Some(path) = media::local_media_path(&item.message, true) {
            if let Some(it) = viewer.0.model.borrow_mut().items.get_mut(selected) {
                it.download = MediaDownload::Ready { path };
            }
        } else {
            request_chat_media_download(state, viewer, selected);
        }
    }

    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    navigation.set_halign(gtk::Align::Center);
    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    previous.set_tooltip_text(Some(&crate::i18n::t("album_previous")));
    previous.set_sensitive(selected > 0);
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some(&crate::i18n::t("album_next")));
    next.set_sensitive(selected + 1 < total);
    let counter = gtk::Label::new(Some(&format!("{} / {}", selected + 1, total)));
    navigation.append(&previous);
    navigation.append(&counter);
    navigation.append(&next);
    let weak_session = Rc::downgrade(&viewer.0);
    for (btn, is_next) in [(previous, false), (next, true)] {
        let s = state.clone();
        let w = weak_session.clone();
        btn.connect_clicked(move |_| {
            if let Some(session) = w.upgrade() {
                navigate_chat_media(&s, &ChatMediaViewer(session), is_next);
            }
        });
    }

    let item = viewer.0.model.borrow().items[selected].clone();
    let ready = if let MediaDownload::Ready { path } = &item.download {
        Some((selected, path.clone()))
    } else {
        None
    };

    if ready != *viewer.0.rendered.borrow() || ready.is_none() {
        viewer.0.media.clear();
        if let Some((_, path)) = &ready {
            viewer.0.media.show(
                state,
                path,
                &item.message.content_type,
                &media::suggest_media_name(&item.message),
                Some(&navigation),
                Some(&item.message.id),
            );
        } else {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
            root.append(&navigation);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.set_halign(gtk::Align::Center);
            let error = match &item.download {
                MediaDownload::Failed { error } => Some(error.clone()),
                _ => None,
            };
            let label = gtk::Label::new(Some(
                &error
                    .clone()
                    .unwrap_or_else(|| crate::i18n::t("media_downloading")),
            ));
            label.set_wrap(true);
            let retry = gtk::Button::with_label(&crate::i18n::t("album_retry"));
            retry.set_visible(error.is_some());
            let s = state.clone();
            let w = Rc::downgrade(&viewer.0);
            retry.connect_clicked(move |_| {
                if let Some(session) = w.upgrade() {
                    retry_chat_media(&s, &ChatMediaViewer(session));
                }
            });
            row.append(&label);
            row.append(&retry);
            root.append(&row);
            viewer.0.media.window.set_child(Some(&root));
        }
        *viewer.0.rendered.borrow_mut() = ready;
    } else if let Some(root) = viewer.0.media.window.child().and_downcast::<gtk::Box>()
        && let Some(bar) = root.first_child()
        && let Some(old) = bar.next_sibling()
    {
        root.remove(&old);
        root.insert_child_after(&navigation, Some(&bar));
    }
}

pub(super) fn finish_chat_media_download(
    state: &AppState,
    request_id: u64,
    weak_viewer: &Weak<ChatMediaSession>,
    message_id: &str,
    result: Result<String, String>,
) {
    let Some(session) = weak_viewer.upgrade() else {
        return;
    };
    if session.closed.get() {
        return;
    }
    let mut model = session.model.borrow_mut();
    let Some((index, item)) = model
        .items
        .iter_mut()
        .enumerate()
        .find(|(_, it)| it.message.id == message_id)
    else {
        return;
    };

    match item.download {
        MediaDownload::Loading {
            request_id: expected_id,
        } if expected_id == request_id => {}
        _ => return,
    }

    match result {
        Ok(path) => {
            item.download = MediaDownload::Ready { path: path.clone() };
            if item.message.content_type.eq_ignore_ascii_case("IMAGE") {
                item.message.image_path = Some(path.clone());
            }
            item.message.file_path = Some(path.clone());
            if state.current_chat.borrow().as_deref() == Some(&session.source_chat_mid) {
                if let Some(msg) = state.media_msgs.borrow_mut().get_mut(message_id) {
                    if msg.content_type.eq_ignore_ascii_case("IMAGE") {
                        msg.image_path = Some(path.clone());
                    }
                    msg.file_path = Some(path.clone());
                }
                if item.message.content_type.eq_ignore_ascii_case("IMAGE") {
                    state
                        .media_ready_paths
                        .borrow_mut()
                        .insert(message_id.to_string(), path.clone());
                }
                for chat_msg in state.chat_media.borrow_mut().iter_mut() {
                    if chat_msg.id == message_id {
                        if chat_msg.content_type.eq_ignore_ascii_case("IMAGE") {
                            chat_msg.image_path = Some(path.clone());
                        }
                        chat_msg.file_path = Some(path.clone());
                    }
                }
            }
        }
        Err(error) => {
            item.download = MediaDownload::Failed { error };
        }
    }
    let is_selected = model.selected == index;
    drop(model);
    if is_selected {
        refresh_chat_media(state, &ChatMediaViewer(session));
    }
}

pub(super) fn fail_pending_chat_media(state: &AppState, error: &str) {
    let mut pending = state.pending.borrow_mut();
    let image_ids: Vec<u64> = pending
        .iter()
        .filter_map(|(id, p)| match p {
            Pending::ChatMediaDownload { .. } => Some(*id),
            _ => None,
        })
        .collect();
    let mut to_fail = Vec::new();
    for id in image_ids {
        if let Some(Pending::ChatMediaDownload { viewer, message_id }) = pending.remove(&id) {
            to_fail.push((viewer, message_id, id));
        }
    }
    drop(pending);
    for (viewer, message_id, id) in to_fail {
        finish_chat_media_download(state, id, &viewer, &message_id, Err(error.to_string()));
    }

    for viewer in state.chat_media_viewers.borrow().iter() {
        if viewer.is_closed() {
            continue;
        }
        let mut model = viewer.0.model.borrow_mut();
        let mut changed = false;
        for item in model.items.iter_mut() {
            if matches!(item.download, MediaDownload::Loading { .. }) {
                item.download = MediaDownload::Failed {
                    error: error.to_string(),
                };
                changed = true;
            }
        }
        drop(model);
        if changed {
            refresh_chat_media(state, viewer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_msg(id: &str, created_time: i64, content_type: &str) -> MessageInfo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "createdTime": created_time,
            "contentType": content_type,
            "text": "[Image]"
        }))
        .unwrap()
    }

    #[test]
    fn deduplicates_and_orders_chat_media_chronologically() {
        let loaded = vec![
            make_msg("img-3", 300, "IMAGE"),
            make_msg("img-1", 100, "IMAGE"),
            make_msg("img-2b", 200, "IMAGE"),
            make_msg("img-2a", 200, "IMAGE"),
            make_msg("img-dup", 150, "IMAGE"),
            make_msg("img-dup", 150, "IMAGE"),
            make_msg("sticker-1", 120, "STICKER"),
            make_msg("video-1", 130, "VIDEO"),
            make_msg("file-1", 140, "FILE"),
            make_msg("", 160, "IMAGE"),
        ];
        let clicked = make_msg("img-2a", 200, "IMAGE");
        let (items, selected) = build_snapshot(&loaded, &clicked, None);

        let ids: Vec<&str> = items.iter().map(|item| item.message.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["img-1", "video-1", "img-dup", "img-2a", "img-2b", "img-3"]
        );
        assert_eq!(selected, 3);
        assert_eq!(items[selected].message.id, "img-2a");
    }

    #[test]
    fn snapshot_creation_includes_clicked_image_if_absent() {
        let loaded = vec![
            make_msg("img-1", 100, "IMAGE"),
            make_msg("img-3", 300, "IMAGE"),
        ];
        let clicked = make_msg("img-missing", 200, "IMAGE");
        let (items, selected) = build_snapshot(&loaded, &clicked, None);

        let ids: Vec<&str> = items.iter().map(|item| item.message.id.as_str()).collect();
        assert_eq!(ids, vec!["img-1", "img-missing", "img-3"]);
        assert_eq!(selected, 1);
        assert_eq!(items[selected].message.id, "img-missing");
    }

    #[test]
    fn snapshot_excludes_stickers_and_non_media_and_empty_ids() {
        let loaded = vec![
            make_msg("sticker", 10, "STICKER"),
            make_msg("video", 20, "VIDEO"),
            make_msg("audio", 30, "AUDIO"),
            make_msg("file", 40, "FILE"),
            make_msg("", 50, "IMAGE"),
            make_msg("img", 60, "image"),
        ];
        let clicked = make_msg("img", 60, "image");
        let (items, selected) = build_snapshot(&loaded, &clicked, None);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].message.id, "video");
        assert_eq!(items[1].message.id, "img");
        assert_eq!(selected, 1);
    }

    #[test]
    fn snapshot_overlays_fresh_cache_and_preserves_clicked_over_stale() {
        let temp = std::env::temp_dir().join(format!("snap-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp);
        let full_img = temp.join("msg1.full.png");
        std::fs::write(&full_img, b"fake-png-data").unwrap();

        let stale_loaded = vec![
            make_msg("img-1", 100, "IMAGE"),
            make_msg("img-2", 200, "IMAGE"),
        ];
        let mut clicked = make_msg("img-1", 100, "IMAGE");
        clicked.image_path = Some(full_img.to_str().unwrap().to_string());

        let (items_clicked_fresh, _) = build_snapshot(&stale_loaded, &clicked, None);
        assert_eq!(
            items_clicked_fresh[0].download,
            MediaDownload::Ready {
                path: full_img.to_str().unwrap().to_string()
            }
        );

        let mut media_msgs = HashMap::new();
        let mut fresh_msg2 = make_msg("img-2", 200, "IMAGE");
        fresh_msg2.image_path = Some(full_img.to_str().unwrap().to_string());
        media_msgs.insert("img-2".to_string(), fresh_msg2);

        let default_clicked = make_msg("img-1", 100, "IMAGE");
        let (items_cache_fresh, _) =
            build_snapshot(&stale_loaded, &default_clicked, Some(&media_msgs));
        assert_eq!(
            items_cache_fresh[1].download,
            MediaDownload::Ready {
                path: full_img.to_str().unwrap().to_string()
            }
        );

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn snapshot_thumbnail_remains_idle() {
        let temp = std::env::temp_dir().join(format!("snap-thumb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp);
        let thumb_path = temp.join("img.thumb.jpg");
        std::fs::write(&thumb_path, b"thumb-data").unwrap();

        let mut msg = make_msg("img-thumb", 100, "IMAGE");
        msg.image_path = Some(thumb_path.to_str().unwrap().to_string());

        let (items, _) = build_snapshot(&[msg.clone()], &msg, None);
        assert_eq!(items[0].download, MediaDownload::Idle);

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn video_snapshot_uses_full_file_and_preserves_thumbnail() {
        let temp = std::env::temp_dir().join(format!("video-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let thumb = temp.join("clip.thumb.jpg");
        let full = temp.join("clip.mp4");
        std::fs::write(&thumb, b"thumbnail").unwrap();
        std::fs::write(&full, b"video").unwrap();
        let mut video = make_msg("clip", 200, "VIDEO");
        video.image_path = Some(thumb.to_string_lossy().into_owned());
        let (items, _) = build_snapshot(&[video.clone()], &video, None);
        assert_eq!(items[0].download, MediaDownload::Idle);
        video.file_path = Some(full.to_string_lossy().into_owned());
        let image = make_msg("image", 100, "IMAGE");
        let (items, selected) = build_snapshot(&[video.clone(), image], &video, None);
        assert_eq!(selected, 1);
        assert_eq!(
            items[selected].download,
            MediaDownload::Ready {
                path: full.to_string_lossy().into_owned()
            }
        );
        assert_eq!(items[selected].message.image_path, video.image_path);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn boundary_navigation_step_is_noop_at_both_ends() {
        let total = 3;
        assert_eq!(step_selected(0, total, false), None);
        assert_eq!(step_selected(0, total, true), Some(1));
        assert_eq!(step_selected(1, total, false), Some(0));
        assert_eq!(step_selected(1, total, true), Some(2));
        assert_eq!(step_selected(2, total, true), None);
        assert_eq!(step_selected(2, total, false), Some(1));
    }

    #[test]
    fn step_selected_handles_single_item() {
        assert_eq!(step_selected(0, 1, false), None);
        assert_eq!(step_selected(0, 1, true), None);
    }

    fn button_with_tooltip(widget: &gtk::Widget, text: &str) -> gtk::Button {
        descendants(widget)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.tooltip_text().as_deref() == Some(text))
            .unwrap()
    }

    fn button_with_label(widget: &gtk::Widget, text: &str) -> gtk::Button {
        descendants(widget)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.label().as_deref() == Some(text))
            .unwrap()
    }

    fn label_containing(widget: &gtk::Widget, substring: &str) -> gtk::Label {
        descendants(widget)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .find(|l| l.text().contains(substring))
            .unwrap()
    }

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut widgets = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(w) = child {
            widgets.extend(descendants(&w));
            child = w.next_sibling();
        }
        widgets
    }

    fn video_stream(win: &gtk::Window) -> gtk::MediaStream {
        descendants(win.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Video>().ok())
            .unwrap()
            .media_stream()
            .unwrap()
    }

    fn drain() {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
    }

    fn wait_for(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !condition() && std::time::Instant::now() < deadline {
            drain();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(condition());
    }

    fn requests(path: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn respond(
        state: &AppState,
        id: u64,
        ok: bool,
        result: serde_json::Value,
        error: Option<String>,
    ) {
        handle_event(
            state,
            ProtocolEvent::Response {
                id,
                ok,
                result,
                error,
            },
        );
        drain();
    }

    fn press(win: &gtk::Window, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let controllers = win.observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(keys) = controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
                && keys.emit_by_name::<bool>("key-pressed", &[&key, &0_u32, &modifiers])
            {
                return true;
            }
        }
        false
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn gtk_chat_media_viewer_navigation_retry_and_isolation() {
        gtk_viewer_fixture(false);
    }

    #[test]
    #[ignore = "requires a GTK display, ffmpeg and a GTK video backend"]
    fn gtk_chat_media_viewer_video_playback_navigation_and_close() {
        gtk_viewer_fixture(true);
    }

    fn gtk_viewer_fixture(with_video: bool) {
        libadwaita::init().unwrap();
        let temp =
            std::env::temp_dir().join(format!("line-chat-viewer-fixture-{}", std::process::id()));
        std::fs::create_dir_all(temp.join("protocol")).unwrap();
        let log = temp.join("requests.jsonl");
        let _ = std::fs::remove_file(&log);
        let fake = temp.join("protocol/line-gtk-protocol");
        std::fs::write(&fake, format!("#!/usr/bin/python3\nimport sys\nfor line in sys.stdin:\n with open({:?}, 'a') as log:\n  log.write(line)\n", log.to_string_lossy())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let data = temp.join("data");
        AppConfig {
            animations: false,
            tray_enabled: false,
            notifications: false,
            language: "en".into(),
            ..AppConfig::default()
        }
        .save(&data);
        let app = Application::builder()
            .application_id("dev.linegtk.ChatViewerFixture")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let sidecar = Rc::new(Sidecar::spawn(&temp, &data).unwrap());
        let (state, _) = build_state(
            &app,
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            data,
            sidecar,
        );
        *state.session_ready.borrow_mut() = true;
        *state.current_chat.borrow_mut() = Some("chat-alpha".into());

        let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 32).unwrap();
        let img0_path = temp.join("msg-0.full.png");
        pixbuf.fill(0xff0000ff);
        pixbuf.savev(&img0_path, "png", &[]).unwrap();
        let img1_path = temp.join(if with_video {
            "msg-1.mp4"
        } else {
            "msg-1.full.png"
        });
        pixbuf.fill(0x00ff00ff);
        pixbuf.savev(&img1_path, "png", &[]).unwrap();
        let img2_path = temp.join(if with_video {
            "msg-2.mp4"
        } else {
            "msg-2.full.png"
        });
        pixbuf.fill(0x0000ffff);
        pixbuf.savev(&img2_path, "png", &[]).unwrap();

        if with_video {
            let output = std::process::Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=size=160x120:rate=15",
                    "-t",
                    "8",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(&img1_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::copy(&img1_path, &img2_path).unwrap();
        }

        let msg0: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "msg-0", "createdTime": 1000, "contentType": "IMAGE", "text": "[Image]",
            "imagePath": img0_path.to_str().unwrap(), "filePath": img0_path.to_str().unwrap()
        }))
        .unwrap();
        let msg1: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "msg-1", "createdTime": 2000, "contentType": if with_video { "VIDEO" } else { "IMAGE" }, "text": "[Media]",
            "imagePath": if with_video { Some(img0_path.to_str().unwrap()) } else { None }
        }))
        .unwrap();
        let msg2: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "msg-2", "createdTime": 3000, "contentType": if with_video { "VIDEO" } else { "IMAGE" }, "text": "[Media]"
        }))
        .unwrap();

        apply_messages(&state, vec![msg0.clone(), msg1.clone(), msg2.clone()]);
        assert_eq!(state.chat_media.borrow().len(), 3);
        let chat_media = state.chat_media.borrow();
        let initial_loaded_ids: Vec<&str> = chat_media.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(initial_loaded_ids, vec!["msg-0", "msg-1", "msg-2"]);
        drop(chat_media);
        drain();

        state.chat_media.borrow_mut()[0].image_path = None;
        state.chat_media.borrow_mut()[0].file_path = None;
        assert!(state.chat_media.borrow()[0].image_path.is_none());
        assert!(state.media_msgs.borrow()["msg-0"].image_path.is_some());

        open_chat_media_viewer(&state, &msg0);
        drain();

        let viewer = state.chat_media_viewers.borrow().last().unwrap().clone();
        let viewer_win = viewer.window();
        assert!(viewer_win.is_visible());
        assert_eq!(
            viewer_win.title().as_deref(),
            Some(crate::i18n::t("media_viewer_title").as_str())
        );
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );
        assert!(
            !requests(&log)
                .iter()
                .any(|r| r["params"]["messageId"] == "msg-0")
        );

        let prev_btn = |win: &gtk::Window| {
            button_with_tooltip(win.upcast_ref(), &crate::i18n::t("album_previous"))
        };
        let next_btn = |win: &gtk::Window| {
            button_with_tooltip(win.upcast_ref(), &crate::i18n::t("album_next"))
        };
        assert!(!prev_btn(&viewer_win).is_sensitive());
        assert!(next_btn(&viewer_win).is_sensitive());

        let initial_child = viewer_win.child().unwrap();
        assert!(press(
            &viewer_win,
            gdk::Key::Left,
            gdk::ModifierType::empty()
        ));
        assert_eq!(viewer_win.child().unwrap(), initial_child);
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );

        assert!(!press(
            &viewer_win,
            gdk::Key::Right,
            gdk::ModifierType::SHIFT_MASK
        ));
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );

        viewer_win.close();
        drain();
        assert_eq!(state.chat_media_viewers.borrow().len(), 0);
        open_chat_media_viewer(&state, &msg0);
        drain();
        let viewer = state.chat_media_viewers.borrow().last().unwrap().clone();
        let viewer_win = viewer.window();
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );
        assert!(
            !requests(&log)
                .iter()
                .any(|r| r["params"]["messageId"] == "msg-0")
        );

        let find_download = |msg_id: &str, min_id: u64| {
            wait_for(|| {
                requests(&log).iter().any(|r| {
                    r["method"] == "download_media"
                        && r["params"]["messageId"] == msg_id
                        && r["id"].as_u64().unwrap_or(0) >= min_id
                })
            });
            requests(&log)
                .into_iter()
                .find(|r| {
                    r["method"] == "download_media"
                        && r["params"]["messageId"] == msg_id
                        && r["id"].as_u64().unwrap_or(0) >= min_id
                })
                .unwrap()
        };

        assert!(press(
            &viewer_win,
            gdk::Key::Right,
            gdk::ModifierType::empty()
        ));
        drain();
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "2 / 3").text(),
            "2 / 3"
        );
        let req1 = find_download("msg-1", 0);
        assert_eq!(req1["params"]["chatMid"], "chat-alpha");
        let id1 = req1["id"].as_u64().unwrap();

        assert!(press(
            &viewer_win,
            gdk::Key::Left,
            gdk::ModifierType::empty()
        ));
        drain();
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );
        let child_at_msg0 = viewer_win.child().unwrap();

        respond(
            &state,
            id1,
            true,
            serde_json::json!({"path": img1_path.to_str().unwrap()}),
            None,
        );
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "1 / 3").text(),
            "1 / 3"
        );
        assert_eq!(viewer_win.child().unwrap(), child_at_msg0);

        assert!(press(
            &viewer_win,
            gdk::Key::Right,
            gdk::ModifierType::empty()
        ));
        drain();
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "2 / 3").text(),
            "2 / 3"
        );
        let dl_btn =
            button_with_tooltip(viewer_win.upcast_ref(), &crate::i18n::t("media_download"));
        assert!(dl_btn.is_visible());
        assert_eq!(
            requests(&log)
                .iter()
                .filter(|r| r["params"]["messageId"] == "msg-1")
                .count(),
            1
        );

        let playing_stream = if with_video {
            let stream = video_stream(&viewer_win);
            wait_for(|| stream.is_prepared() || stream.error().is_some());
            assert!(stream.error().is_none(), "{:?}", stream.error());
            wait_for(|| stream.timestamp() > 0);
            assert_eq!(
                state.media_msgs.borrow()["msg-1"].image_path.as_deref(),
                img0_path.to_str()
            );
            assert!(!state.media_ready_paths.borrow().contains_key("msg-1"));
            assert_eq!(
                state.chat_media.borrow()[1].image_path.as_deref(),
                img0_path.to_str()
            );
            Some(stream)
        } else {
            None
        };
        next_btn(&viewer_win).emit_clicked();
        if let Some(stream) = playing_stream {
            assert!(!stream.is_playing());
        }
        drain();
        assert_eq!(
            label_containing(viewer_win.upcast_ref(), "3 / 3").text(),
            "3 / 3"
        );
        let req2 = find_download("msg-2", 0);
        let id2 = req2["id"].as_u64().unwrap();

        respond(
            &state,
            id2,
            false,
            serde_json::Value::Null,
            Some("Network error".into()),
        );
        let retry_btn = button_with_label(viewer_win.upcast_ref(), &crate::i18n::t("album_retry"));
        assert!(retry_btn.is_visible());
        assert!(label_containing(viewer_win.upcast_ref(), "Network error").is_visible());

        retry_btn.emit_clicked();
        drain();
        let req2_retry = find_download("msg-2", id2 + 1);
        let id2_retry = req2_retry["id"].as_u64().unwrap();

        respond(&state, id2_retry, true, serde_json::json!({}), None);
        assert!(
            label_containing(
                viewer_win.upcast_ref(),
                &crate::i18n::t("media_download_failed")
            )
            .is_visible()
        );

        let fresh_retry_btn =
            button_with_label(viewer_win.upcast_ref(), &crate::i18n::t("album_retry"));
        assert!(fresh_retry_btn.is_visible());

        fresh_retry_btn.emit_clicked();
        drain();
        let req2_retry2 = find_download("msg-2", id2_retry + 1);
        let id2_retry2 = req2_retry2["id"].as_u64().unwrap();

        handle_event(&state, ProtocolEvent::Exited(1));
        drain();
        assert!(
            label_containing(viewer_win.upcast_ref(), &crate::i18n::t("protocol_offline"))
                .is_visible()
        );

        *state.session_ready.borrow_mut() = true;
        let fresh_retry_btn2 =
            button_with_label(viewer_win.upcast_ref(), &crate::i18n::t("album_retry"));
        *state.current_chat.borrow_mut() = Some("chat-beta".into());
        clear_messages(&state);
        drain();

        fresh_retry_btn2.emit_clicked();
        drain();
        let req2_retry3 = find_download("msg-2", id2_retry2 + 1);
        assert_eq!(req2_retry3["params"]["chatMid"], "chat-alpha");
        let id2_retry3 = req2_retry3["id"].as_u64().unwrap();

        let weak_session = Rc::downgrade(&viewer.0);
        assert!(weak_session.upgrade().is_some());
        viewer_win.close();
        drain();
        drop(viewer);
        drop(viewer_win);
        assert!(weak_session.upgrade().is_none());
        assert!(state.chat_media_viewers.borrow().is_empty());

        respond(
            &state,
            id2_retry3,
            true,
            serde_json::json!({"path": img2_path.to_str().unwrap()}),
            None,
        );
        drain();
        assert!(weak_session.upgrade().is_none());
        assert!(state.chat_media_viewers.borrow().is_empty());
        assert_eq!(
            gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .filter(|w| {
                    w.is_visible()
                        && w.title().as_deref()
                            == Some(crate::i18n::t("media_viewer_title").as_str())
                })
                .count(),
            0
        );

        let beta_msg: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "beta-msg-0", "createdTime": 5000, "contentType": "IMAGE", "text": "[Image]",
            "to": "chat-beta"
        }))
        .unwrap();
        apply_messages(&state, vec![beta_msg.clone()]);
        open_chat_media_viewer(&state, &beta_msg);
        drain();

        assert_eq!(state.chat_media_viewers.borrow().len(), 1);
        let viewer2 = state.chat_media_viewers.borrow().last().unwrap().clone();
        let viewer2_win = viewer2.window();
        assert!(viewer2_win.is_visible());
        assert_eq!(
            label_containing(viewer2_win.upcast_ref(), "1 / 1").text(),
            "1 / 1"
        );

        let req_beta = find_download("beta-msg-0", 0);
        assert_eq!(req_beta["params"]["chatMid"], "chat-beta");

        let weak_session2 = Rc::downgrade(&viewer2.0);
        viewer2_win.close();
        drain();
        drop(viewer2);
        drop(viewer2_win);
        assert!(weak_session2.upgrade().is_none());
        assert!(state.chat_media_viewers.borrow().is_empty());

        if with_video {
            clear_messages(&state);
            let mut cached_video = msg1.clone();
            cached_video.file_path = Some(img1_path.to_string_lossy().into_owned());
            apply_messages(&state, vec![cached_video.clone()]);
            media::request_media_download(&state, &cached_video, "open_viewer");
            drain();
            let video_viewer = state.chat_media_viewers.borrow().last().unwrap().clone();
            let stream = video_stream(&video_viewer.window());
            wait_for(|| stream.is_prepared() || stream.error().is_some());
            assert!(stream.error().is_none(), "{:?}", stream.error());
            wait_for(|| stream.timestamp() > 0);
            video_viewer.window().close();
            drain();
            assert!(!stream.is_playing());
            assert!(state.chat_media_viewers.borrow().is_empty());

            media::open_media_viewer(
                &state,
                img1_path.to_str().unwrap(),
                "VIDEO",
                "clip.mp4",
                None,
            );
            drain();
            let standalone = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|window| window.downcast::<gtk::Window>().ok())
                .find(|window| {
                    window.is_visible()
                        && window.title().as_deref()
                            == Some(crate::i18n::t("media_viewer_title").as_str())
                })
                .unwrap();
            let stream = video_stream(&standalone);
            wait_for(|| stream.is_prepared() || stream.error().is_some());
            assert!(stream.error().is_none(), "{:?}", stream.error());
            wait_for(|| stream.timestamp() > 0);
            standalone.close();
            drain();
            assert!(!stream.is_playing());
        }

        for win in gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
        {
            win.close();
        }
        drain();
        let _ = std::fs::remove_dir_all(temp);
    }
}
