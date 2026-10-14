use super::*;
use crate::protocol::{AlbumMediaKind, AlbumPhoto, AlbumRef};

#[derive(Clone)]
pub(super) struct AlbumGallery {
    window: gtk::Window,
    album: AlbumRef,
    grid: gtk::FlowBox,
    status: gtk::Label,
    more: gtk::Button,
    cursor: Rc<RefCell<Option<String>>>,
    seen: Rc<RefCell<HashSet<String>>>,
}

pub(super) fn append_album_card(state: &AppState, bubble: &gtk::Box, album: &AlbumRef) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::builder()
        .label(&album.title)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["heading"])
        .build();
    card.append(&title);
    let open = gtk::Button::with_label(&crate::i18n::t("album_open"));
    let s = state.clone();
    let album = album.clone();
    open.connect_clicked(move |_| open_album(&s, &album));
    card.append(&open);
    bubble.append(&card);
}

pub(super) fn open_album(state: &AppState, album: &AlbumRef) {
    let window = gtk::Window::builder()
        .title(&album.title)
        .transient_for(&state.window)
        .default_width(760)
        .default_height(650)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(8)
        .row_spacing(8)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .build();
    let status = gtk::Label::builder().wrap(true).build();
    let more = gtk::Button::with_label(&crate::i18n::t("album_more"));
    root.append(&scroll);
    root.append(&status);
    root.append(&more);
    window.set_child(Some(&root));
    let ui = AlbumGallery {
        window,
        album: album.clone(),
        grid,
        status,
        more,
        cursor: Rc::new(RefCell::new(None)),
        seen: Rc::new(RefCell::new(HashSet::new())),
    };
    let s = state.clone();
    let ui2 = ui.clone();
    ui.more.connect_clicked(move |_| request_page(&s, &ui2));
    ui.window.present();
    request_page(state, &ui);
}

fn request_page(state: &AppState, ui: &AlbumGallery) {
    ui.more.set_sensitive(false);
    ui.status.set_text(&crate::i18n::t("album_loading"));
    match state.sidecar.request("list_album_photos", Some(serde_json::json!({"chatMid": ui.album.chat_mid, "albumId": ui.album.album_id, "cursor": *ui.cursor.borrow()}))) {
        Ok(id) => { state.pending.borrow_mut().insert(id, Pending::AlbumPage(ui.clone())); }
        Err(e) => page_failed(ui, &e.to_string()),
    }
}

pub(super) fn page_failed(ui: &AlbumGallery, error: &str) {
    ui.status.set_text(error);
    ui.more.set_label(&crate::i18n::t("album_retry"));
    ui.more.set_sensitive(true);
}

pub(super) fn page_ready(state: &AppState, ui: &AlbumGallery, value: serde_json::Value) {
    if !ui.window.is_visible() {
        return;
    }
    let page = match serde_json::from_value::<crate::protocol::AlbumPage>(value) {
        Ok(page) => page,
        Err(e) => {
            page_failed(ui, &e.to_string());
            return;
        }
    };
    let nonempty = !page.photos.is_empty();
    for photo in page.photos {
        if !ui.seen.borrow_mut().insert(photo.oid.clone()) {
            continue;
        }
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let pic = gtk::Picture::builder()
            .width_request(160)
            .height_request(150)
            .can_shrink(true)
            .build();
        tile.append(&pic);
        let open = gtk::Button::with_label(&crate::i18n::t("album_photo_open"));
        open.set_sensitive(false);
        tile.append(&open);
        ui.grid.insert(&tile, -1);
        request_photo(state, ui, &photo, &pic, &open);
    }
    let has_more =
        nonempty && page.next_cursor.is_some() && page.next_cursor != *ui.cursor.borrow();
    *ui.cursor.borrow_mut() = page.next_cursor;
    ui.more.set_visible(has_more);
    ui.more.set_sensitive(true);
    ui.more.set_label(&crate::i18n::t("album_more"));
    let status = if ui.seen.borrow().is_empty() {
        crate::i18n::t("album_empty")
    } else {
        String::new()
    };
    ui.status.set_text(&status);
}

fn request_photo(
    state: &AppState,
    ui: &AlbumGallery,
    photo: &AlbumPhoto,
    pic: &gtk::Picture,
    button: &gtk::Button,
) {
    button.set_sensitive(false);
    match state.sidecar.request("download_album_photo", Some(serde_json::json!({"chatMid":ui.album.chat_mid,"albumId":ui.album.album_id,"oid":photo.oid,"kind":photo.kind}))) {
        Ok(id) => { state.pending.borrow_mut().insert(id, Pending::AlbumPhoto { ui: ui.clone(), photo: photo.clone(), picture: pic.clone(), button: button.clone() }); }
        Err(e) => photo_failed(state, ui, photo, pic, button, &e.to_string()),
    }
}

