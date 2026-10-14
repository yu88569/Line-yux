use super::*;

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

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        widgets.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    widgets
}

fn button(widget: &gtk::Widget, key: &str) -> gtk::Button {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(crate::i18n::t(key).as_str()))
        .unwrap()
}

fn share_window() -> Option<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| {
            window.is_visible()
                && window.title().as_deref() == Some(crate::i18n::t("file_share").as_str())
        })
}

fn requests(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn respond(state: &AppState, request: &serde_json::Value, result: serde_json::Value) {
    handle_event(
        state,
        ProtocolEvent::Response {
            id: request["id"].as_u64().unwrap(),
            ok: true,
            result,
            error: None,
        },
    );
}

#[test]
#[ignore = "requires a GTK display"]
#[allow(deprecated)]
fn gtk_file_and_image_actions_save_pick_and_share_without_network() {
    libadwaita::init().unwrap();
    let temp =
        std::env::temp_dir().join(format!("line-file-actions-fixture-{}", std::process::id()));
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
        auto_mark_read: false,
        language: "en".into(),
        ..AppConfig::default()
    }
    .save(&data);
    let app = Application::builder()
        .application_id("dev.linegtk.FileActionFixture")
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
    *state.current_chat.borrow_mut() = Some("source-chat".into());
    *state.chats.borrow_mut() = vec![
        serde_json::from_value(serde_json::json!({"mid":"source-chat", "name":"Source"})).unwrap(),
        serde_json::from_value(serde_json::json!({"mid":"target-chat", "name":"Target"})).unwrap(),
    ];
    let source = temp.join("cached.bin");
    std::fs::write(&source, b"OK").unwrap();
    let msg: MessageInfo = serde_json::from_value(serde_json::json!({"id":"file-42", "contentType":"FILE", "text":"[File]", "filePath":source, "fileName":"old.bin"})).unwrap();
    let mut fresh = msg.clone();
    fresh.file_name = Some("รายงาน.xlsx".into());
    state.media_msgs.borrow_mut().insert(msg.id.clone(), fresh);
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    append_file_card(&state, &bubble, &msg);
    let folder = temp.join("saved");
    state.config.borrow_mut().download_dir_file = folder.to_string_lossy().into_owned();
    state.config.borrow_mut().download_ask_every_time = true;
    button(bubble.upcast_ref(), "file_save").emit_clicked();
    button(bubble.upcast_ref(), "file_save").emit_clicked();
    assert_eq!(std::fs::read(folder.join("รายงาน.xlsx")).unwrap(), b"OK");
    assert_eq!(std::fs::read(folder.join("รายงาน-1.xlsx")).unwrap(), b"OK");
    assert!(requests(&log).is_empty());

    state.config.borrow_mut().download_ask_every_time = false;
    state.config.borrow_mut().download_dir_file = temp
        .join("missing/subfolder")
        .to_string_lossy()
        .into_owned();
    button(bubble.upcast_ref(), "file_save_as").emit_clicked();
    let chooser = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| {
                widget
                    .downcast::<gtk::FileChooserDialog>()
                    .ok()
                    .filter(|dialog| dialog.is_visible())
            })
    };
    wait_for(|| chooser().is_some());
    let chooser = chooser().unwrap();
    assert_eq!(chooser.current_name().as_deref(), Some("รายงาน.xlsx"));
    chooser.response(gtk::ResponseType::Cancel);
    drain();
    assert!(!temp.join("missing/subfolder").exists());
    assert!(!folder.join("รายงาน-2.xlsx").exists());

    button(bubble.upcast_ref(), "file_share").emit_clicked();
    wait_for(|| share_window().is_some());
    let picker = share_window().unwrap();
    let send = button(picker.upcast_ref(), "send");
    assert!(!send.is_sensitive());
    send.emit_clicked();
    assert!(requests(&log).is_empty());
    button(picker.upcast_ref(), "file_share_cancel").emit_clicked();
    drain();
    assert!(requests(&log).is_empty());

    button(bubble.upcast_ref(), "file_share").emit_clicked();
    wait_for(|| share_window().is_some());
    let picker = share_window().unwrap();
    let list = descendants(picker.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ListBox>().ok())
        .unwrap();
    let search = descendants(picker.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::SearchEntry>().ok())
        .unwrap();
    search.set_text("Target");
    search.emit_by_name::<()>("search-changed", &[]);
    drain();
    assert!(!list.row_at_index(0).unwrap().is_child_visible());
    assert!(list.row_at_index(1).unwrap().is_child_visible());
    list.select_row(list.row_at_index(1).as_ref());
    let send = button(picker.upcast_ref(), "send");
    assert!(send.is_sensitive());
    send.emit_clicked();
    send.emit_clicked();
    wait_for(|| requests(&log).len() == 1);
    let request = requests(&log).remove(0);
    assert_eq!(request["method"], "send_media");
    assert_eq!(
        request["params"],
        serde_json::json!({"chatMid":"target-chat","filePath":source,"fileName":"รายงาน.xlsx","oType":"file"})
    );
    let before = state.seen_msg_ids.borrow().len();
    respond(
        &state,
        &request,
        serde_json::json!({"message":{"id":"sent-file", "chatMid":"target-chat", "contentType":"FILE","fileName":"รายงาน.xlsx"}}),
    );
    assert_eq!(state.seen_msg_ids.borrow().len(), before);

    let missing: MessageInfo = serde_json::from_value(
        serde_json::json!({"id":"remote-file","contentType":"FILE","text":"[File]"}),
    )
    .unwrap();
    file_actions::open_share_picker(&state, &missing);
    wait_for(|| share_window().is_some());
    let picker = share_window().unwrap();
    let list = descendants(picker.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ListBox>().ok())
        .unwrap();
    list.select_row(list.row_at_index(1).as_ref());
    *state.current_chat.borrow_mut() = Some("other-active-chat".into());
    button(picker.upcast_ref(), "send").emit_clicked();
    wait_for(|| requests(&log).len() == 2);
    let download = requests(&log).remove(1);
    assert_eq!(download["method"], "download_media");
    assert_eq!(
        download["params"],
        serde_json::json!({"chatMid":"source-chat","messageId":"remote-file"})
    );
    respond(
        &state,
        &download,
        serde_json::json!({"path":source,"fileName":"original.pdf","bytes":2}),
    );
    wait_for(|| requests(&log).len() == 3);
    let shared = requests(&log).remove(2);
    assert_eq!(shared["params"]["fileName"], "original.pdf");
    assert_eq!(shared["params"]["chatMid"], "target-chat");
    assert_eq!(shared["params"]["oType"], "file");
    *state.current_chat.borrow_mut() = Some("target-chat".into());
    respond(
        &state,
        &shared,
        serde_json::json!({"message":{"id":"sent-remote", "chatMid":"target-chat", "contentType":"FILE","fileName":"original.pdf"}}),
    );
    assert!(state.seen_msg_ids.borrow().contains("sent-remote"));
    *state.session_ready.borrow_mut() = false;
    file_actions::open_share_picker(&state, &missing);
    assert!(share_window().is_none());
    *state.session_ready.borrow_mut() = true;
    state.chats.borrow_mut().clear();
    file_actions::open_share_picker(&state, &missing);
    assert!(share_window().is_none());
    assert_eq!(requests(&log).len(), 3);
    for lang in ["en", "th"] {
        crate::i18n::set_lang(&state.repo_root, lang);
        let actions = file_actions::build_media_actions(&state, &msg);
        let (minimum_width, _, _, _) = actions.measure(gtk::Orientation::Horizontal, -1);
        assert!(
            minimum_width <= 320,
            "file actions fit a narrow chat in {lang}"
        );
    }
    crate::i18n::set_lang(&state.repo_root, "en");
    *state.current_chat.borrow_mut() = Some("source-chat".into());
    *state.chats.borrow_mut() = vec![
        serde_json::from_value(serde_json::json!({"mid":"target-chat", "name":"Target"})).unwrap(),
    ];
    let images = temp.join("saved-images");
    state.config.borrow_mut().download_dir_image = images.to_string_lossy().into_owned();
    let preview = temp.join("image-preview.png");
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 16, 16).unwrap();
    pixels.fill(0xff0000ff);
    pixels.savev(&preview, "png", &[]).unwrap();
    let full = temp.join("downloaded.full.png");
    pixels.fill(0x00ff00ff);
    pixels.savev(&full, "png", &[]).unwrap();
    let original = std::fs::read(&full).unwrap();
    let image: MessageInfo = serde_json::from_value(serde_json::json!({
        "id":"image-42", "contentType":"IMAGE", "text":"[Image]",
        "imagePath":preview, "filePath":preview, "fileName":"photo.png"
    }))
    .unwrap();
    append_message(&state, &image, false);
    let image_bubble = state.media_slots.borrow().get(&image.id).unwrap().clone();
    assert_image_actions_below_visual(&image_bubble);
    button(image_bubble.upcast_ref(), "file_save").emit_clicked();
    wait_for(|| requests(&log).len() == 4);
    let download = requests(&log).remove(3);
    assert_eq!(download["method"], "download_media");
    assert_eq!(download["params"]["messageId"], image.id);
    assert!(!images.exists());
    respond(
        &state,
        &download,
        serde_json::json!({"path":full, "fileName":"photo.png", "bytes":original.len()}),
    );
    assert_eq!(std::fs::read(images.join("photo.png")).unwrap(), original);
    assert_ne!(
        std::fs::read(images.join("photo.png")).unwrap(),
        std::fs::read(&preview).unwrap()
    );
    button(image_bubble.upcast_ref(), "file_save").emit_clicked();
    assert_eq!(std::fs::read(images.join("photo-1.png")).unwrap(), original);
    assert_eq!(requests(&log).len(), 4);
    button(image_bubble.upcast_ref(), "file_save_as").emit_clicked();
    let chooser = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| {
                widget
                    .downcast::<gtk::FileChooserDialog>()
                    .ok()
                    .filter(|dialog| dialog.is_visible())
            })
    };
    wait_for(|| chooser().is_some());
    let chooser = chooser().unwrap();
    assert_eq!(chooser.current_name().as_deref(), Some("photo.png"));
    wait_for(|| {
        chooser
            .current_folder()
            .and_then(|folder| folder.path())
            .is_some()
    });
    assert_eq!(
        chooser.current_folder().and_then(|folder| folder.path()),
        Some(images.clone())
    );
    chooser.response(gtk::ResponseType::Cancel);
    drain();
    button(image_bubble.upcast_ref(), "file_share").emit_clicked();
    wait_for(|| share_window().is_some());
    let picker = share_window().unwrap();
    let list = descendants(picker.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ListBox>().ok())
        .unwrap();
    list.select_row(list.row_at_index(0).as_ref());
    button(picker.upcast_ref(), "send").emit_clicked();
    wait_for(|| requests(&log).len() == 5);
    let shared = requests(&log).remove(4);
    assert_eq!(shared["method"], "send_media");
    assert_eq!(
        shared["params"],
        serde_json::json!({"chatMid":"target-chat","filePath":full,"fileName":"photo.png","oType":"image"})
    );

    let waiting: MessageInfo = serde_json::from_value(
        serde_json::json!({"id":"waiting-image", "contentType":"IMAGE", "text":"[Image]"}),
    )
    .unwrap();
    append_message(&state, &waiting, false);
    let waiting_bubble = state.media_slots.borrow().get(&waiting.id).unwrap().clone();
    assert_image_actions_below_visual(&waiting_bubble);
    mark_media_failed(&state, &waiting.id);
    assert_image_actions_below_visual(&waiting_bubble);
    for _ in 0..2 {
        handle_event(
            &state,
            ProtocolEvent::MediaReady {
                chat_mid: "source-chat".into(),
                message_id: waiting.id.clone(),
                image_path: preview.to_string_lossy().into_owned(),
                audio_path: None,
                file_path: None,
            },
        );
        wait_for(|| !*state.media_pumping.borrow() && state.media_queue.borrow().is_empty());
        assert_image_actions_below_visual(&waiting_bubble);
        assert!(waiting_bubble.first_child().unwrap().is::<gtk::Picture>());
    }
    button(waiting_bubble.upcast_ref(), "file_share").emit_clicked();
    wait_for(|| share_window().is_some());
    let picker = share_window().unwrap();
    let list = descendants(picker.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ListBox>().ok())
        .unwrap();
    list.select_row(list.row_at_index(0).as_ref());
    *state.current_chat.borrow_mut() = Some("other-active-chat".into());
    button(picker.upcast_ref(), "send").emit_clicked();
    wait_for(|| requests(&log).len() == 6);
    let download = requests(&log).remove(5);
    assert_eq!(download["method"], "download_media");
    assert_eq!(
        download["params"],
        serde_json::json!({"chatMid":"source-chat","messageId":"waiting-image"})
    );
    respond(
        &state,
        &download,
        serde_json::json!({"path":full,"fileName":"shared-photo.png","bytes":original.len()}),
    );
    wait_for(|| requests(&log).len() == 7);
    let shared = requests(&log).remove(6);
    assert_eq!(shared["method"], "send_media");
    assert_eq!(
        shared["params"],
        serde_json::json!({"chatMid":"target-chat","filePath":full,"fileName":"shared-photo.png","oType":"image"})
    );
    for content_type in ["STICKER", "VIDEO"] {
        let msg: MessageInfo = serde_json::from_value(
            serde_json::json!({"id":content_type, "contentType":content_type, "text":""}),
        )
        .unwrap();
        append_message(&state, &msg, false);
        let bubble = state
            .media_slots
            .borrow()
            .get(content_type)
            .unwrap()
            .clone();
        assert!(
            !descendants(bubble.upcast_ref())
                .iter()
                .any(|widget| widget.has_css_class("line-file-actions"))
        );
    }
    state.window.close();
    drain();
    std::fs::remove_dir_all(temp).unwrap();
}

fn assert_image_actions_below_visual(bubble: &gtk::Box) {
    let actions = bubble.last_child().unwrap();
    assert!(actions.has_css_class("line-file-actions"));
    let visual = actions.prev_sibling().unwrap();
    assert!(
        visual.is::<gtk::Picture>()
            || visual.has_css_class("line-media-placeholder")
            || visual.has_css_class("line-media-failed")
    );
    assert_eq!(
        descendants(bubble.upcast_ref())
            .iter()
            .filter(|widget| widget.has_css_class("line-file-actions"))
            .count(),
        1
    );
    for key in ["file_save", "file_save_as", "file_share"] {
        assert!(button(actions.upcast_ref(), key).is_sensitive());
    }
}
