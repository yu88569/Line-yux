use super::*;
use crate::protocol::{AlbumMediaKind, AlbumPage, AlbumPhoto, AlbumRef};
use std::cell::Cell;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Download {
    Loading,
    Ready(String),
    Failed(String),
}

#[derive(Clone)]
struct AlbumItem {
    photo: AlbumPhoto,
    download: Download,
    picture: gtk::Picture,
    button: gtk::Button,
}

#[derive(Default)]
struct AlbumModel {
    items: Vec<AlbumItem>,
    cursor: Option<String>,
    visited: HashSet<String>,
    page: PageState,
}

#[derive(Default, PartialEq, Eq)]
enum PageState {
    #[default]
    Available,
    Loading,
    Failed(String),
    Exhausted,
}

impl AlbumModel {
    fn images(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (item.photo.kind == AlbumMediaKind::Image).then_some(i))
            .collect()
    }

    fn accept_page(&mut self, page: &AlbumPage) -> Vec<AlbumPhoto> {
        let photos = unique_photos(
            self.items.iter().map(|i| i.photo.oid.as_str()),
            &page.photos,
        );
        self.page = if !page.photos.is_empty()
            && page
                .next_cursor
                .as_ref()
                .is_some_and(|c| self.visited.insert(c.clone()))
        {
            PageState::Available
        } else {
            PageState::Exhausted
        };
        self.cursor = page.next_cursor.clone();
        photos
    }
}

fn unique_photos<'a>(
    existing: impl Iterator<Item = &'a str>,
    photos: &[AlbumPhoto],
) -> Vec<AlbumPhoto> {
    let mut seen: HashSet<_> = existing.map(str::to_owned).collect();
    photos
        .iter()
        .filter(|photo| seen.insert(photo.oid.clone()))
        .cloned()
        .collect()
}

fn neighbor(images: &[usize], position: usize, next: bool) -> Option<usize> {
    if next {
        images.get(position + 1).copied()
    } else {
        position.checked_sub(1).and_then(|p| images.get(p)).copied()
    }
}

#[derive(Clone)]
struct AlbumViewer {
    media: media::MediaViewer,
    selected: Rc<Cell<usize>>,
    advance: Rc<Cell<bool>>,
    rendered: Rc<RefCell<Option<(usize, String)>>>,
}

#[derive(Clone)]
pub(super) struct AlbumGallery {
    window: gtk::Window,
    album: AlbumRef,
    grid: gtk::FlowBox,
    status: gtk::Label,
    more: gtk::Button,
    model: Rc<RefCell<AlbumModel>>,
    viewer: Rc<RefCell<Option<AlbumViewer>>>,
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
        model: Rc::new(RefCell::new(AlbumModel::default())),
        viewer: Rc::new(RefCell::new(None)),
    };
    let s = state.clone();
    let ui2 = ui.clone();
    ui.more.connect_clicked(move |_| request_page(&s, &ui2));
    ui.window.present();
    request_page(state, &ui);
}

fn request_page(state: &AppState, ui: &AlbumGallery) {
    {
        let mut model = ui.model.borrow_mut();
        if matches!(model.page, PageState::Loading | PageState::Exhausted) {
            return;
        }
        model.page = PageState::Loading;
    }
    ui.more.set_sensitive(false);
    ui.status.set_text(&crate::i18n::t("album_loading"));
    let params = serde_json::json!({"chatMid": ui.album.chat_mid, "albumId": ui.album.album_id, "cursor": ui.model.borrow().cursor});
    match state.sidecar.request("list_album_photos", Some(params)) {
        Ok(id) => {
            state
                .pending
                .borrow_mut()
                .insert(id, Pending::AlbumPage(ui.clone()));
        }
        Err(e) => page_failed(state, ui, &e.to_string()),
    }
}

pub(super) fn page_failed(state: &AppState, ui: &AlbumGallery, error: &str) {
    ui.model.borrow_mut().page = PageState::Failed(error.into());
    ui.status.set_text(error);
    ui.more.set_label(&crate::i18n::t("album_retry"));
    ui.more.set_sensitive(true);
    refresh_viewer(state, ui);
}