pub(super) fn photo_failed(
    state: &AppState,
    ui: &AlbumGallery,
    photo: &AlbumPhoto,
    pic: &gtk::Picture,
    button: &gtk::Button,
    error: &str,
) {
    button.set_label(&crate::i18n::t("album_retry"));
    button.set_tooltip_text(Some(error));
    button.set_sensitive(true);
    let s = state.clone();
    let ui = ui.clone();
    let photo = photo.clone();
    let pic = pic.clone();
    let handler = Rc::new(RefCell::new(None));
    let disconnect = handler.clone();
    let id = button.connect_clicked(move |button| {
        if let Some(id) = disconnect.borrow_mut().take() {
            button.disconnect(id);
        }
        request_photo(&s, &ui, &photo, &pic, button);
    });
    *handler.borrow_mut() = Some(id);
}

pub(super) fn photo_ready(
    state: &AppState,
    ui: &AlbumGallery,
    photo: &AlbumPhoto,
    picture: &gtk::Picture,
    button: &gtk::Button,
    value: &serde_json::Value,
) {
    if !ui.window.is_visible() {
        return;
    }
    let Some(path) = value
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|p| std::path::Path::new(p).is_file())
    else {
        photo_failed(state, ui, photo, picture, button, "Album photo unavailable");
        return;
    };
    if photo.kind == AlbumMediaKind::Image {
        attach_texture_async_anim(picture.clone(), path.to_owned(), 180, false);
    }
    button.set_label(&crate::i18n::t("album_photo_open"));
    button.set_tooltip_text(None);
    button.set_sensitive(true);
    let s = state.clone();
    let path = path.to_owned();
    let name = format!(
        "{}.{}",
        photo.oid,
        if photo.kind == AlbumMediaKind::Video {
            "mp4"
        } else {
            "jpg"
        }
    );
    let kind = if photo.kind == AlbumMediaKind::Video {
        "VIDEO"
    } else {
        "IMAGE"
    };
    button.connect_clicked(move |_| open_media_viewer(&s, &path, kind, &name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain() {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
    }
    fn latest(path: &std::path::Path) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            drain();
            if let Some(value) = std::fs::read_to_string(path)
                .ok()
                .and_then(|s| s.lines().last().and_then(|l| serde_json::from_str(l).ok()))
            {
                return value;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn respond(state: &AppState, id: u64, ok: bool, result: serde_json::Value) {
        handle_event(
            state,
            ProtocolEvent::Response {
                id,
                ok,
                result,
                error: (!ok).then(|| "Album offline".into()),
            },
        );
        drain();
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn gtk_album_gallery_retry_pagination_and_full_viewer() {
        libadwaita::init().unwrap();
        let temp = std::env::temp_dir().join(format!("line-album-fixture-{}", std::process::id()));
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
            .application_id("dev.linegtk.AlbumFixture")
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
        let album = AlbumRef {
            chat_mid: format!("c{}", "1".repeat(32)),
            album_id: "5333564955743123169".into(),
            title: "Album fixture".into(),
        };
        open_album(&state, &album);
        let request = latest(&log);
        assert_eq!(request["method"], "list_album_photos");
        let id = request["id"].as_u64().unwrap();
        let ui = match state.pending.borrow().get(&id).unwrap() {
            Pending::AlbumPage(ui) => ui.clone(),
            _ => panic!(),
        };
        respond(&state, id, false, serde_json::Value::Null);
        assert!(ui.more.is_sensitive());
        assert_eq!(
            ui.more.label().as_deref(),
            Some(crate::i18n::t("album_retry").as_str())
        );
        ui.more.emit_clicked();
        let id = *state.pending.borrow().keys().max().unwrap();
        respond(
            &state,
            id,
            true,
            serde_json::json!({"photos":[{"oid":"photo.1","kind":"image"}],"nextCursor":"next"}),
        );
        let id = *state.pending.borrow().keys().max().unwrap();
        let (picture, button, photo) = match state.pending.borrow().get(&id).unwrap() {
            Pending::AlbumPhoto {
                picture,
                button,
                photo,
                ..
            } => (picture.clone(), button.clone(), photo.clone()),
            _ => panic!(),
        };
        respond(&state, id, false, serde_json::Value::Null);
        assert!(button.is_sensitive());
        button.emit_clicked();
        let id = *state.pending.borrow().keys().max().unwrap();
        let image = temp.join("photo.png");
        let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 20, 20).unwrap();
        pixbuf.fill(0xff0000ff);
        pixbuf.savev(&image, "png", &[]).unwrap();
        respond(&state, id, true, serde_json::json!({"path":image}));
        assert_eq!(
            button.label().as_deref(),
            Some(crate::i18n::t("album_photo_open").as_str())
        );
        button.emit_clicked();
        drain();
        assert!(
            gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .any(|w| w.is_visible()
                    && w.title().as_deref() == Some(crate::i18n::t("media_viewer_title").as_str()))
        );
        assert_eq!(photo.oid, "photo.1");
        assert!(picture.is_visible());
        ui.more.emit_clicked();
        let id = *state.pending.borrow().keys().max().unwrap();
        respond(
            &state,
            id,
            true,
            serde_json::json!({"photos":[],"nextCursor":"different-but-empty"}),
        );
        assert!(!ui.more.is_visible());
        assert_eq!(ui.seen.borrow().len(), 1);
        for window in gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
        {
            window.close();
        }

        drain();
        let _ = std::fs::remove_dir_all(temp);
    }
}
