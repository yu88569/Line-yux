use super::*;
use super::message_share::{
    ShareMediaType, ShareSelection, ShareableKind, classify_shareable, collect_ordered_selection,
    resolve_share_filename,
};
use std::collections::{HashMap, HashSet};

fn msg_from_json(value: serde_json::Value) -> MessageInfo {
    serde_json::from_value(value).unwrap()
}

#[test]
fn classify_shareable_accepts_only_supported_real_messages() {
    let text = msg_from_json(serde_json::json!({
        "id": "101", "contentType": "NONE", "text": "hello world"
    }));
    assert!(matches!(
        classify_shareable(&text),
        Some(ShareableKind::Text(ref t)) if t == "hello world"
    ));

    let blank_text = msg_from_json(serde_json::json!({
        "id": "102", "contentType": "NONE", "text": "   \n "
    }));
    assert!(classify_shareable(&blank_text).is_none());

    let image = msg_from_json(serde_json::json!({
        "id": "103", "contentType": "IMAGE", "text": "[Image]"
    }));
    assert!(matches!(
        classify_shareable(&image),
        Some(ShareableKind::Media {
            o_type: ShareMediaType::Image,
            duration_ms: None
        })
    ));

    let video = msg_from_json(serde_json::json!({
        "id": "104", "contentType": "VIDEO", "durationMs": 4200
    }));
    assert!(matches!(
        classify_shareable(&video),
        Some(ShareableKind::Media {
            o_type: ShareMediaType::Video,
            duration_ms: Some(4200)
        })
    ));

    let audio = msg_from_json(serde_json::json!({
        "id": "105", "contentType": "AUDIO", "durationMs": 1500
    }));
    assert!(matches!(
        classify_shareable(&audio),
        Some(ShareableKind::Media {
            o_type: ShareMediaType::Audio,
            duration_ms: Some(1500)
        })
    ));

    let file = msg_from_json(serde_json::json!({
        "id": "106", "contentType": "FILE", "fileName": "doc.pdf"
    }));
    assert!(matches!(
        classify_shareable(&file),
        Some(ShareableKind::Media {
            o_type: ShareMediaType::File,
            duration_ms: None
        })
    ));

    let valid_loc = msg_from_json(serde_json::json!({
        "id": "107", "contentType": "LOCATION",
        "location": { "title": "Bangkok", "address": "TH", "latitude": 13.7563, "longitude": 100.5018 }
    }));
    assert!(matches!(
        classify_shareable(&valid_loc),
        Some(ShareableKind::Location(_))
    ));

    let invalid_loc = msg_from_json(serde_json::json!({
        "id": "108", "contentType": "LOCATION",
        "location": { "title": "Bad", "address": "", "latitude": 120.0, "longitude": 10.0 }
    }));
    assert!(classify_shareable(&invalid_loc).is_none());

    let sticker = msg_from_json(serde_json::json!({
        "id": "109", "contentType": "STICKER", "text": "[Sticker]"
    }));
    assert!(classify_shareable(&sticker).is_none());

    let flex_ct = msg_from_json(serde_json::json!({
        "id": "110", "contentType": "FLEX", "text": "Flex"
    }));
    assert!(classify_shareable(&flex_ct).is_none());

    let flex_payload = msg_from_json(serde_json::json!({
        "id": "111", "contentType": "NONE", "text": "Alt",
        "flex": { "altText": "Alt", "texts": ["Alt"] }
    }));
    assert!(classify_shareable(&flex_payload).is_none());

    let album_ct = msg_from_json(serde_json::json!({
        "id": "112", "contentType": "POSTNOTIFICATION", "text": "Album"
    }));
    assert!(classify_shareable(&album_ct).is_none());

    let album_payload = msg_from_json(serde_json::json!({
        "id": "113", "contentType": "NONE", "text": "Album",
        "album": { "chatMid": "c1", "albumId": "a1", "title": "Trip" }
    }));
    assert!(classify_shareable(&album_payload).is_none());

    let call = msg_from_json(serde_json::json!({
        "id": "114", "contentType": "CALL"
    }));
    assert!(classify_shareable(&call).is_none());

    let chat_event = msg_from_json(serde_json::json!({
        "id": "115", "contentType": "CHATEVENT"
    }));
    assert!(classify_shareable(&chat_event).is_none());

    let empty_id = msg_from_json(serde_json::json!({
        "id": "", "contentType": "NONE", "text": "hello"
    }));
    assert!(classify_shareable(&empty_id).is_none());

    let pending = msg_from_json(serde_json::json!({
        "id": "pending-42", "contentType": "NONE", "text": "sending"
    }));
    assert!(classify_shareable(&pending).is_none());
}