pub(super) fn page_ready(state: &AppState, ui: &AlbumGallery, value: serde_json::Value) {
    if !ui.window.is_visible() {
        return;
    }
    let page = match serde_json::from_value::<AlbumPage>(value) {
        Ok(page) => page,
        Err(e) => {
            page_failed(state, ui, &e.to_string());
            return;
        }
    };
    let photos = ui.model.borrow_mut().accept_page(&page);
    let mut downloads = Vec::new();
    for photo in photos {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let picture = gtk::Picture::builder()
            .width_request(160)
            .height_request(150)
            .can_shrink(true)
            .build();
        tile.append(&picture);
        let button = gtk::Button::with_label(&crate::i18n::t("album_photo_open"));
        button.set_sensitive(false);
        tile.append(&button);
        ui.grid.insert(&tile, -1);
        let index = ui.model.borrow().items.len();
        ui.model.borrow_mut().items.push(AlbumItem {
            photo: photo.clone(),
            download: Download::Loading,
            picture: picture.clone(),
            button: button.clone(),
        });
        let s = state.clone();
        let ui2 = ui.clone();
        button.connect_clicked(move |_| open_item(&s, &ui2, index));
        downloads.push((photo, picture, button));
    }
    let model = ui.model.borrow();
    ui.more.set_visible(model.page != PageState::Exhausted);
    ui.more.set_sensitive(true);
    ui.more.set_label(&crate::i18n::t("album_more"));
    ui.status.set_text(&if model.items.is_empty() {
        crate::i18n::t("album_empty")
    } else {
        String::new()
    });
    drop(model);
    for (photo, picture, button) in downloads {
        request_photo(state, ui, &photo, &picture, &button);
    }
    refresh_viewer(state, ui);
}

fn request_photo(
    state: &AppState,
    ui: &AlbumGallery,
    photo: &AlbumPhoto,
    pic: &gtk::Picture,
    button: &gtk::Button,
) {
    if let Some(item) = ui
        .model
        .borrow_mut()
        .items
        .iter_mut()
        .find(|i| i.photo.oid == photo.oid)
    {
        item.download = Download::Loading;
    }
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
    _pic: &gtk::Picture,
    button: &gtk::Button,
    error: &str,
) {
    if let Some(item) = ui
        .model
        .borrow_mut()
        .items
        .iter_mut()
        .find(|i| i.photo.oid == photo.oid)
    {
        item.download = Download::Failed(error.into());
    }
    button.set_label(&crate::i18n::t("album_retry"));
    button.set_tooltip_text(Some(error));
    button.set_sensitive(true);
    refresh_viewer(state, ui);
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
    if let Some(item) = ui
        .model
        .borrow_mut()
        .items
        .iter_mut()
        .find(|i| i.photo.oid == photo.oid)
    {
        item.download = Download::Ready(path.into());
    }
    if photo.kind == AlbumMediaKind::Image {
        attach_texture_async_anim(picture.clone(), path.to_owned(), 180, false);
    }
    button.set_label(&crate::i18n::t("album_photo_open"));
    button.set_tooltip_text(None);
    button.set_sensitive(true);
    refresh_viewer(state, ui);
}

