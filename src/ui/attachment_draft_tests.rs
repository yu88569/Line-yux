use super::*;

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() && std::time::Instant::now() < deadline {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
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

fn response(state: &AppState, request: &serde_json::Value, ok: bool) {
    handle_event(
        state,
        ProtocolEvent::Response {
            id: request["id"].as_u64().unwrap(),
            ok,
            result: serde_json::json!({}),
            error: (!ok).then(|| "fixture failure".into()),
        },
    );
}

fn drag_provider(state: &AppState, provider: gdk::ContentProvider, composer: bool) -> bool {
    fn ignore_test_toasts(widget: &gtk::Widget) {
        if widget.type_().name() == "AdwToastWidget" {
            widget.set_can_target(false);
            return;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            ignore_test_toasts(&current);
        }
    }
    ignore_test_toasts(state.toast_overlay.upcast_ref());
    let label = gtk::Label::new(Some("Drag fixture"));
    label.set_size_request(-1, 90);
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::COPY);
    drag.set_content(Some(&provider));
    label.add_controller(drag);
    state.conversation.prepend(&label);
    state.window.set_title(Some("LINE synthetic drag target"));
    state.window.set_decorated(false);
    state.composer.set_sensitive(true);
    state.window.unmaximize();
    state.window.unfullscreen();
    state.stack.set_visible_child_name("shell");
    state.window.present();
    wait_for(|| label.is_mapped() && label.height() > 0 && state.composer.width() > 0);
    let (origin_x, origin_y) = if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let script = r#"local w = hl.get_window("title:^(LINE synthetic drag target)$")
assert(w, "synthetic drag window missing")
hl.dispatch(hl.dsp.window.fullscreen_state({window=w, internal=0, client=0}))
hl.dispatch(hl.dsp.window.float({window=w, action="set"}))
hl.dispatch(hl.dsp.window.move({window=w, monitor=0, follow=true}))
hl.dispatch(hl.dsp.window.resize({window=w, x=900, y=650}))
hl.dispatch(hl.dsp.window.move({window=w, x=300, y=150}))"#;
        let output = std::process::Command::new("hyprctl")
            .args(["eval", script])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
        let ready = Rc::new(std::cell::Cell::new(false));
        let done = ready.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
            done.set(true)
        });
        wait_for(|| ready.get());
        let output = std::process::Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .unwrap();
        let clients: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let window = clients
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["title"] == "LINE synthetic drag target")
            .unwrap();
        (
            window["at"][0].as_i64().unwrap() as i32,
            window["at"][1].as_i64().unwrap() as i32,
        )
    } else {
        let output = std::process::Command::new("xdotool")
            .args(["search", "--name", "^LINE synthetic drag target$"])
            .output()
            .unwrap();
        let id = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .last()
            .unwrap()
            .to_string();
        assert!(
            std::process::Command::new("xdotool")
                .args(["windowactivate", "--sync", &id])
                .status()
                .unwrap()
                .success()
        );
        let output = std::process::Command::new("xdotool")
            .args(["getwindowgeometry", "--shell", &id])
            .output()
            .unwrap();
        let geometry = String::from_utf8(output.stdout).unwrap();
        let coordinate = |name: &str| {
            geometry
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .unwrap()
                .parse::<i32>()
                .unwrap()
        };
        (coordinate("X="), coordinate("Y="))
    };
    let source = label
        .compute_point(&state.window, &gtk::graphene::Point::new(80.0, 40.0))
        .unwrap();
    let sx = (origin_x + source.x() as i32).to_string();
    let sy = (origin_y + source.y() as i32).to_string();
    let mx = (origin_x + source.x() as i32 + 30).to_string();
    let target: gtk::Widget = if composer {
        state.composer.clone().upcast()
    } else {
        state.conversation.clone().upcast()
    };
    let point = target
        .compute_point(
            &state.window,
            &gtk::graphene::Point::new(40.0, if composer { 12.0 } else { 250.0 }),
        )
        .unwrap();
    let x = (origin_x + point.x() as i32).to_string();
    let y = (origin_y + point.y() as i32).to_string();

    eprintln!("Native drag {} {} -> {} {}", sx, sy, x, y);
    let mut driver = if let Some(pointer) = std::env::var_os("LINE_TEST_POINTER") {
        let mut driver = std::process::Command::new(pointer);
        driver.args([&sx, &sy, &x, &y]);
        driver
    } else {
        let mut driver = std::process::Command::new("xdotool");
        driver.args([
            "mousemove",
            &sx,
            &sy,
            "sleep",
            "0.1",
            "mousedown",
            "1",
            "sleep",
            "0.1",
            "mousemove",
            &mx,
            &sy,
            "sleep",
            "0.3",
            "mousemove",
            &x,
            &y,
            "sleep",
            "0.8",
            "mouseup",
            "1",
        ]);
        driver
    };
    let mut command = driver.spawn().unwrap();
    let mut highlighted = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        highlighted |= state.conversation.has_css_class("line-attachment-drop");
        if let Some(status) = command.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let done = Rc::new(RefCell::new(false));
    let finished = done.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
        *finished.borrow_mut() = true
    });
    wait_for(|| *done.borrow());
    assert!(!state.conversation.has_css_class("line-attachment-drop"));

    state.conversation.remove(&label);
    highlighted
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_attachments_hold_remove_confirm_retry_and_chat_isolation() {
    libadwaita::init().unwrap();
    let temp = std::env::temp_dir().join(format!("line-attachment-fixture-{}", std::process::id()));
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
        .application_id("dev.linegtk.AttachmentFixture")
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
    *state.current_chat.borrow_mut() = Some("chat-a".into());
    let long_name = format!("{}.docx", "a".repeat(230));
    let long_file = temp.join(&long_name);
    std::fs::write(&long_file, b"office fixture").unwrap();
    assert!(stage_attachment(&state, "long-name", long_file));
    assert_eq!(
        state.attachment_drafts.borrow()["long-name"][0].name,
        long_name
    );
    assert!(
        state.attachment_drafts.borrow()["long-name"][0]
            .path
            .file_name()
            .unwrap()
            .len()
            < 100
    );
    let long_extension = temp.join(format!("a.{}", "x".repeat(230)));
    std::fs::write(&long_extension, b"extension fixture").unwrap();
    assert!(stage_attachment(&state, "long-extension", long_extension));
    assert_eq!(
        state.attachment_drafts.borrow()["long-extension"][0]
            .path
            .extension()
            .unwrap(),
        "bin"
    );
    let image = temp.join("ภาพหนึ่ง.png");
    let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 4, 4).unwrap();
    pixbuf.fill(0xff0000ff);
    pixbuf.savev(&image, "png", &[]).unwrap();
    let image2 = temp.join("ภาพสอง.png");
    pixbuf.fill(0x00ff00ff);
    pixbuf.savev(&image2, "png", &[]).unwrap();
    let pdf = temp.join("รายงาน.pdf");
    std::fs::write(&pdf, b"%PDF-1.4 fixture").unwrap();
    assert!(stage_attachment(&state, "chat-a", image.clone()));
    assert!(stage_attachment(&state, "chat-a", pdf.clone()));
    assert!(stage_attachment(&state, "chat-a", image2.clone()));
    assert!(requests(&log).is_empty());
    assert!(state.attachment_scroll.is_visible());
    let staged = state.attachment_drafts.borrow()["chat-a"].clone();
    assert_eq!(
        std::fs::read(&staged[0].path).unwrap(),
        std::fs::read(&image).unwrap()
    );
    assert_eq!(
        std::fs::read(&staged[2].path).unwrap(),
        std::fs::read(&image2).unwrap()
    );
    let remove = state
        .attachment_strip
        .first_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    remove.emit_clicked();
    remove_attachment(&state, "chat-a", staged[2].id);
    state.send_btn.emit_clicked();
    assert!(requests(&log).is_empty());
    assert_eq!(state.attachment_drafts.borrow()["chat-a"].len(), 1);
    *state.session_ready.borrow_mut() = true;
    state.send_btn.emit_clicked();
    state.send_btn.emit_clicked();
    wait_for(|| requests(&log).len() == 1);
    let sent = requests(&log);
    assert_eq!(sent[0]["params"]["fileName"], "รายงาน.pdf");
    assert_eq!(sent[0]["params"]["oType"], "file");
    assert_eq!(sent[0]["params"]["chatMid"], "chat-a");
    response(&state, &sent[0], false);
    assert_eq!(
        state.attachment_drafts.borrow()["chat-a"][0].status,
        DraftStatus::Failed
    );
    state.send_btn.emit_clicked();
    wait_for(|| requests(&log).len() == 2);
    response(&state, &requests(&log)[1], true);
    assert!(state.attachment_drafts.borrow()["chat-a"].is_empty());
    assert!(!state.attachment_scroll.is_visible());

    stage_attachment(&state, "chat-a", image.clone());
    *state.current_chat.borrow_mut() = Some("chat-b".into());
    refresh_attachment_drafts(&state);
    assert!(!state.attachment_scroll.is_visible());
    state.send_btn.emit_clicked();
    assert_eq!(requests(&log).len(), 2);
    stage_attachment(&state, "chat-a", pdf.clone());
    assert!(!state.attachment_scroll.is_visible());
    assert_eq!(state.attachment_drafts.borrow()["chat-a"].len(), 2);
    assert!(send_local_image_file(&state, &image));
    state.composer.set_text("caption");
    state.send_btn.emit_clicked();
    wait_for(|| requests(&log).len() == 4);
    let sent = requests(&log);
    assert_eq!(sent[2]["params"]["oType"], "image");
    assert_eq!(sent[2]["params"]["chatMid"], "chat-b");
    assert_eq!(sent[3]["method"], "send_message");
    assert_eq!(sent[3]["params"]["text"], "caption");
    assert_eq!(state.composer.text(), "");
    response(&state, &sent[2], true);
    *state.current_chat.borrow_mut() = Some("chat-a".into());
    refresh_attachment_drafts(&state);
    state.send_btn.emit_clicked();
    wait_for(|| requests(&log).len() == 6);
    handle_event(&state, ProtocolEvent::Exited(1));
    assert!(!attachment_batch_pending(&state, "chat-a"));
    assert!(state.attachment_requests.borrow().is_empty());
    assert_eq!(state.attachment_drafts.borrow()["chat-a"].len(), 2);
    assert!(
        state.attachment_drafts.borrow()["chat-a"]
            .iter()
            .all(|item| item.status == DraftStatus::Failed)
    );
    *state.current_chat.borrow_mut() = Some("paste-original".into());
    let files = gdk::FileList::from_array(&[gio::File::for_path(&image)]);
    state
        .composer
        .clipboard()
        .set_content(Some(&gdk::ContentProvider::for_value(&files.to_value())))
        .unwrap();
    assert!(try_paste_clipboard_attachment(&state));
    *state.current_chat.borrow_mut() = Some("paste-new".into());
    wait_for(|| {
        state
            .attachment_drafts
            .borrow()
            .get("paste-original")
            .is_some_and(|items| items.len() == 1)
    });
    assert!(state.attachment_drafts.borrow().get("paste-new").is_none());
    let texture = gdk::Texture::from_file(&gio::File::for_path(&image)).unwrap();
    state.composer.clipboard().set_texture(&texture);
    assert!(try_paste_clipboard_attachment(&state));
    *state.current_chat.borrow_mut() = Some("drop-chat".into());
    wait_for(|| {
        state
            .attachment_drafts
            .borrow()
            .get("paste-new")
            .is_some_and(|items| items.len() == 1)
    });
    for value in [
        files.to_value(),
        texture.to_value(),
        gio::File::for_path(&image).to_value(),
    ] {
        assert!(
            AttachmentDrop::from_value(&value)
                .unwrap()
                .stage(&state, "drop-chat")
        );
    }
    assert_eq!(state.attachment_drafts.borrow()["drop-chat"].len(), 3);
    assert_eq!(requests(&log).len(), 6);
    assert!(AttachmentDrop::from_value(&gio::File::for_path(&temp).to_value()).is_none());
    assert!(
        AttachmentDrop::from_value(&gio::File::for_uri("https://example.com/photo.png").to_value())
            .is_none()
    );
    let mixed = gdk::FileList::from_array(&[
        gio::File::for_path(&temp),
        gio::File::for_path(&image),
        gio::File::for_path(&pdf),
    ]);
    match AttachmentDrop::from_value(&mixed.to_value()).unwrap() {
        AttachmentDrop::Files(paths) => assert_eq!(paths, vec![image.clone(), pdf.clone()]),
        _ => panic!("local files must retain their original paths"),
    }
    assert!(!AttachmentDrop::Files(vec![temp.join("missing.png")]).stage(&state, "drop-chat"));
    let before_texture = std::fs::read_dir(state.data_dir.join("cache/media"))
        .unwrap()
        .count();
    assert!(AttachmentDrop::Image(texture.clone()).stage(&state, "texture-cleanup"));
    assert_eq!(
        std::fs::read_dir(state.data_dir.join("cache/media"))
            .unwrap()
            .count(),
        before_texture + 1
    );
    if std::env::var("LINE_TEST_REAL_DROP").as_deref() == Ok("1") {
        let thai = temp.join("รูปภาพ ทดสอบ.png");
        std::fs::copy(&image, &thai).unwrap();
        let uri = format!(
            "# fixture\r\n{}\r\n{}\r\n",
            gio::File::for_path(&thai).uri(),
            gio::File::for_path(temp.join(&long_name)).uri()
        );
        let provider = gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_owned(uri.into_bytes()),
        );
        assert!(
            drag_provider(&state, provider, true),
            "valid URI drop must highlight composer"
        );
        assert_eq!(state.attachment_drafts.borrow()["drop-chat"].len(), 5);
        assert_eq!(
            state.attachment_drafts.borrow()["drop-chat"][3].name,
            "รูปภาพ ทดสอบ.png"
        );
        let png = gdk::ContentProvider::for_bytes(
            "image/png",
            &glib::Bytes::from_owned(std::fs::read(&image).unwrap()),
        );
        let remote = gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_static(b"https://example.com/photo.png\r\n"),
        );
        assert!(drag_provider(
            &state,
            gdk::ContentProvider::new_union(&[remote, png]),
            false
        ));
        assert_eq!(state.attachment_drafts.borrow()["drop-chat"].len(), 6);
        assert_eq!(
            state.attachment_drafts.borrow()["drop-chat"][5].o_type,
            "image"
        );
        assert!(!drag_provider(
            &state,
            gdk::ContentProvider::for_value(&"ordinary text".to_value()),
            true
        ));
        assert_eq!(state.composer.text().as_str(), "ordinary text");
        let paintable = gtk::WidgetPaintable::new(Some(&state.conversation));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            state.conversation.width() as f64,
            state.conversation.height() as f64,
        );
        state
            .window
            .renderer()
            .unwrap()
            .render_texture(
                snapshot.to_node().unwrap(),
                Some(&gtk::graphene::Rect::new(
                    0.0,
                    0.0,
                    state.conversation.width() as f32,
                    state.conversation.height() as f32,
                )),
            )
            .save_to_png("/tmp/line-gtk-chat-drop-preview.png")
            .unwrap();

        state.composer.set_text("");
        assert_eq!(state.attachment_drafts.borrow()["drop-chat"].len(), 6);
        assert_eq!(requests(&log).len(), 6);
    }

    let viewer = gtk::Window::new();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    append_image_viewer(&state, &bar, &body, image.to_str().unwrap(), &viewer);
    let mut widgets = Vec::new();
    let mut child = bar.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        widgets.push(widget);
    }
    let draw = widgets
        .iter()
        .find_map(|widget| widget.clone().downcast::<gtk::ToggleButton>().ok())
        .unwrap();
    draw.set_active(true);
    let area = body.first_child().unwrap();
    let controllers = area.observe_controllers();
    let drag = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
        .unwrap();
    drag.emit_by_name::<()>("drag-begin", &[&1.0_f64, &1.0_f64]);
    drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
    let attach = widgets
        .iter()
        .filter_map(|widget| widget.clone().downcast::<gtk::Button>().ok())
        .find(|button| button.icon_name().as_deref() == Some("mail-send-symbolic"))
        .unwrap();
    attach.emit_clicked();
    let last = state.attachment_drafts.borrow()["drop-chat"]
        .last()
        .unwrap()
        .clone();
    assert_eq!(last.o_type, "image");
    assert_ne!(
        std::fs::read(last.path).unwrap(),
        std::fs::read(&image).unwrap()
    );
    assert_eq!(requests(&log).len(), 6);
    *state.current_chat.borrow_mut() = Some("long-name".into());
    *state.session_ready.borrow_mut() = true;
    refresh_attachment_drafts(&state);
    state.send_btn.emit_clicked();
    wait_for(|| requests(&log).len() == 7);
    assert_eq!(requests(&log)[6]["params"]["fileName"], long_name);
    response(&state, &requests(&log)[6], true);
    let cache_files = || {
        std::fs::read_dir(state.data_dir.join("cache/media"))
            .unwrap()
            .count()
    };
    let before_cancelled_paste = cache_files();
    *state.current_chat.borrow_mut() = Some("invalidated-session".into());
    state
        .composer
        .clipboard()
        .set_content(Some(&gdk::ContentProvider::for_value(&files.to_value())))
        .unwrap();
    assert!(try_paste_clipboard_attachment(&state));
    invalidate_attachment_session(&state);
    state.attachment_drafts.borrow_mut().clear();
    *state.current_chat.borrow_mut() = None;
    let done = Rc::new(RefCell::new(false));
    let finished = done.clone();
    glib::idle_add_local_once(move || {
        *finished.borrow_mut() = true;
    });
    wait_for(|| *done.borrow());
    assert!(state.attachment_drafts.borrow().is_empty());
    *state.current_chat.borrow_mut() = Some("invalidated-texture".into());
    state.composer.clipboard().set_texture(&texture);
    assert!(try_paste_clipboard_attachment(&state));
    invalidate_attachment_session(&state);
    state.attachment_drafts.borrow_mut().clear();
    let done = Rc::new(RefCell::new(false));
    let finished = done.clone();
    glib::idle_add_local_once(move || {
        *finished.borrow_mut() = true;
    });
    wait_for(|| *done.borrow());
    assert!(state.attachment_drafts.borrow().is_empty());
    assert_eq!(cache_files(), before_cancelled_paste);
    state
        .composer
        .clipboard()
        .set_content(None::<&gdk::ContentProvider>)
        .unwrap();
    state.window.close();
}