#[test]
fn collect_ordered_selection_sorts_chronologically_dedupes_and_merges_fresh_media() {
    let messages = vec![
        msg_from_json(serde_json::json!({
            "id": "30", "contentType": "NONE", "text": "third", "createdTime": 2000
        })),
        msg_from_json(serde_json::json!({
            "id": "20", "contentType": "IMAGE", "text": "[Image]", "createdTime": 1000
        })),
        msg_from_json(serde_json::json!({
            "id": "10", "contentType": "NONE", "text": "first", "createdTime": 1000
        })),
        msg_from_json(serde_json::json!({
            "id": "20", "contentType": "IMAGE", "text": "[Image]", "createdTime": 1000
        })),
        msg_from_json(serde_json::json!({
            "id": "40", "contentType": "STICKER", "text": "[Sticker]", "createdTime": 3000
        })),
    ];
    let mut fresh = HashMap::new();
    fresh.insert(
        "20".to_string(),
        msg_from_json(serde_json::json!({
            "id": "20", "contentType": "IMAGE", "filePath": "/tmp/20.full.png", "fileName": "slip.png"
        })),
    );
    let selected: HashSet<String> = ["30", "20", "10", "40"]
        .into_iter()
        .map(str::to_string)
        .collect();

    let ordered = collect_ordered_selection(&messages, &fresh, &selected);
    let ids: Vec<&str> = ordered.iter().map(|item| item.msg.id.as_str()).collect();
    assert_eq!(ids, vec!["10", "20", "30"]);
    assert_eq!(ordered[1].msg.file_path.as_deref(), Some("/tmp/20.full.png"));
    assert_eq!(ordered[1].msg.file_name.as_deref(), Some("slip.png"));
}

#[test]
fn resolve_share_filename_preserves_original_or_path_extension() {
    let file_msg = msg_from_json(serde_json::json!({
        "id": "55", "contentType": "FILE", "fileName": "รายงาน.xlsx", "text": "[File]"
    }));
    assert_eq!(
        resolve_share_filename(&file_msg, None, "/tmp/55.bin"),
        "รายงาน.xlsx"
    );

    let img_msg = msg_from_json(serde_json::json!({
        "id": "56", "contentType": "IMAGE", "text": "[Image]"
    }));
    assert_eq!(
        resolve_share_filename(&img_msg, None, "/tmp/cache/56.full.png"),
        "56.png"
    );
    assert_eq!(
        resolve_share_filename(&img_msg, Some("receipt.webp"), "/tmp/cache/56.full.webp"),
        "receipt.webp"
    );
}

#[test]
fn share_i18n_keys_exist_in_thai_and_english() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let keys = [
        "share_message",
        "share_select_title",
        "share_button_count",
        "share_choose_chats",
        "share_selected_summary",
        "share_selected_summary_one",
        "share_send_count",
        "share_sending",
        "share_sent_one",
        "share_sent_many",
        "share_sent_multi_chats",
        "share_sent_one_multi_chats",
        "share_partial",
        "share_failed",
    ];
    for lang in ["th", "en"] {
        let bundle = crate::i18n::I18n::load(&root, lang);
        for key in keys {
            let value = bundle.t(key);
            assert_ne!(value, key, "missing translation for {key} in {lang}");
            assert!(!value.trim().is_empty(), "empty translation for {key} in {lang}");
        }
    }
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
    assert!(condition(), "timed out waiting for condition");
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

fn requests(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|req| req["method"] != "resolve_sender_avatar")
        .collect()
}

