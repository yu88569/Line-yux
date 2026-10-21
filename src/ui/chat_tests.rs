//! GTK interaction coverage uses an isolated application and a protocol process
//! that exits immediately: no real account, credentials, or network is involved.
use super::*;

fn drain() {
    while glib::MainContext::default().pending() {
        glib::MainContext::default().iteration(false);
    }
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !condition() && std::time::Instant::now() < deadline {
        drain();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(condition(), "GTK fixture reached its expected layout state");
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut all = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(w) = child {
        all.extend(descendants(&w));
        child = w.next_sibling();
    }
    all
}

fn chat(mid: &str, activity: i64) -> ChatInfo {
    serde_json::from_value(
        serde_json::json!({"mid":mid,"name":mid,"lastActivity":activity,"unread":7}),
    )
    .unwrap()
}

fn row(state: &AppState, mid: &str) -> gtk::ListBoxRow {
    let index = state
        .chats
        .borrow()
        .iter()
        .position(|c| c.mid == mid)
        .unwrap();
    state.chat_list.row_at_index(index as i32).unwrap()
}

fn activate_visible_item(state: &AppState, mid: &str, item: usize) {
    let target = row(state, mid);
    wait_for(|| target.is_mapped() && target.height() > 0);
    let point = target
        .compute_point(&state.chat_list, &gtk::graphene::Point::new(12.0, 12.0))
        .unwrap();
    let gesture = state
        .chat_list
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|c| {
            c.downcast::<gtk::GestureClick>()
                .ok()
                .filter(|c| c.button() == 3)
        })
        .unwrap();
    gesture.emit_by_name::<()>(
        "pressed",
        &[&1i32, &(point.x() as f64), &(point.y() as f64)],
    );
    drain();
    let popover = descendants(state.chat_list.upcast_ref())
        .into_iter()
        .find_map(|w| {
            w.downcast::<gtk::PopoverMenu>()
                .ok()
                .filter(|p| p.is_visible())
        })
        .expect("right-click opens visible native menu");
    let buttons: Vec<_> = descendants(popover.upcast_ref())
        .into_iter()
        .filter(|w| w.type_().name() == "GtkModelButton")
        .collect();
    assert_eq!(buttons.len(), 3);
    assert!(buttons[item].is_mapped());
    assert!(buttons[item].is_sensitive());
    assert!(
        buttons[item].activate(),
        "activate the visible GTK menu item"
    );
    drain();
    assert!(!popover.is_visible());
    assert!(popover.parent().is_none());
}

fn snapshot_fixture(state: &AppState, compact: bool) {
    let frame = Rc::new(std::cell::Cell::new(false));
    let ready = frame.clone();
    state.window.add_tick_callback(move |_, _| {
        ready.set(true);
        glib::ControlFlow::Break
    });
    wait_for(|| frame.get());
    let paintable = gtk::WidgetPaintable::new(Some(&state.sidebar));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        state.sidebar.width() as f64,
        state.sidebar.height() as f64,
    );
    let node = snapshot
        .to_node()
        .expect("fixture has a rendered GTK scene");
    state
        .window
        .renderer()
        .unwrap()
        .render_texture(
            &node,
            Some(&gtk::graphene::Rect::new(
                0.0,
                0.0,
                state.sidebar.width() as f32,
                260.0,
            )),
        )
        .save_to_png(format!(
            "/tmp/line-gtk-chat-menu-{}.png",
            if compact { "compact" } else { "regular" }
        ))
        .unwrap();
}

fn status_icon(state: &AppState, mid: &str, name: &str, visible: bool) {
    let icon = descendants(row(state, mid).upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Image>().ok())
        .find(|i| i.icon_name().as_deref() == Some(name))
        .expect("native row status icon exists");
    assert_eq!(icon.is_visible(), visible, "{mid}: {name}");
    if visible {
        assert!(
            icon.is_mapped(),
            "status stays mapped in compact and regular rows"
        );
        let key = if name == "view-pin-symbolic" {
            "chat_pinned_status"
        } else {
            "chat_muted_status"
        };
        assert_eq!(
            icon.tooltip_text().as_deref(),
            Some(crate::i18n::t(key).as_str())
        );
    }
}