#[test]
#[ignore = "requires a native GTK display"]
fn gtk_attachment_high_resolution_preview_stays_bounded() {
    libadwaita::init().unwrap();
    let temp = std::env::temp_dir().join(format!("line-attachment-layout-{}", std::process::id()));
    std::fs::create_dir_all(temp.join("protocol")).unwrap();
    let fake = temp.join("protocol/line-gtk-protocol");
    std::fs::write(
        &fake,
        "#!/usr/bin/python3\nimport sys\nfor line in sys.stdin: pass\n",
    )
    .unwrap();
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
        .application_id("dev.linegtk.AttachmentLayoutFixture")
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
    state.window.set_default_size(900, 650);
    state.stack.set_visible_child_name("shell");
    *state.current_chat.borrow_mut() = Some("layout".into());
    for (name, width, height) in [("portrait.png", 1200, 1600), ("landscape.png", 2400, 800)] {
        let image = temp.join(name);
        let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height).unwrap();
        pixbuf.fill(0x224488ff);
        pixbuf.savev(&image, "png", &[]).unwrap();
        assert!(stage_attachment(&state, "layout", image));
        let snapshot = state.attachment_drafts.borrow()["layout"]
            .last()
            .unwrap()
            .path
            .clone();
        let original = Pixbuf::from_file(snapshot).unwrap();
        assert_eq!((original.width(), original.height()), (width, height));
    }
    {
        let mut drafts = state.attachment_drafts.borrow_mut();
        drafts.get_mut("layout").unwrap()[0].name = "Long filename ".repeat(40);
        drafts.get_mut("layout").unwrap()[0].status = DraftStatus::Failed;
    }
    refresh_attachment_drafts(&state);
    wait_for(|| state.attachment_strip.height() > 0);
    let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("Mapped tray height {}", state.attachment_scroll.height());
    assert!(
        state.attachment_scroll.height() <= 220,
        "tray height {}",
        state.attachment_scroll.height()
    );
    let mut child = state.attachment_strip.first_child();
    while let Some(card) = child {
        child = card.next_sibling();
        eprintln!("Mapped card {}x{}", card.width(), card.height());
        assert!(card.width() <= 220, "card width {}", card.width());
        assert!(card.height() <= 220, "card height {}", card.height());
    }
    state.window.close();
}