fn open_item(state: &AppState, ui: &AlbumGallery, index: usize) {
    let item = ui.model.borrow().items[index].clone();
    if let Download::Failed(_) = item.download {
        request_photo(state, ui, &item.photo, &item.picture, &item.button);
        refresh_viewer(state, ui);
        return;
    }
    if item.photo.kind == AlbumMediaKind::Video {
        if let Download::Ready(path) = item.download {
            open_media_viewer(
                state,
                &path,
                "VIDEO",
                &format!("{}.mp4", item.photo.oid),
                None,
            );
        }
        return;
    }
    let existing = ui.viewer.borrow().clone();
    let viewer = if let Some(viewer) = existing {
        viewer
    } else {
        let viewer = AlbumViewer {
            media: media::MediaViewer::new(state),
            selected: Rc::new(Cell::new(index)),
            advance: Rc::new(Cell::new(false)),
            rendered: Rc::new(RefCell::new(None)),
        };
        let s = state.clone();
        let ui2 = ui.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::SHIFT_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Left => navigate(&s, &ui2, false),
                gdk::Key::Right => navigate(&s, &ui2, true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        viewer.media.window.add_controller(keys);
        let slot = ui.viewer.clone();
        viewer.media.window.connect_close_request(move |_| {
            slot.borrow_mut().take();
            glib::Propagation::Proceed
        });
        *ui.viewer.borrow_mut() = Some(viewer.clone());
        viewer
    };
    viewer.selected.set(index);
    viewer.advance.set(false);
    refresh_viewer(state, ui);
    viewer.media.window.present();
}

fn navigate(state: &AppState, ui: &AlbumGallery, next: bool) {
    let Some(viewer) = ui.viewer.borrow().clone() else {
        return;
    };
    let model = ui.model.borrow();
    let images = model.images();
    let Some(position) = images.iter().position(|i| *i == viewer.selected.get()) else {
        return;
    };
    if !next {
        viewer.advance.set(false);
    }
    if let Some(index) = neighbor(&images, position, next) {
        viewer.selected.set(index);
        viewer.advance.set(false);
    } else if next && model.page != PageState::Exhausted {
        viewer.advance.set(true);
    }
    drop(model);
    refresh_viewer(state, ui);
}

fn refresh_viewer(state: &AppState, ui: &AlbumGallery) {
    let Some(viewer) = ui.viewer.borrow().clone() else {
        return;
    };
    let model = ui.model.borrow();
    let images = model.images();
    let mut position = images
        .iter()
        .position(|i| *i == viewer.selected.get())
        .unwrap_or(0);
    if viewer.advance.get() {
        if let Some(index) = images.get(position + 1) {
            viewer.selected.set(*index);
            viewer.advance.set(false);
            position += 1;
        } else if model.page == PageState::Exhausted {
            viewer.advance.set(false);
        }
    }
    let waiting = viewer.advance.get();
    let item = model.items[viewer.selected.get()].clone();
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    navigation.set_halign(gtk::Align::Center);
    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    previous.set_tooltip_text(Some(&crate::i18n::t("album_previous")));
    previous.set_sensitive(position > 0 || waiting);
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some(&crate::i18n::t("album_next")));
    next.set_sensitive(
        !waiting && (position + 1 < images.len() || model.page != PageState::Exhausted),
    );
    let counter = gtk::Label::new(Some(&format!(
        "{} / {}{}",
        position + 1,
        images.len(),
        if model.page == PageState::Exhausted {
            ""
        } else {
            "+"
        }
    )));
    navigation.append(&previous);
    navigation.append(&counter);
    navigation.append(&next);
    for (button, next) in [(previous, false), (next, true)] {
        let s = state.clone();
        let ui2 = ui.clone();
        button.connect_clicked(move |_| navigate(&s, &ui2, next));
    }
    let page_error = match &model.page {
        PageState::Failed(error) => Some(error.clone()),
        _ => None,
    };
    let needs_page = waiting && matches!(model.page, PageState::Available);
    drop(model);
    let ready = if !waiting {
        if let Download::Ready(path) = &item.download {
            Some((viewer.selected.get(), path.clone()))
        } else {
            None
        }
    } else {
        None
    };
    if ready != *viewer.rendered.borrow() || ready.is_none() {
        viewer.media.clear();
        if let Some((_, path)) = &ready {
            viewer.media.show(
                state,
                path,
                "IMAGE",
                &format!("{}.jpg", item.photo.oid),
                Some(&navigation),
                None,
            );
        } else {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
            root.append(&navigation);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let error = if waiting {
                page_error
            } else {
                match &item.download {
                    Download::Failed(error) => Some(error.clone()),
                    _ => None,
                }
            };
            let label = gtk::Label::new(Some(
                &error
                    .clone()
                    .unwrap_or_else(|| crate::i18n::t("album_loading")),
            ));
            label.set_wrap(true);
            label.set_hexpand(true);
            let retry = gtk::Button::with_label(&crate::i18n::t("album_retry"));
            retry.set_visible(error.is_some());
            let s = state.clone();
            let ui2 = ui.clone();
            let item = item.clone();
            retry.connect_clicked(move |_| {
                if waiting {
                    request_page(&s, &ui2);
                } else {
                    request_photo(&s, &ui2, &item.photo, &item.picture, &item.button);
                }
                refresh_viewer(&s, &ui2);
            });
            row.append(&label);
            row.append(&retry);
            root.append(&row);
            viewer.media.window.set_child(Some(&root));
        }
        *viewer.rendered.borrow_mut() = ready;
    } else if let Some(root) = viewer.media.window.child().and_downcast::<gtk::Box>()
        && let Some(bar) = root.first_child()
        && let Some(old) = bar.next_sibling()
    {
        root.remove(&old);
        root.insert_child_after(&navigation, Some(&bar));
    }
    if needs_page {
        request_page(state, ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_photos_deduplicate_without_reordering() {
        let photos: Vec<AlbumPhoto> = serde_json::from_value(serde_json::json!([
            {"oid":"third","kind":"image"}, {"oid":"second","kind":"image"},
            {"oid":"third","kind":"image"}, {"oid":"video","kind":"video"},
            {"oid":"first","kind":"image"}
        ]))
        .unwrap();
        let result = unique_photos(["first"].into_iter(), &photos);
        assert_eq!(
            result.iter().map(|p| p.oid.as_str()).collect::<Vec<_>>(),
            vec!["third", "second", "video"]
        );
        assert_eq!(result[2].kind, AlbumMediaKind::Video);
    }

    #[test]
    fn navigation_stops_at_boundaries_and_skips_video_indices() {
        let images = [0, 2, 5];
        assert_eq!(neighbor(&images, 0, true), Some(2));
        assert_eq!(neighbor(&images, 1, false), Some(0));
        assert_eq!(neighbor(&images, 2, true), None);
        assert_eq!(neighbor(&images, 0, false), None);
    }

    #[test]
    fn pagination_stops_at_empty_or_repeated_cursor() {
        let mut model = AlbumModel::default();
        let page = |cursor: &str| {
            serde_json::from_value::<AlbumPage>(serde_json::json!({
                "photos":[{"oid":"video","kind":"video"}], "nextCursor":cursor
            }))
            .unwrap()
        };
        assert_eq!(model.accept_page(&page("a"))[0].oid, "video");
        assert!(model.page == PageState::Available);
        model.accept_page(&page("b"));
        assert!(model.page == PageState::Available);
        model.accept_page(&page("a"));
        assert!(model.page == PageState::Exhausted);
        let empty =
            serde_json::from_value::<AlbumPage>(serde_json::json!({"photos":[],"nextCursor":"c"}))
                .unwrap();
        model.accept_page(&empty);
        assert!(model.page == PageState::Exhausted);
    }

    fn button_in(widget: &gtk::Widget, icon: &str) -> gtk::Button {
        fn find(widget: &gtk::Widget, icon: &str) -> Option<gtk::Button> {
            if let Ok(button) = widget.clone().downcast::<gtk::Button>()
                && button.icon_name().as_deref() == Some(icon)
            {
                return Some(button);
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if let Some(button) = find(&widget, icon) {
                    return Some(button);
                }
            }
            None
        }
        find(widget, icon).unwrap()
    }

    fn key_controller_count(win: &gtk::Window) -> usize {
        let controllers = win.observe_controllers();
        (0..controllers.n_items())
            .filter(|i| {
                controllers
                    .item(*i)
                    .and_downcast::<gtk::EventControllerKey>()
                    .is_some()
            })
            .count()
    }

    fn press(win: &gtk::Window, key: gdk::Key, modifiers: gdk::ModifierType) {
        let controllers = win.observe_controllers();
        let keys: Vec<_> = (0..controllers.n_items())
            .filter_map(|i| {
                controllers
                    .item(i)
                    .and_downcast::<gtk::EventControllerKey>()
            })
            .collect();
        for keys in keys {
            if keys.emit_by_name::<bool>("key-pressed", &[&key, &0_u32, &modifiers]) {
                return;
            }
        }
        panic!("key was not handled");
    }

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
        let viewer = ui.viewer.borrow().clone().unwrap();
        let win = viewer.media.window.clone();
        let initial_key_controllers = key_controller_count(&win);
        assert!(!button_in(win.upcast_ref(), "go-previous-symbolic").is_sensitive());
        button_in(win.upcast_ref(), "go-next-symbolic").emit_clicked();
        let page_id = *state.pending.borrow().keys().max().unwrap();
        assert!(matches!(
            state.pending.borrow().get(&page_id),
            Some(Pending::AlbumPage(_))
        ));
        press(&win, gdk::Key::Right, gdk::ModifierType::empty());
        assert_eq!(state.pending.borrow().len(), 1);
        respond(&state, page_id, false, serde_json::Value::Null);
        assert_eq!(ui.status.text(), "Album offline");
        let root = win.child().unwrap().downcast::<gtk::Box>().unwrap();
        let row = root.last_child().unwrap().downcast::<gtk::Box>().unwrap();
        row.last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        let page_id = *state.pending.borrow().keys().max().unwrap();
        respond(
            &state,
            page_id,
            true,
            serde_json::json!({"photos":[
            {"oid":"photo.1","kind":"image"}, {"oid":"video.1","kind":"video"}
        ], "nextCursor":"video-only"}),
        );
        let page_id = *state
            .pending
            .borrow()
            .iter()
            .find(|(_, p)| matches!(p, Pending::AlbumPage(_)))
            .unwrap()
            .0;
        respond(
            &state,
            page_id,
            true,
            serde_json::json!({"photos":[
            {"oid":"photo.2","kind":"image"}, {"oid":"photo.3","kind":"image"}
        ], "nextCursor":"last"}),
        );
        assert_eq!(viewer.selected.get(), 2);
        assert_eq!(*viewer.rendered.borrow(), None);
        let pending_photo = |oid: &str| {
            *state
                .pending
                .borrow()
                .iter()
                .find(|(_, p)| matches!(p, Pending::AlbumPhoto { photo, .. } if photo.oid == oid))
                .unwrap()
                .0
        };
        let green = temp.join("green.png");
        pixbuf.fill(0x00ff00ff);
        pixbuf.savev(&green, "png", &[]).unwrap();
        let blue = temp.join("blue.png");
        pixbuf.fill(0x0000ffff);
        pixbuf.savev(&blue, "png", &[]).unwrap();
        respond(
            &state,
            pending_photo("photo.3"),
            true,
            serde_json::json!({"path":blue}),
        );
        assert_eq!(viewer.selected.get(), 2);
        respond(
            &state,
            pending_photo("photo.2"),
            false,
            serde_json::Value::Null,
        );
        let root = win.child().unwrap().downcast::<gtk::Box>().unwrap();
        let row = root.last_child().unwrap().downcast::<gtk::Box>().unwrap();
        assert_eq!(
            row.first_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .text(),
            "Album offline"
        );
        row.last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        press(&win, gdk::Key::Left, gdk::ModifierType::empty());
        assert_eq!(viewer.selected.get(), 0);
        respond(
            &state,
            pending_photo("photo.2"),
            true,
            serde_json::json!({"path":green}),
        );
        assert_eq!(
            viewer
                .rendered
                .borrow()
                .as_ref()
                .map(|(_, path)| path.as_str()),
            image.to_str()
        );
        let next = button_in(win.upcast_ref(), "go-next-symbolic");
        next.grab_focus();
        press(&win, gdk::Key::Right, gdk::ModifierType::empty());
        assert_eq!(
            viewer
                .rendered
                .borrow()
                .as_ref()
                .map(|(_, path)| path.as_str()),
            green.to_str()
        );
        assert_eq!(key_controller_count(&win), initial_key_controllers);
        let clipboard = win.clipboard();
        press(&win, gdk::Key::c, gdk::ModifierType::CONTROL_MASK);
        let texture = glib::MainContext::default()
            .block_on(clipboard.read_texture_future())
            .unwrap()
            .unwrap();
        let copied = image::load_from_memory(texture.save_to_png_bytes().as_ref())
            .unwrap()
            .into_rgba8();
        assert_eq!(copied.get_pixel(0, 0).0, [0, 255, 0, 255]);
        button_in(win.upcast_ref(), "go-next-symbolic").emit_clicked();
        assert_eq!(
            viewer
                .rendered
                .borrow()
                .as_ref()
                .map(|(_, path)| path.as_str()),
            blue.to_str()
        );
        press(&win, gdk::Key::Right, gdk::ModifierType::empty());
        let page_id = *state
            .pending
            .borrow()
            .iter()
            .find(|(_, p)| matches!(p, Pending::AlbumPage(_)))
            .unwrap()
            .0;
        press(&win, gdk::Key::Escape, gdk::ModifierType::empty());
        assert!(ui.viewer.borrow().is_none());
        respond(
            &state,
            page_id,
            true,
            serde_json::json!({"photos":[],"nextCursor":"different-but-empty"}),
        );
        assert!(!win.is_visible());
        assert!(ui.viewer.borrow().is_none());
        assert!(!ui.more.is_visible());
        assert_eq!(ui.model.borrow().items.len(), 4);
        clipboard
            .set_content(None::<&gdk::ContentProvider>)
            .unwrap();
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