fn chat_fixture(name: &str, application_id: &str) -> (PathBuf, PathBuf, Application, AppState) {
    libadwaita::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    let temp = std::env::temp_dir().join(format!("line-gtk-{name}-{}", std::process::id()));
    std::fs::create_dir_all(temp.join("protocol")).unwrap();
    let fake_protocol = temp.join("protocol/line-gtk-protocol");
    std::fs::write(&fake_protocol, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake_protocol, std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = temp.join("data");
    let config = AppConfig {
        animations: false,
        tray_enabled: false,
        close_to_tray: false,
        notifications: false,
        auto_mark_read: false,
        language: "en".into(),
        ..AppConfig::default()
    };
    config.save(&data);
    let app = Application::builder()
        .application_id(application_id)
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
    (temp, data, app, state)
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_chat_updates_preserve_typing_focus_and_draft() {
    let (temp, _, _app, state) = chat_fixture("typing-focus", "dev.linegtk.TypingFocusFixture");
    *state.restored_last_chat.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.window.set_default_size(1000, 700);
    apply_chats(
        &state,
        vec![chat("u-selected", 100), chat("c-other", 10)],
        false,
    );
    state.current_chat.borrow_mut().replace("u-selected".into());
    select_sidebar_chat(&state, Some("u-selected"));
    state.window.present();
    wait_for(|| state.composer.is_mapped());
    state.composer.set_text("draft message");
    assert!(state.composer.grab_focus());
    drain();
    state.composer.set_position(5);
    assert_eq!(state.composer.position(), 5);
    let typing_focus = gtk::prelude::GtkWindowExt::focus(&state.window)
        .expect("composer has a focused text widget");
    assert!(typing_focus.is_ancestor(&state.composer));

    let incoming: MessageInfo = serde_json::from_value(serde_json::json!({
        "id": "focus-message", "from": "u-member", "to": "c-other",
        "mine": false, "text": "new notification", "createdTime": 200,
    }))
    .unwrap();
    handle_event(&state, ProtocolEvent::Message(incoming));
    drain();
    assert_eq!(state.chats.borrow()[0].mid, "c-other");
    assert_eq!(state.current_chat.borrow().as_deref(), Some("u-selected"));
    assert_eq!(
        state.chat_list.selected_row(),
        Some(row(&state, "u-selected"))
    );
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&state.window),
        Some(typing_focus.clone()),
        "incoming message must preserve composer focus"
    );
    assert_eq!(state.composer.text(), "draft message");
    assert_eq!(state.composer.position(), 5);

    let refreshed = state.chats.borrow().clone();
    apply_chats(&state, refreshed, true);
    drain();
    assert_eq!(
        state.chat_list.selected_row(),
        Some(row(&state, "u-selected"))
    );
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&state.window),
        Some(typing_focus),
        "chat list refresh must preserve composer focus"
    );
    assert_eq!(state.composer.text(), "draft message");
    assert_eq!(state.composer.position(), 5);

    state.window.destroy();
    state.discord.shutdown();
    drain();
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_chat_upserts_preserve_resolved_names_and_accept_renames() {
    let (temp, _, _app, state) = chat_fixture("name-fixture", "dev.linegtk.ChatNameFixture");
    *state.restored_last_chat.borrow_mut() = true;
    for compact in [false, true] {
        *state.sidebar_compact.borrow_mut() = compact;
        apply_chats(&state, vec![chat("c-target", 10)], false);
        let mut named_group = chat("c-target", 10);
        named_group.name = "กลุ่มเพื่อน".into();
        upsert_chat_row(&state, named_group);
        for (incoming_name, expected_name) in [
            ("c-target", "กลุ่มเพื่อน"),
            ("กลุ่มชื่อใหม่", "กลุ่มชื่อใหม่"),
            ("", "กลุ่มชื่อใหม่"),
        ] {
            let mut incoming = chat("c-target", 10);
            incoming.name = incoming_name.into();
            upsert_chat_row(&state, incoming);
            let target = row(&state, "c-target");
            let label = descendants(target.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .find(|label| label.has_css_class("line-chat-name"))
                .unwrap();
            assert_eq!(label.text().as_str(), expected_name);
            assert_eq!(target.tooltip_text().as_deref(), Some(expected_name));
            assert_eq!(
                state
                    .chats
                    .borrow()
                    .iter()
                    .find(|c| c.mid == "c-target")
                    .unwrap()
                    .name,
                expected_name
            );
        }
    }
    state.window.destroy();
    state.discord.shutdown();
    drain();
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_menu_actions_mutate_real_state_and_show_status_icons() {
    let (temp, data, app, state) =
        chat_fixture("menu-interaction", "dev.linegtk.ChatInteractionFixture");
    wire_actions(&state);
    *state.session_ready.borrow_mut() = true;
    *state.restored_last_chat.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.window.set_default_size(1000, 700);
    apply_chats(
        &state,
        vec![chat("u-selected", 100), chat("c-target", 10)],
        false,
    );
    let incoming: MessageInfo = serde_json::from_value(serde_json::json!({
        "id": "group-message", "from": "u-member", "to": "c-target",
        "mine": false, "text": "group hello", "createdTime": 200,
    }))
    .unwrap();
    handle_event(&state, ProtocolEvent::Message(incoming));
    assert_eq!(
        state.chats.borrow()[0].mid,
        "c-target",
        "incoming group message promotes the group instead of the member"
    );
    assert_eq!(state.chats.borrow().len(), 2, "no member DM is created");
    assert_eq!(state.chats.borrow()[0].unread, 8);
    assert!(
        state.chat_previews.borrow()["c-target"]
            .text()
            .contains("group hello")
    );
    assert_eq!(state.chats.borrow()[1].unread, 7);
    state.current_chat.borrow_mut().replace("c-target".into());
    for (id, from, mine, text) in [
        ("group-message-open", "u-member", false, "open group hello"),
        ("group-message-own", "u-self", true, "own group hello"),
    ] {
        let message: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": id, "from": from, "to": "c-target", "mine": mine,
            "text": text, "createdTime": 201,
        }))
        .unwrap();
        handle_event(&state, ProtocolEvent::Message(message));
        assert!(state.seen_msg_ids.borrow().contains(id));
        let model = state.message_list.widget().model().unwrap();
        assert!(
            (0..model.n_items()).any(|index| {
                let item = model
                    .item(index)
                    .and_downcast::<glib::BoxedAnyObject>()
                    .unwrap();
                descendants(&item.borrow::<gtk::Widget>())
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                    .any(|label| label.text() == text)
            }),
            "open group renders the incoming or own message text"
        );
        assert_eq!(state.chats.borrow()[0].mid, "c-target");
        assert_eq!(
            state.chats.borrow().len(),
            2,
            "no member or own DM is created"
        );
        assert!(
            state.chat_previews.borrow()["c-target"]
                .text()
                .contains(text)
        );
    }
    state.current_chat.borrow_mut().take();
    let room_message: MessageInfo = serde_json::from_value(serde_json::json!({
        "id": "room-message", "from": "u-member", "to": "r-room",
        "mine": false, "text": "room hello", "createdTime": 202,
    }))
    .unwrap();
    handle_event(&state, ProtocolEvent::Message(room_message));
    assert_eq!(state.chats.borrow()[0].mid, "r-room");
    assert_eq!(state.chats.borrow()[0].kind, "group");
    assert_eq!(state.chats.borrow()[0].unread, 1);
    assert_eq!(state.chats.borrow().len(), 3);
    let activation_count = Rc::new(std::cell::Cell::new(0));
    let count = activation_count.clone();
    state
        .chat_list
        .connect_row_activated(move |_, _| count.set(count.get() + 1));

    for compact in [false, true] {
        state.config.borrow_mut().language = if compact { "th" } else { "en" }.into();
        apply_ui_language(&state);
        let width = if compact { 80 } else { 320 };
        state.sidebar_paned.set_position(width);
        apply_sidebar_compact(&state, width);
        apply_chats(
            &state,
            vec![chat("u-selected", 100), chat("c-target", 10)],
            false,
        );
        state.current_chat.borrow_mut().replace("u-selected".into());
        select_sidebar_chat(&state, Some("u-selected"));
        drain();

        activate_visible_item(&state, "c-target", 1);
        assert!(state.config.borrow().chat_muted("c-target", false));
        assert_eq!(state.current_chat.borrow().as_deref(), Some("u-selected"));
        status_icon(&state, "c-target", "notifications-disabled-symbolic", true);
        activate_visible_item(&state, "c-target", 2);
        assert!(state.config.borrow().chat_pinned("c-target"));
        assert_eq!(state.chats.borrow()[0].mid, "c-target");
        assert_eq!(
            state.chat_list.selected_row(),
            Some(row(&state, "u-selected"))
        );
        status_icon(&state, "c-target", "view-pin-symbolic", true);
        status_icon(&state, "c-target", "notifications-disabled-symbolic", true);
        upsert_chat_row(&state, chat("c-target", 10));
        promote_chat_to_top(&state, "u-selected");
        assert_eq!(state.chats.borrow()[0].mid, "c-target");
        let refreshed = state.chats.borrow().clone();
        apply_chats(&state, refreshed, true);
        status_icon(&state, "c-target", "view-pin-symbolic", true);
        status_icon(&state, "c-target", "notifications-disabled-symbolic", true);
        snapshot_fixture(&state, compact);
        let restarted = AppConfig::load(&data);
        assert!(restarted.chat_pinned("c-target"));
        assert!(restarted.chat_muted("c-target", false));

        activate_visible_item(&state, "c-target", 1);
        assert!(!state.config.borrow().chat_muted("c-target", true));
        status_icon(&state, "c-target", "notifications-disabled-symbolic", false);
        activate_visible_item(&state, "c-target", 2);
        assert!(!state.config.borrow().chat_pinned("c-target"));
        assert_eq!(state.chats.borrow()[0].mid, "u-selected");
        status_icon(&state, "c-target", "view-pin-symbolic", false);

        state.current_chat.borrow_mut().replace("c-target".into());
        select_sidebar_chat(&state, Some("c-target"));
        state.mute_btn.emit_clicked();
        assert!(state.config.borrow().chat_muted("c-target", false));
        status_icon(&state, "c-target", "notifications-disabled-symbolic", true);
        state.mute_btn.emit_clicked();
        status_icon(&state, "c-target", "notifications-disabled-symbolic", false);

        // No override on this DM: server state must update the same row indicator.
        handle_event(
            &state,
            ProtocolEvent::ChatMute {
                mid: "u-selected".into(),
                muted: true,
            },
        );
        status_icon(
            &state,
            "u-selected",
            "notifications-disabled-symbolic",
            true,
        );
        handle_event(
            &state,
            ProtocolEvent::ChatMute {
                mid: "u-selected".into(),
                muted: false,
            },
        );
        status_icon(
            &state,
            "u-selected",
            "notifications-disabled-symbolic",
            false,
        );
        activate_visible_item(&state, "c-target", 0);
        assert!(state.config.borrow().chat_hidden("c-target"));
        assert!(!row(&state, "c-target").is_visible());
        assert!(state.current_chat.borrow().is_none());
        ensure_chat_visible(&state, &chat("c-target", 200));
        promote_chat_to_top(&state, "c-target");
        assert!(!row(&state, "c-target").is_visible());
        let refreshed = state.chats.borrow().clone();
        apply_chats(&state, refreshed, true);
        assert!(!row(&state, "c-target").is_visible());
        set_chat_hidden(&state, "c-target", false);
        drain();
        assert!(row(&state, "c-target").is_visible());
        assert!(!AppConfig::load(&data).chat_hidden("c-target"));
        assert!(!state.config.borrow().chat_muted("c-target", true));
        // Reopen and reverse both context actions with the target selected.
        state.current_chat.borrow_mut().replace("c-target".into());
        select_sidebar_chat(&state, Some("c-target"));
        activate_visible_item(&state, "c-target", 1);
        activate_visible_item(&state, "c-target", 2);
        assert_eq!(state.current_chat.borrow().as_deref(), Some("c-target"));
        assert_eq!(
            state.chat_list.selected_row(),
            Some(row(&state, "c-target"))
        );
        status_icon(&state, "c-target", "view-pin-symbolic", true);
        status_icon(&state, "c-target", "notifications-disabled-symbolic", true);
        activate_visible_item(&state, "c-target", 1);
        activate_visible_item(&state, "c-target", 2);
        handle_event(
            &state,
            ProtocolEvent::ChatMute {
                mid: "c-target".into(),
                muted: true,
            },
        );
        status_icon(&state, "c-target", "notifications-disabled-symbolic", false);
        let refreshed = state.chats.borrow().clone();
        apply_chats(&state, refreshed, true);
        status_icon(&state, "c-target", "notifications-disabled-symbolic", false);
        state.current_chat.borrow_mut().replace("u-selected".into());
        select_sidebar_chat(&state, Some("u-selected"));
        activate_visible_item(&state, "c-target", 0);
        assert_eq!(state.current_chat.borrow().as_deref(), Some("u-selected"));
        assert!(!row(&state, "c-target").is_visible());
        set_chat_hidden(&state, "c-target", false);
        state.config.borrow_mut().chat_preferences.clear();
    }
    assert_eq!(
        activation_count.get(),
        0,
        "right-click never opens the target chat"
    );
    state.window.destroy();
    state.discord.shutdown();
    for name in ["hide-chat", "mute-chat", "pin-chat"] {
        app.remove_action(name);
    }
    drain();
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_incoming_sender_avatars_cache_late_updates_and_cleanup() {
    libadwaita::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    let temp = std::env::temp_dir().join(format!("line-gtk-avatar-fixture-{}", std::process::id()));
    std::fs::create_dir_all(temp.join("protocol")).unwrap();
    let fake_protocol = temp.join("protocol/line-gtk-protocol");
    std::fs::write(&fake_protocol, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake_protocol, std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = temp.join("data");
    let config = AppConfig {
        animations: false,
        tray_enabled: false,
        close_to_tray: false,
        notifications: false,
        auto_mark_read: false,
        language: "en".into(),
        ..AppConfig::default()
    };
    config.save(&data);
    let app = Application::builder()
        .application_id("dev.linegtk.MessageAvatarFixture")
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

    *state.session_ready.borrow_mut() = true;
    *state.restored_last_chat.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state.window.set_default_size(820, 650);
    state.chat_title.set_text("Avatar fixture");
    *state.current_chat.borrow_mut() = Some("c-fixture".into());
    let first = format!("u{}", "1".repeat(32));
    let second = format!("u{}", "2".repeat(32));
    let third = format!("u{}", "3".repeat(32));
    let cache = data.join("cache/avatars");
    std::fs::create_dir_all(&cache).unwrap();
    let first_path = cache.join(format!("{first}.jpg"));
    let second_path = cache.join(format!("{second}.jpg"));
    image::RgbImage::from_pixel(80, 80, image::Rgb([219, 94, 67]))
        .save(&first_path)
        .unwrap();
    image::RgbImage::from_pixel(80, 80, image::Rgb([44, 139, 164]))
        .save(&second_path)
        .unwrap();
    let message = |id: &str, sender: &str, mine: bool, text: &str| -> MessageInfo {
        serde_json::from_value(
            serde_json::json!({"id":id,"from":sender,"to":"c-fixture","mine":mine,"text":text}),
        )
        .unwrap()
    };
    append_message(
        &state,
        &message("first", &first, false, "Hello from the first sender"),
        false,
    );
    append_message(
        &state,
        &message("second", &second, false, "Hello from a different sender"),
        false,
    );
    append_message(
        &state,
        &message("mine", &first, true, "My reply has no avatar"),
        false,
    );
    append_message(
        &state,
        &message(
            "late-one",
            &third,
            false,
            "A photo arriving after history loads",
        ),
        false,
    );
    append_message(
        &state,
        &message("late-two", &third, false, "The same sender in another row"),
        false,
    );
    let row_at = |index| {
        let item = state
            .message_list
            .widget()
            .model()
            .unwrap()
            .item(index)
            .unwrap()
            .downcast::<glib::BoxedAnyObject>()
            .unwrap();
        item.borrow::<gtk::Widget>().clone()
    };
    let avatar_at = |index| -> Option<gtk::Picture> {
        descendants(&row_at(index))
            .into_iter()
            .find(|w| w.has_css_class("line-message-avatar"))
            .and_then(|w| {
                descendants(&w)
                    .into_iter()
                    .find_map(|w| w.downcast::<gtk::Picture>().ok())
            })
    };
    let a = avatar_at(0).unwrap();
    let b = avatar_at(1).unwrap();
    assert!(
        avatar_at(2).is_none(),
        "outgoing rows keep their original layout"
    );
    let late_one = avatar_at(3).unwrap();
    let late_two = avatar_at(4).unwrap();
    wait_for(|| a.paintable().is_some() && b.paintable().is_some());
    let pixels = |picture: &gtk::Picture| {
        let texture = picture
            .paintable()
            .unwrap()
            .downcast::<gdk::Texture>()
            .unwrap();
        let mut bytes = vec![0; texture.width() as usize * texture.height() as usize * 4];
        texture.download(&mut bytes, texture.width() as usize * 4);
        bytes
    };
    assert_ne!(
        pixels(&a),
        pixels(&b),
        "different senders display their own cached photos"
    );
    assert!(late_one.paintable().is_none());
    handle_event(
        &state,
        ProtocolEvent::AvatarReady {
            mid: third.clone(),
            avatar_path: second_path.to_string_lossy().into_owned(),
        },
    );
    wait_for(|| late_one.paintable().is_some() && late_two.paintable().is_some());
    assert_eq!(pixels(&late_one), pixels(&b));
    assert_eq!(pixels(&late_two), pixels(&b));
    state.window.present();
    wait_for(|| a.is_mapped() && state.conversation.width() > 100);
    let frame = Rc::new(std::cell::Cell::new(false));
    let ready = frame.clone();
    state.window.add_tick_callback(move |_, _| {
        ready.set(true);
        glib::ControlFlow::Break
    });
    wait_for(|| frame.get());
    let paintable = gtk::WidgetPaintable::new(Some(&state.conversation));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        state.conversation.width() as f64,
        state.conversation.height() as f64,
    );
    let node = snapshot.to_node().unwrap();
    state
        .window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png("/tmp/line-gtk-message-avatars.png")
        .unwrap();
    append_message(
        &state,
        &message(
            "long-incoming",
            &first,
            false,
            "A longer incoming message should wrap within this narrow conversation while its sender profile picture stays aligned with the top of the bubble.",
        ),
        false,
    );
    state.sidebar_paned.set_end_child(None::<&gtk::Widget>);
    let narrow = gtk::Window::builder()
        .application(&app)
        .transient_for(&state.window)
        .modal(true)
        .default_width(600)
        .default_height(720)
        .title("Avatar narrow fixture")
        .build();
    narrow.set_child(Some(&state.conversation));
    narrow.present();
    wait_for(|| state.conversation.width() >= 540 && state.conversation.width() <= 650);
    let long_row = row_at(5);
    let avatar_frame = descendants(&long_row)
        .into_iter()
        .find(|w| w.has_css_class("line-message-avatar"))
        .unwrap();
    let bubble = descendants(&long_row)
        .into_iter()
        .find(|w| w.has_css_class("line-bubble-in"))
        .unwrap();
    wait_for(|| long_row.is_mapped() && bubble.height() > 40);
    let avatar_bounds = avatar_frame.compute_bounds(&long_row).unwrap();
    let bubble_bounds = bubble.compute_bounds(&long_row).unwrap();
    assert!(
        (avatar_bounds.y() - bubble_bounds.y()).abs() < 1.0,
        "avatar stays top aligned beside wrapped text: avatar={avatar_bounds:?}, bubble={bubble_bounds:?}"
    );
    assert!(
        bubble_bounds.x() + bubble_bounds.width() <= state.conversation.width() as f32,
        "wrapped bubble fits narrow conversation"
    );
    let frame = Rc::new(std::cell::Cell::new(false));
    let ready = frame.clone();
    narrow.add_tick_callback(move |_, _| {
        ready.set(true);
        glib::ControlFlow::Break
    });
    wait_for(|| frame.get());
    let paintable = gtk::WidgetPaintable::new(Some(&state.conversation));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        state.conversation.width() as f64,
        state.conversation.height() as f64,
    );
    let node = snapshot.to_node().unwrap();
    narrow
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png("/tmp/line-gtk-message-avatars-narrow.png")
        .unwrap();
    narrow.set_child(None::<&gtk::Widget>);
    narrow.destroy();
    drop(narrow);
    state.sidebar_paned.set_end_child(Some(&state.conversation));
    drop(long_row);
    drop(avatar_frame);
    drop(bubble);
    let old_weak = a.downgrade();
    drop(a);
    clear_messages(&state);
    drain();
    wait_for(|| old_weak.upgrade().is_none());
    *state.current_chat.borrow_mut() = Some("c-other".into());
    append_message(
        &state,
        &message("new-chat", &second, false, "New conversation"),
        false,
    );
    let fresh = avatar_at(0).unwrap();
    wait_for(|| fresh.paintable().is_some());
    let before = pixels(&fresh);
    handle_event(
        &state,
        ProtocolEvent::AvatarReady {
            mid: first,
            avatar_path: second_path.to_string_lossy().into_owned(),
        },
    );
    drain();
    assert_eq!(
        pixels(&fresh),
        before,
        "old sender event cannot change another chat sender"
    );
    append_message(
        &state,
        &message("remembered", &third, false, "Remembered sender avatar"),
        false,
    );
    let remembered = avatar_at(1).unwrap();
    wait_for(|| remembered.paintable().is_some());
    assert_eq!(
        pixels(&remembered),
        before,
        "ready paths survive row rebuilds"
    );
    append_message(
        &state,
        &message(
            "invalid-sender",
            first_path.with_extension("").to_str().unwrap(),
            false,
            "Invalid sender keeps fallback",
        ),
        false,
    );
    let invalid = avatar_at(2).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    while std::time::Instant::now() < deadline {
        drain();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        invalid.paintable().is_none(),
        "untrusted sender paths cannot be loaded as avatars"
    );
    state.window.close();
    clear_messages(&state);
    drain();
    let _ = std::fs::remove_dir_all(temp);
}