fn respond_ok(state: &AppState, request: &serde_json::Value, result: serde_json::Value) {
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

fn respond_err(state: &AppState, request: &serde_json::Value, error: &str) {
    handle_event(
        state,
        ProtocolEvent::Response {
            id: request["id"].as_u64().unwrap(),
            ok: false,
            result: serde_json::Value::Null,
            error: Some(error.to_string()),
        },
    );
}

fn render_widget_png(window: &gtk::Window, widget: &gtk::Widget, out_path: &str) {
    let frame = Rc::new(std::cell::Cell::new(false));
    let ready = frame.clone();
    window.add_tick_callback(move |_, _| {
        ready.set(true);
        glib::ControlFlow::Break
    });
    wait_for(|| frame.get());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
    if let Some(node) = snapshot.to_node()
        && let Some(renderer) = window.renderer()
    {
        renderer
            .render_texture(&node, None)
            .save_to_png(out_path)
            .unwrap();
    }
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_message_share_selects_filters_and_sends_chronologically() {
    libadwaita::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    let temp = std::env::temp_dir().join(format!("line-msg-share-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(temp.join("protocol")).unwrap();
    let log = temp.join("requests.jsonl");
    let fake = temp.join("protocol/line-gtk-protocol");
    std::fs::write(
        &fake,
        format!(
            "#!/usr/bin/python3\nimport sys\nfor line in sys.stdin:\n with open({:?}, 'a') as log:\n  log.write(line)\n",
            log.to_string_lossy()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();

    let source_mid = "u00000000000000000000000000000001";
    let alpha_mid = "u0000000000000000000000000000000a";
    let beta_mid = "u0000000000000000000000000000000b";
    let hidden_mid = "u0000000000000000000000000000000c";

    let data = temp.join("data");
    let mut cfg = AppConfig {
        animations: false,
        tray_enabled: false,
        notifications: false,
        auto_mark_read: false,
        language: "en".into(),
        ..AppConfig::default()
    };
    cfg.chat_preferences.entry(beta_mid.into()).or_default().pinned = true;
    cfg.chat_preferences
        .entry(hidden_mid.into())
        .or_default()
        .hidden = true;
    cfg.save(&data);

    let app = Application::builder()
        .application_id("dev.linegtk.MessageShareFixture")
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
    wire_actions(&state);
    *state.session_ready.borrow_mut() = true;
    *state.restored_last_chat.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state.window.set_default_size(980, 720);
    state.chat_title.set_text("Source Chat");
    *state.current_chat.borrow_mut() = Some(source_mid.into());

    apply_chats(
        &state,
        vec![
            serde_json::from_value(serde_json::json!({"mid": source_mid, "name": "Source Chat", "lastActivity": 300})).unwrap(),
            serde_json::from_value(serde_json::json!({"mid": alpha_mid, "name": "Alpha Friend", "lastActivity": 200})).unwrap(),
            serde_json::from_value(serde_json::json!({"mid": beta_mid, "name": "Beta Pinned", "lastActivity": 100})).unwrap(),
            serde_json::from_value(serde_json::json!({"mid": hidden_mid, "name": "Hidden Chat", "lastActivity": 999})).unwrap(),
        ],
        false,
    );

    let cached_video = temp.join("1003.mp4");
    std::fs::write(&cached_video, vec![0u8; 512]).unwrap();
    let cached_audio = temp.join("1004.m4a");
    std::fs::write(&cached_audio, vec![0u8; 512]).unwrap();
    let cached_file = temp.join("1005.pdf");
    std::fs::write(&cached_file, b"%PDF-1.4\n").unwrap();
    let img_preview = temp.join("1002.thumb.png");
    let remote_dir = temp.join("remote");
    std::fs::create_dir_all(&remote_dir).unwrap();
    let full_png = remote_dir.join("1002.full.png");
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 32).unwrap();
    pixels.fill(0x228833ff);
    pixels.savev(&img_preview, "png", &[]).unwrap();
    pixels.fill(0x1166ccff);
    pixels.savev(&full_png, "png", &[]).unwrap();

    let messages: Vec<MessageInfo> = vec![
        serde_json::from_value(serde_json::json!({
            "id": "1001", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "NONE", "text": "First text message", "createdTime": 1700000001000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1002", "from": "u-self", "to": source_mid, "mine": true,
            "contentType": "IMAGE", "text": "[Image]", "imagePath": img_preview, "createdTime": 1700000002000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1003", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "VIDEO", "text": "[Video]", "filePath": cached_video, "durationMs": 5000, "createdTime": 1700000003000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1004", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "AUDIO", "text": "Voice message", "audioPath": cached_audio, "filePath": cached_audio, "durationMs": 3000, "createdTime": 1700000004000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1005", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "FILE", "text": "[File]", "filePath": cached_file, "fileName": "สลิปโอนเงิน.pdf", "fileSize": 9, "createdTime": 1700000005000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1006", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "LOCATION", "text": "Bangkok", "createdTime": 1700000006000_i64,
            "location": { "title": "Siam", "address": "Bangkok", "latitude": 13.7465, "longitude": 100.5347 }
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1007", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "STICKER", "text": "[Sticker]", "stickerId": "1", "stickerPackageId": "1", "createdTime": 1700000007000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1008", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "FLEX", "text": "Flex card", "flex": { "altText": "Flex card", "texts": ["Flex card"] }, "createdTime": 1700000008000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "1009", "from": source_mid, "to": "u-self", "mine": false,
            "contentType": "POSTNOTIFICATION", "text": "Album", "album": { "chatMid": source_mid, "albumId": "1", "title": "Photos" }, "createdTime": 1700000009000_i64
        })).unwrap(),
        serde_json::from_value(serde_json::json!({
            "id": "pending-1", "from": "u-self", "to": source_mid, "mine": true,
            "contentType": "NONE", "text": "Pending", "createdTime": 1700000010000_i64
        })).unwrap(),
    ];
    for m in &messages {
        append_message(&state, m, false);
    }

    state.window.present();
    wait_for(|| state.message_list.rows().len() >= 10);

    let row_by_id = |id: &str| -> gtk::Widget {
        state
            .message_list
            .rows()
            .into_iter()
            .find(|row| row.has_css_class("line-history-message") && row.widget_name() == id)
            .unwrap()
    };

    // 1) Right-click text bubble -> Share menu button enters selection mode.
    let text_row = row_by_id("1001");
    wait_for(|| text_row.is_mapped());
    let text_bubble = descendants(&text_row)
        .into_iter()
        .find(|w| w.has_css_class("line-bubble"))
        .unwrap();
    let rclick = text_bubble
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|c| {
            c.downcast::<gtk::GestureClick>()
                .ok()
                .filter(|g| g.button() == 3)
        })
        .unwrap();
    rclick.emit_by_name::<()>("pressed", &[&1i32, &10.0_f64, &10.0_f64]);
    drain();

    let popover = descendants(&text_bubble)
        .into_iter()
        .find_map(|w| {
            w.downcast::<gtk::Popover>()
                .ok()
                .filter(|p| p.is_visible())
        })
        .expect("right-click opens message menu popover");
    let share_menu_btn = descendants(popover.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some(crate::i18n::t("share_message").as_str()))
        .expect("share menu button exists");
    share_menu_btn.emit_clicked();
    drain();

    assert_eq!(
        state.chat_header_stack.visible_child_name().as_deref(),
        Some("select")
    );
    assert_eq!(
        state.composer_stack.visible_child_name().as_deref(),
        Some("share")
    );
    assert_eq!(state.share_confirm_btn.label().as_deref(), Some("Share (1)"));
    assert!(state.share_confirm_btn.is_sensitive());

    // Exactly 6 shareable rows have visible checkboxes; non-shareable have none.
    let mut visible_checks = 0;
    for id in ["1001", "1002", "1003", "1004", "1005", "1006"] {
        let r = row_by_id(id);
        let check = descendants(&r)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .find(|c| c.has_css_class("line-msg-select-check"))
            .unwrap();
        assert!(check.is_visible(), "row {id} should show selection checkbox");
        visible_checks += 1;
    }
    assert_eq!(visible_checks, 6);
    for id in ["1007", "1008", "1009", "pending-1"] {
        let r = row_by_id(id);
        assert!(
            !descendants(&r)
                .into_iter()
                .any(|w| w.has_css_class("line-msg-select-check")),
            "row {id} must not have selection checkbox"
        );
    }

    // 2) Clicking the image row toggles selection without opening a media viewer window.
    let img_row = row_by_id("1002");
    let row_click = img_row
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|c| {
            c.downcast::<gtk::GestureClick>()
                .ok()
                .filter(|g| g.button() == 1)
        })
        .unwrap();
    row_click.emit_by_name::<()>("pressed", &[&1i32, &200.0_f64, &12.0_f64]);
    drain();
    assert_eq!(state.share_confirm_btn.label().as_deref(), Some("Share (2)"));
    assert!(!gtk::Window::list_toplevels().into_iter().any(|w| {
        w.downcast::<gtk::Window>()
            .ok()
            .is_some_and(|win| {
                win.is_visible()
                    && win.title().as_deref() == Some(crate::i18n::t("media_viewer_title").as_str())
            })
    }));

    // 3) Direct checkbox toggle works once (no double-toggle) and unchecking all disables Share.
    let img_check = descendants(&img_row)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .find(|c| c.has_css_class("line-msg-select-check"))
        .unwrap();
    img_check.set_active(false);
    drain();
    assert_eq!(state.share_confirm_btn.label().as_deref(), Some("Share (1)"));
    let text_check = descendants(&text_row)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .find(|c| c.has_css_class("line-msg-select-check"))
        .unwrap();
    text_check.set_active(false);
    drain();
    assert!(!state.share_confirm_btn.is_sensitive());

    // 4) Right-click inside selection mode does NOT open a context menu popover.
    rclick.emit_by_name::<()>("pressed", &[&1i32, &10.0_f64, &10.0_f64]);
    drain();
    assert!(!descendants(&text_bubble).into_iter().any(|w| {
        w.downcast::<gtk::Popover>()
            .ok()
            .is_some_and(|p| p.is_visible())
    }));

    // 5) Back button exits selection mode cleanly.
    state.share_select_back.emit_clicked();
    drain();
    assert_eq!(
        state.chat_header_stack.visible_child_name().as_deref(),
        Some("normal")
    );
    assert_eq!(
        state.composer_stack.visible_child_name().as_deref(),
        Some("compose")
    );
    assert_eq!(*state.share_selection.borrow(), ShareSelection::Off);

    // 6) Re-enter in Thai, select 4 items (Text 1001, uncached Image 1002, File 1005, Location 1006).
    state.config.borrow_mut().language = "th".into();
    apply_ui_language(&state);
    message_share::enter_selection_mode(&state, "1001");
    for id in ["1002", "1005", "1006"] {
        let r = row_by_id(id);
        let check = descendants(&r)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .find(|c| c.has_css_class("line-msg-select-check"))
            .unwrap();
        check.set_active(true);
    }
    drain();
    assert_eq!(state.share_select_title.text().as_str(), "เลือกข้อความ");
    assert_eq!(state.share_confirm_btn.label().as_deref(), Some("แชร์ (4)"));
    render_widget_png(
        state.window.upcast_ref(),
        state.conversation.upcast_ref(),
        "/tmp/line-gtk-share-select.png",
    );

    // 7) Open multi-chat picker, verify hidden excluded & pinned first, search preserves checked.
    state.share_confirm_btn.emit_clicked();
    let picker = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .find(|w| {
                w.is_visible()
                    && w.title().as_deref() == Some(crate::i18n::t("share_choose_chats").as_str())
            })
    };
    wait_for(|| picker().is_some());
    let picker_win = picker().unwrap();
    let picker_list = descendants(picker_win.upcast_ref())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::ListBox>().ok())
        .unwrap();
    let mut row_names = Vec::new();
    let mut idx = 0;
    while let Some(r) = picker_list.row_at_index(idx) {
        let label = descendants(r.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .find(|l| l.has_css_class("line-chat-name"))
            .unwrap();
        row_names.push(label.text().to_string());
        idx += 1;
    }
    assert_eq!(row_names, vec!["Beta Pinned", "Source Chat", "Alpha Friend"]);

    let search = descendants(picker_win.upcast_ref())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::SearchEntry>().ok())
        .unwrap();
    search.set_text("Alpha");
    search.emit_by_name::<()>("search-changed", &[]);
    drain();
    assert!(!picker_list.row_at_index(0).unwrap().is_visible());
    assert!(!picker_list.row_at_index(1).unwrap().is_visible());
    assert!(picker_list.row_at_index(2).unwrap().is_visible());
    let alpha_check = descendants(picker_list.row_at_index(2).unwrap().upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .next()
        .unwrap();
    alpha_check.set_active(true);
    drain();

    search.set_text("Beta");
    search.emit_by_name::<()>("search-changed", &[]);
    drain();
    assert!(picker_list.row_at_index(0).unwrap().is_visible());
    assert!(!picker_list.row_at_index(2).unwrap().is_visible());
    let beta_check = descendants(picker_list.row_at_index(0).unwrap().upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .next()
        .unwrap();
    beta_check.set_active(true);
    drain();

    search.set_text("");
    search.emit_by_name::<()>("search-changed", &[]);
    drain();
    let send_btn = descendants(picker_win.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some("ส่ง (2)"))
        .expect("picker send button displays count");
    assert!(send_btn.is_sensitive());
    render_widget_png(&picker_win, picker_win.upcast_ref(), "/tmp/line-gtk-share-picker.png");

    // 8) Click Send: closes picker, exits selection mode, stays in source chat, prepares uncached image then sends sequentially.
    send_btn.emit_clicked();
    drain();
    assert!(picker().is_none());
    assert!(!message_share::is_selecting(&state));
    assert_eq!(state.current_chat.borrow().as_deref(), Some(source_mid));

    wait_for(|| requests(&log).len() == 1);
    let dl_req = requests(&log).remove(0);
    assert_eq!(dl_req["method"], "download_media");
    assert_eq!(
        dl_req["params"],
        serde_json::json!({"chatMid": source_mid, "messageId": "1002"})
    );
    respond_ok(
        &state,
        &dl_req,
        serde_json::json!({"path": full_png, "bytes": std::fs::metadata(&full_png).unwrap().len()}),
    );

    // Next 4 sends to Beta Pinned (row 0), then 4 sends to Alpha Friend (row 1).
    let expected_sends = [
        (beta_mid, "send_message", serde_json::json!({"chatMid": beta_mid, "text": "First text message"})),
        (beta_mid, "send_media", serde_json::json!({"chatMid": beta_mid, "filePath": full_png, "oType": "image", "fileName": "1002.png"})),
        (beta_mid, "send_media", serde_json::json!({"chatMid": beta_mid, "filePath": cached_file, "oType": "file", "fileName": "สลิปโอนเงิน.pdf"})),
        (beta_mid, "send_location", serde_json::json!({"chatMid": beta_mid, "location": {"title": "Siam", "address": "Bangkok", "latitude": 13.7465, "longitude": 100.5347}})),
        (alpha_mid, "send_message", serde_json::json!({"chatMid": alpha_mid, "text": "First text message"})),
        (alpha_mid, "send_media", serde_json::json!({"chatMid": alpha_mid, "filePath": full_png, "oType": "image", "fileName": "1002.png"})),
        (alpha_mid, "send_media", serde_json::json!({"chatMid": alpha_mid, "filePath": cached_file, "oType": "file", "fileName": "สลิปโอนเงิน.pdf"})),
        (alpha_mid, "send_location", serde_json::json!({"chatMid": alpha_mid, "location": {"title": "Siam", "address": "Bangkok", "latitude": 13.7465, "longitude": 100.5347}})),
    ];

    for (idx, (_mid, method, params)) in expected_sends.iter().enumerate() {
        let want_len = 2 + idx;
        wait_for(|| requests(&log).len() == want_len);
        let req = requests(&log).remove(want_len - 1);
        assert_eq!(req["method"], *method, "step {idx}");
        assert_eq!(req["params"], *params, "step {idx}");
        if idx == 5 {
            // Inject one failure mid-batch; remaining items must continue and complete!
            respond_err(&state, &req, "simulated_failure");
        } else {
            respond_ok(
                &state,
                &req,
                serde_json::json!({"message": {"id": format!("sent-{idx}"), "text": "ok", "contentType": "NONE"}}),
            );
        }
    }
    assert!(state.share_batches.borrow().is_empty());
    assert_eq!(requests(&log).len(), 9);

    // 9) fail_pending_shares aborts an active batch cleanly on disconnect.
    message_share::start_share_batch(
        &state,
        source_mid.to_string(),
        vec![super::message_share::ShareSourceItem {
            msg: messages[0].clone(),
            kind: ShareableKind::Text("abort test".into()),
        }],
        vec![alpha_mid.to_string()],
    );
    assert!(!state.share_batches.borrow().is_empty());
    message_share::fail_pending_shares(&state);
    assert!(state.share_batches.borrow().is_empty());
    assert!(!state.pending.borrow().values().any(|p| matches!(
        p,
        Pending::SharePrepareMedia { .. } | Pending::ShareSend { .. }
    )));

    state.window.close();
    drain();
    let _ = std::fs::remove_dir_all(temp);
}
