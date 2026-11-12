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
    assert!(!button(bubble.upcast_ref(), "file_open_folder").is_visible());
    let folder = temp.join("saved");
    state.config.borrow_mut().download_dir_file = folder.to_string_lossy().into_owned();
    state.config.borrow_mut().download_ask_every_time = true;
    button(bubble.upcast_ref(), "file_save").emit_clicked();
    button(bubble.upcast_ref(), "file_save").emit_clicked();
    assert_eq!(std::fs::read(folder.join("รายงาน.xlsx")).unwrap(), b"OK");
    assert_eq!(std::fs::read(folder.join("รายงาน-1.xlsx")).unwrap(), b"OK");
    assert!(button(bubble.upcast_ref(), "file_open_folder").is_visible());
    assert_eq!(
        state.saved_attachments.borrow().destination(&msg.id),
        Some(folder.join("รายงาน-1.xlsx").as_path())
    );
    assert_eq!(
        downloads::SavedAttachments::load(&state.data_dir).destination(&msg.id),
        Some(folder.join("รายงาน-1.xlsx").as_path())
    );
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
    assert_eq!(
        state.saved_attachments.borrow().destination(&msg.id),
        Some(folder.join("รายงาน-1.xlsx").as_path())
    );
    copy_media_to_dest(
        &state,
        temp.join("missing-source").to_str().unwrap(),
        &temp.join("failed.bin"),
        Some(&msg.id),
    );
    assert_eq!(
        state.saved_attachments.borrow().destination(&msg.id),
        Some(folder.join("รายงาน-1.xlsx").as_path())
    );
    copy_media_to_dest(
        &state,
        temp.join("missing-source").to_str().unwrap(),
        &temp.join("failed.bin"),
        Some("never-saved"),
    );
    assert!(
        state
            .saved_attachments
            .borrow()
            .destination("never-saved")
            .is_none()
    );

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
    wait_for(|| chooser.current_folder().is_some());
    let custom = temp.join("custom folder ภาษาไทย");
    std::fs::create_dir_all(&custom).unwrap();
    chooser
        .set_current_folder(Some(&gio::File::for_path(&custom)))
        .unwrap();
    wait_for(|| {
        chooser
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&custom)
    });
    chooser.set_current_name("renamed.xlsx");
    chooser.response(gtk::ResponseType::Accept);
    let renamed = custom.join("renamed.xlsx");
    wait_for(|| renamed.is_file());
    assert_eq!(std::fs::read(&renamed).unwrap(), b"OK");
    assert_eq!(
        state.saved_attachments.borrow().destination(&msg.id),
        Some(renamed.as_path())
    );
    *state.saved_attachments.borrow_mut() = downloads::SavedAttachments::load(&state.data_dir);
    assert_eq!(
        state.saved_attachments.borrow().destination(&msg.id),
        Some(renamed.as_path())
    );
    let rebuilt = file_actions::build_media_actions(&state, &msg);
    assert!(button(rebuilt.upcast_ref(), "file_open_folder").is_visible());

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
    assert!(!button(image_bubble.upcast_ref(), "file_open_folder").is_visible());
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
    assert!(button(image_bubble.upcast_ref(), "file_open_folder").is_visible());
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
    assert_eq!(
        state.saved_attachments.borrow().destination(&image.id),
        Some(images.join("photo-1.png").as_path())
    );
    request_media_download(&state, &image, "open_viewer");
    let viewer = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| {
            window.is_visible()
                && window.title().as_deref() == Some(crate::i18n::t("media_viewer_title").as_str())
        })
        .unwrap();
    let download = descendants(viewer.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            button.tooltip_text().as_deref() == Some(crate::i18n::t("media_download").as_str())
        })
        .unwrap();
    download.emit_clicked();
    assert_eq!(std::fs::read(images.join("photo-2.png")).unwrap(), original);
    assert_eq!(
        downloads::SavedAttachments::load(&state.data_dir).destination(&image.id),
        Some(images.join("photo-2.png").as_path())
    );
    viewer.close();
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
    let opened_files = temp.join("opened files ภาษาไทย");
    state.config.borrow_mut().download_dir_file = opened_files.to_string_lossy().into_owned();
    state.config.borrow_mut().download_ask_every_time = true;
    let text_source = temp.join("cached-text.bin");
    std::fs::write(&text_source, b"original attachment").unwrap();
    let text_file: MessageInfo = serde_json::from_value(serde_json::json!({
        "id":"opened-text", "contentType":"FILE", "filePath":text_source,
        "fileName":"บันทึก.txt", "text":"[File]"
    }))
    .unwrap();
    let file_bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    append_file_card(&state, &file_bubble, &text_file);
    let file_card = file_bubble
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let text_viewer = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|window| {
                window.is_visible()
                    && descendants(window.upcast_ref())
                        .iter()
                        .any(|widget| widget.is::<gtk::TextView>())
            })
    };
    let assert_viewer_text = |expected: &str| {
        let viewer = text_viewer().unwrap();
        let view = descendants(viewer.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::TextView>().ok())
            .unwrap();
        let buffer = view.buffer();
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            expected
        );
        viewer.close();
        drain();
    };
    file_card.emit_clicked();
    let saved_text = opened_files.join("บันทึก.txt");
    assert!(
        saved_text.is_file(),
        "opening a file must save it to the configured directory"
    );
    assert_eq!(std::fs::read(&saved_text).unwrap(), b"original attachment");
    assert_viewer_text("original attachment");
    assert!(button(file_bubble.upcast_ref(), "file_open_folder").is_visible());
    assert_eq!(
        downloads::SavedAttachments::load(&state.data_dir).destination(&text_file.id),
        Some(saved_text.as_path())
    );
    std::fs::write(&saved_text, b"edited saved attachment").unwrap();
    file_card.emit_clicked();
    assert_viewer_text("edited saved attachment");
    assert!(!opened_files.join("บันทึก-1.txt").exists());
    std::fs::remove_file(&text_source).unwrap();
    *state.session_ready.borrow_mut() = false;
    file_card.emit_clicked();
    assert_viewer_text("edited saved attachment");
    assert_eq!(requests(&log).len(), 7);
    *state.session_ready.borrow_mut() = true;
    std::fs::write(&text_source, b"original attachment").unwrap();

    let changed_folder = temp.join("changed-download-folder");
    state.config.borrow_mut().download_dir_file = changed_folder.to_string_lossy().into_owned();
    std::fs::create_dir_all(&changed_folder).unwrap();
    std::fs::write(changed_folder.join("บันทึก.txt"), b"existing unrelated file").unwrap();
    file_card.emit_clicked();
    let changed_text = changed_folder.join("บันทึก-1.txt");
    assert_eq!(
        std::fs::read(&changed_text).unwrap(),
        b"original attachment"
    );
    assert_eq!(
        std::fs::read(changed_folder.join("บันทึก.txt")).unwrap(),
        b"existing unrelated file"
    );
    assert_viewer_text("original attachment");
    std::fs::remove_file(&changed_text).unwrap();
    file_card.emit_clicked();
    assert_eq!(
        std::fs::read(&changed_text).unwrap(),
        b"original attachment"
    );
    assert_viewer_text("original attachment");

    let blocked_folder = temp.join("not-a-directory");
    std::fs::write(&blocked_folder, b"block download directory").unwrap();
    state.config.borrow_mut().download_dir_file = blocked_folder.to_string_lossy().into_owned();
    file_card.emit_clicked();
    assert!(
        text_viewer().is_none(),
        "a failed save must not open the cached source"
    );
    assert_eq!(
        state.saved_attachments.borrow().destination(&text_file.id),
        Some(changed_text.as_path())
    );

    state.config.borrow_mut().download_dir_file = opened_files.to_string_lossy().into_owned();
    let remote_text: MessageInfo = serde_json::from_value(serde_json::json!({
        "id":"remote-opened-text", "contentType":"FILE", "fileName":"remote.txt", "text":"[File]"
    }))
    .unwrap();
    let remote_bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    append_file_card(&state, &remote_bubble, &remote_text);
    remote_bubble
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    wait_for(|| requests(&log).len() == 8);
    let download = requests(&log).remove(7);
    assert_eq!(download["method"], "download_media");
    respond(
        &state,
        &download,
        serde_json::json!({"path":text_source,"fileName":"remote.txt","bytes":19}),
    );
    assert_eq!(
        std::fs::read(opened_files.join("remote.txt")).unwrap(),
        b"original attachment"
    );
    assert_viewer_text("original attachment");
    assert!(button(remote_bubble.upcast_ref(), "file_open_folder").is_visible());
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
