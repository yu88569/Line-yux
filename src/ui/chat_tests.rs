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
    handle_event(&state, ProtocolEvent::Message(Box::new(incoming)));
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
    handle_event(&state, ProtocolEvent::Message(Box::new(incoming)));
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
        handle_event(&state, ProtocolEvent::Message(Box::new(message)));
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
    handle_event(&state, ProtocolEvent::Message(Box::new(room_message)));
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

#[test]
#[ignore = "requires GTK and an X11 display with xdotool"]
fn gtk_message_reply_right_click_and_lifecycle() {
    let (temp, _, _app, state) = chat_fixture("message-reply", "dev.linegtk.ReplyFixture");
    let log = temp.join("requests.jsonl");
    std::fs::write(
        temp.join("protocol/line-gtk-protocol"),
        format!(
            "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; done\n",
            log.display()
        ),
    )
    .unwrap();
    state.sidecar.restart().unwrap();
    *state.session_ready.borrow_mut() = true;
    let peer = "u11111111111111111111111111111111";
    *state.current_chat.borrow_mut() = Some(peer.into());
    let mut person = chat(peer, 1);
    person.name = "Alice".into();
    state.chats.borrow_mut().push(person);
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state
        .window
        .set_title(Some("LINE reply interaction fixture"));
    state.window.present();
    let original: MessageInfo = serde_json::from_value(serde_json::json!({"id":"123456789","text":"Original selectable text","from":peer,"to":"u22222222222222222222222222222222","createdTime":1})).unwrap();
    append_message(&state, &original, false);
    wait_for(|| {
        descendants(state.window.upcast_ref())
            .iter()
            .any(|w| w.has_css_class("line-bubble-text") && w.is_mapped())
    });
    let label = descendants(state.window.upcast_ref())
        .into_iter()
        .find(|w| w.has_css_class("line-bubble-text"))
        .unwrap();
    assert!(
        label
            .clone()
            .downcast::<gtk::Label>()
            .unwrap()
            .is_selectable()
    );
    let bubble = label.parent().unwrap();
    let menu = descendants(&bubble)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Popover>().ok())
        .unwrap();
    let controllers = bubble.observe_controllers();
    assert!((0..controllers.n_items()).any(|i| {
        controllers
            .item(i)
            .and_then(|c| c.downcast::<gtk::GestureClick>().ok())
            .is_some_and(|c| {
                c.button() == 3 && c.propagation_phase() == gtk::PropagationPhase::Capture
            })
    }));
    unsafe extern "C" {
        fn gdk_x11_surface_get_xid(surface: *mut gdk::ffi::GdkSurface) -> std::os::raw::c_ulong;
    }
    use glib::translate::ToGlibPtr;
    let surface = state.window.surface().unwrap();
    let window = unsafe { gdk_x11_surface_get_xid(surface.to_glib_none().0) }.to_string();
    gdk::Display::default().unwrap().flush();
    wait_for(|| label.width() > 0 && label.height() > 0);
    let point = label
        .compute_point(&state.window, &gtk::graphene::Point::new(10.0, 10.0))
        .unwrap();
    std::process::Command::new("xdotool")
        .args(["windowmap", &window])
        .status()
        .unwrap();
    std::process::Command::new("xdotool")
        .args(["windowraise", &window])
        .status()
        .unwrap();
    assert!(
        std::process::Command::new("xdotool")
            .args([
                "mousemove",
                "--window",
                &window,
                &(point.x() as i32).to_string(),
                &(point.y() as i32).to_string(),
                "click",
                "3"
            ])
            .status()
            .unwrap()
            .success()
    );
    wait_for(|| menu.is_visible());
    let actions: Vec<_> = descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .collect();
    assert!(
        actions
            .iter()
            .any(|b| b.label().as_deref() == Some(&crate::i18n::t("qr_copy")))
    );
    actions
        .iter()
        .find(|b| b.label().as_deref() == Some("❤️"))
        .unwrap()
        .emit_clicked();
    wait_for(|| {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("react_message")
    });
    let reaction_request: serde_json::Value = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|r| r["method"] == "react_message")
        .unwrap();
    assert_eq!(reaction_request["params"]["messageId"], original.id);
    assert_eq!(reaction_request["params"]["reaction"], 3);
    actions
        .iter()
        .find(|b| b.label().as_deref() == Some(&crate::i18n::t("reply")))
        .unwrap()
        .emit_clicked();
    assert_eq!(
        state.reply_target.borrow().as_ref().unwrap().id,
        original.id
    );
    assert!(state.reply_label.text().contains("Alice"));
    state
        .reply_strip
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert!(state.reply_target.borrow().is_none());
    set_reply(&state, reply_target(&state, &original));
    state.composer.set_text("A real reply");
    send_current(&state);
    assert!(state.reply_target.borrow().is_none());
    wait_for(|| {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("A real reply")
    });
    let request: serde_json::Value = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|r| r["method"] == "send_message")
        .unwrap();
    assert_eq!(request["params"]["reply"]["id"], original.id);
    let id = request["id"].as_u64().unwrap();
    handle_event(
        &state,
        ProtocolEvent::Response {
            id,
            ok: false,
            result: serde_json::Value::Null,
            error: Some("fixture failed".into()),
        },
    );
    assert_eq!(state.composer.text(), "A real reply");
    assert_eq!(
        state.reply_target.borrow().as_ref().unwrap().id,
        original.id
    );
    let audio: MessageInfo = serde_json::from_value(serde_json::json!({"id":"234567890","text":"Voice message","from":peer,"to":"u22222222222222222222222222222222","contentType":"AUDIO","reply":{"chatMid":peer,"id":original.id,"sender":"Alice","preview":"Original selectable text"}})).unwrap();
    append_message(&state, &audio, false);
    let audio_bubble = state.media_slots.borrow().get(&audio.id).unwrap().clone();
    attach_audio_to_slot(&state, &audio.id, "/tmp/line-reply-fixture-unavailable.m4a");
    let hydrated = descendants(audio_bubble.upcast_ref());
    assert!(hydrated.iter().any(|w| w.is::<gtk::Popover>()));
    assert!(hydrated.iter().any(|w| w.has_css_class("line-reply-quote")));
    assert!(
        reply_target(
            &state,
            &serde_json::from_value::<MessageInfo>(
                serde_json::json!({"id":"pending-1","from":peer})
            )
            .unwrap()
        )
        .is_none()
    );
    let other = chat("u33333333333333333333333333333333", 2);
    open_chat(&state, &other);
    assert!(state.reply_target.borrow().is_none());
    set_reply(&state, reply_target(&state, &original));
    assert!(
        state.reply_target.borrow().is_none(),
        "cross-chat targets are rejected"
    );
    state.window.close();
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
#[ignore = "requires GTK display"]
fn gtk_location_receive_send_and_failure() {
    let (temp, _, app, state) = chat_fixture("location", "dev.linegtk.LocationFixture");
    let log = temp.join("requests.jsonl");
    std::fs::write(
        temp.join("protocol/line-gtk-protocol"),
        format!(
            "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; done\n",
            log.display()
        ),
    )
    .unwrap();
    state.sidecar.restart().unwrap();
    *state.session_ready.borrow_mut() = true;
    let peer = "u11111111111111111111111111111111";
    *state.current_chat.borrow_mut() = Some(peer.into());
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state.window.present();
    let incoming: MessageInfo = serde_json::from_value(serde_json::json!({"id":"location-1","from":peer,"to":"u22222222222222222222222222222222","contentType":"LOCATION","location":{"title":"Bangkok","address":"Thailand","latitude":13.7563,"longitude":100.5018}})).unwrap();
    append_message(&state, &incoming, false);
    wait_for(|| {
        descendants(state.window.upcast_ref())
            .iter()
            .any(|w| w.is::<gtk::LinkButton>() && w.is_mapped())
    });
    let link = descendants(state.window.upcast_ref())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::LinkButton>().ok())
        .unwrap();
    assert_eq!(
        link.uri(),
        "https://www.google.com/maps/search/?api=1&query=13.7563%2C100.5018"
    );
    let opened = Rc::new(RefCell::new(false));
    link.connect_activate_link({
        let opened = opened.clone();
        move |_| {
            *opened.borrow_mut() = true;
            glib::Propagation::Stop
        }
    });
    link.emit_by_name::<bool>("activate-link", &[]);
    assert!(*opened.borrow());
    wire_actions(&state);
    *state.reply_target.borrow_mut() = Some(crate::protocol::ReplyTarget {
        chat_mid: peer.into(),
        id: "123".into(),
        sender: "Alice".into(),
        preview: "Reply".into(),
    });
    state.location_btn.emit_clicked();
    drain();
    assert_eq!(app.windows().len(), 1);
    clear_reply(&state);
    state.location_btn.emit_clicked();
    drain();
    let window = app
        .windows()
        .into_iter()
        .find(|w| w.as_ptr() != state.window.as_ptr().cast())
        .unwrap();
    wait_for(|| window.is_mapped() && window.width() > 0);
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let frame = Rc::new(std::cell::Cell::new(false));
    let ready = frame.clone();
    window.add_tick_callback(move |_, _| {
        ready.set(true);
        glib::ControlFlow::Break
    });
    wait_for(|| frame.get());
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    if let Some(node) = snapshot.to_node() {
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png("/tmp/line-gtk-location-dialog.png")
            .unwrap();
    }
    let entries: Vec<gtk::Entry> = descendants(window.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect();
    assert_eq!(entries.len(), 3);
    entries[0].set_text("0, -120.123456789");
    entries[1].set_text("Test place");
    entries[2].set_text("Test address");
    let send = descendants(window.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.has_css_class("suggested-action"))
        .unwrap();
    *state.current_chat.borrow_mut() = Some("c33333333333333333333333333333333".into());
    *state.reply_target.borrow_mut() = Some(crate::protocol::ReplyTarget {
        chat_mid: peer.into(),
        id: "123".into(),
        sender: "Alice".into(),
        preview: "Reply".into(),
    });
    send.emit_clicked();
    drain();
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("send_location")
    );
    clear_reply(&state);
    send.emit_clicked();
    wait_for(|| {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("send_location")
    });
    let requests: Vec<serde_json::Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    let request = requests
        .iter()
        .find(|r| r["method"] == "send_location")
        .unwrap();
    assert_eq!(request["params"]["chatMid"], peer);
    assert_eq!(request["params"]["location"]["longitude"], -120.123456789);
    assert!(!send.is_sensitive());
    let id = request["id"].as_u64().unwrap();
    handle_event(
        &state,
        ProtocolEvent::Response {
            id,
            ok: false,
            result: serde_json::Value::Null,
            error: Some("Test failure".into()),
        },
    );
    assert!(send.is_sensitive());
    assert_eq!(entries[0].text(), "0, -120.123456789");
    let previous = std::fs::read_to_string(&log).unwrap();
    entries[0].set_text("NaN,0");
    send.emit_clicked();
    drain();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), previous);
    entries[0].set_text("0, -120.123456789");
    send.emit_clicked();
    wait_for(|| {
        state
            .pending
            .borrow()
            .values()
            .any(|p| matches!(p, Pending::LocationSend { .. }))
    });
    let id = *state
        .pending
        .borrow()
        .iter()
        .find(|(_, p)| matches!(p, Pending::LocationSend { .. }))
        .unwrap()
        .0;
    *state.current_chat.borrow_mut() = Some(peer.into());
    handle_event(
        &state,
        ProtocolEvent::Response {
            id,
            ok: true,
            error: None,
            result: serde_json::json!({"message":{"id":"sent-location","to":peer,"mine":true,"contentType":"LOCATION","location":{"latitude":0,"longitude":-120.123456789,"title":"Test place","address":"Test address"}}}),
        },
    );
    assert!(!window.is_visible());
    assert!(state.seen_msg_ids.borrow().contains("sent-location"));
    let unavailable = gtk::Box::new(gtk::Orientation::Vertical, 0);
    locations::append_location_card(&unavailable, None);
    assert!(
        !descendants(unavailable.upcast_ref())
            .iter()
            .any(|w| w.is::<gtk::LinkButton>())
    );
    state.window.destroy();
    state.discord.shutdown();
    drain();
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires GTK and an X11 display"]
fn gtk_reaction_refresh_preserves_scroll_position() {
    let (_temp, _, _app, state) =
        chat_fixture("reaction-scroll", "dev.linegtk.ReactionScrollFixture");
    *state.current_chat.borrow_mut() = Some("u-peer".into());
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state.window.present();
    let messages: Vec<MessageInfo> = (0..60).map(|i| serde_json::from_value(serde_json::json!({
        "id": (i+1).to_string(), "from":"u-peer", "to":"u-self", "text":format!("Message {i}"), "createdTime":i+1
    })).unwrap()).collect();
    apply_messages(&state, messages.clone());
    wait_for(|| {
        state.message_scroll.vadjustment().upper()
            > state.message_scroll.vadjustment().page_size() + 400.0
    });
    let adj = state.message_scroll.vadjustment();
    *state.scroll_pinning.borrow_mut() = true;
    adj.set_value(150.0);
    *state.scroll_pinning.borrow_mut() = false;
    *state.stick_bottom.borrow_mut() = false;
    let before = adj.value();
    let mut refreshed = messages;
    refreshed[0].reactions =
        serde_json::from_value(serde_json::json!([{"from":"u-peer", "type":3}])).unwrap();
    state.pending.borrow_mut().insert(
        999,
        Pending::RefreshReactions {
            chat_mid: "u-peer".into(),
        },
    );
    handle_event(
        &state,
        ProtocolEvent::Response {
            id: 999,
            ok: false,
            result: serde_json::Value::Null,
            error: Some("offline".into()),
        },
    );
    assert_eq!(
        state.msg_stack.visible_child_name().as_deref(),
        Some("list")
    );
    apply_messages_preserving_scroll(&state, refreshed);
    wait_for(|| !*state.scroll_pinning.borrow());
    assert!(
        (adj.value() - before).abs() < 2.0,
        "expected {before}, got {}",
        adj.value()
    );
    assert!(!*state.stick_bottom.borrow());
    state.window.close();
    clear_messages(&state);
    drain();
}

#[test]
#[ignore = "requires GTK display"]
fn gtk_flex_chat_integration() {
    let (temp, _, _app, state) = chat_fixture("flex", "dev.linegtk.FlexFixture");
    *state.session_ready.borrow_mut() = true;
    let peer = "u11111111111111111111111111111111";
    *state.current_chat.borrow_mut() = Some(peer.into());
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    let contents = if let Ok(path) = std::env::var("LINE_FLEX_FIXTURE") {
        let roots: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        roots[0].clone()
    } else {
        serde_json::json!({"type":"bubble","header":{"type":"box","layout":"vertical","backgroundColor":"#0b6b5a","contents":[{"type":"text","text":"พรุ่งนี้มี 1 รายการ","color":"#ffffff","wrap":true,"weight":"bold"}]},"body":{"type":"box","layout":"vertical","contents":[{"type":"text","text":"ประชุมรับการประเมินคุณภาพมาตรฐานโรงพยาบาล","wrap":true},{"type":"text","text":"09:00–11:00 น.","wrap":true}]}})
    };
    let incoming: MessageInfo = serde_json::from_value(serde_json::json!({"id":"flex-1","from":peer,"contentType":"FLEX","flex":{"altText":"Schedule","contents":contents}})).unwrap();
    append_message(&state, &incoming, false);
    state.window.present();
    wait_for(|| {
        descendants(state.conversation.upcast_ref())
            .iter()
            .any(|w| w.has_css_class("line-bubble-flex") && w.is_mapped())
    });
    let bubble = descendants(state.conversation.upcast_ref())
        .into_iter()
        .find(|w| w.has_css_class("line-bubble-flex"))
        .unwrap();
    let card = bubble.first_child().unwrap();
    assert_eq!(
        bubble.width(),
        card.width(),
        "structured cards have no extra speech bubble padding"
    );
    assert!(
        card.width() <= 360,
        "card remains bounded inside conversation"
    );
    assert!(bubble.compute_bounds(&state.conversation).unwrap().x() >= 0.0);
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
    state
        .window
        .renderer()
        .unwrap()
        .render_texture(&snapshot.to_node().unwrap(), None)
        .save_to_png("/tmp/line-gtk-flex-chat.png")
        .unwrap();
    state.window.close();
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
#[ignore = "requires GTK display"]
fn gtk_history_prepend_keeps_reading_anchor_and_live_rows() {
    let (temp, _, _app, state) = chat_fixture("history", "dev.linegtk.HistoryFixture");
    wire_scroll_pin(&state);
    let mid = "u11111111111111111111111111111111";
    *state.session_ready.borrow_mut() = true;
    *state.current_chat.borrow_mut() = Some(mid.into());
    history::reset(&state, mid);
    state.stack.set_visible_child_name("shell");
    state.msg_stack.set_visible_child_name("list");
    state.window.present();
    let message = |n: usize| -> MessageInfo {
        serde_json::from_value(serde_json::json!({"id":n.to_string(),"from":mid,"text":format!("Message {n} with enough text to exercise a full visible chat row"),"contentType":"NONE","createdTime":1700000000000_i64 + n as i64 * 1000,"historyCursor":{"messageId":n.to_string(),"deliveredTime":n.to_string()}})).unwrap()
    };
    apply_messages(&state, (40..80).map(message).collect());
    wait_for(|| {
        !state.history.borrow().rendering
            && state.message_scroll.vadjustment().upper()
                > state.message_scroll.vadjustment().page_size()
    });
    state.history.borrow_mut().phase = history::LoadPhase::Loading(76);
    history::failed(&state, 76, mid);
    assert!(matches!(
        state.history.borrow().phase,
        history::LoadPhase::Failed
    ));
    assert_eq!(state.seen_msg_ids.borrow().len(), 40);
    state.history.borrow_mut().phase = history::LoadPhase::Loading(77);
    history::update_status(&state);
    *state.stick_bottom.borrow_mut() = false;
    *state.scroll_pinning.borrow_mut() = false;
    state.message_scroll.vadjustment().set_value(0.0);
    drain();
    let anchor = state
        .message_list
        .rows()
        .into_iter()
        .find(|row| {
            row.is_mapped()
                && row.has_css_class("line-history-message")
                && row
                    .compute_bounds(&state.message_scroll)
                    .is_some_and(|r| r.y() + r.height() > 0.0)
        })
        .unwrap();
    let position = anchor.compute_bounds(&state.message_scroll).unwrap().y();
    append_message(&state, &message(80), true);
    let mut pending = message(81);
    pending.id = "pending-history-test".into();
    pending.mine = true;
    append_message(&state, &pending, false);
    let old_incoming = state.last_incoming_id.borrow().clone();
    let page: Vec<_> = (0..40).map(|n| serde_json::json!({"id":n.to_string(),"from":mid,"text":format!("Older message {n}"),"contentType":"NONE","createdTime":1700000000000_i64+n*1000,"historyCursor":{"messageId":n.to_string(),"deliveredTime":n.to_string()}})).collect();
    history::received(
        &state,
        77,
        mid,
        &serde_json::json!({"messages":page,"before":{"messageId":"0","deliveredTime":"0"},"hasMore":false}),
    );
    let settled = Rc::new(std::cell::Cell::new(0));
    let ticks = settled.clone();
    state.window.add_tick_callback(move |_, _| {
        ticks.set(ticks.get() + 1);
        if ticks.get() >= 6 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_for(|| settled.get() >= 6);
    wait_for(|| {
        anchor.is_mapped()
            && anchor
                .compute_bounds(&state.message_scroll)
                .is_some_and(|r| (r.y() - position).abs() < 2.0)
    });
    let rows = state.message_list.rows();
    let index = rows.iter().position(|row| row == &anchor).unwrap();
    if index > 0 {
        let growing = &rows[index - 1];
        growing.set_size_request(-1, growing.height() + 80);
        let settled = Rc::new(std::cell::Cell::new(0));
        let ticks = settled.clone();
        state.window.add_tick_callback(move |_, _| {
            ticks.set(ticks.get() + 1);
            if ticks.get() >= 6 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        wait_for(|| settled.get() >= 6);
        assert!(
            (anchor.compute_bounds(&state.message_scroll).unwrap().y() - position).abs() < 2.0,
            "delayed media resize preserves reading anchor"
        );
    }
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
        .render_texture(snapshot.to_node().unwrap(), None)
        .save_to_png("/tmp/line-gtk-history.png")
        .unwrap();
    assert!(state.seen_msg_ids.borrow().contains("80"));
    assert!(
        state
            .pending_rows
            .borrow()
            .contains_key("pending-history-test")
    );
    assert_eq!(*state.last_incoming_id.borrow(), old_incoming);
    assert_eq!(state.history.borrow().messages.len(), 82);
    assert_eq!(
        state
            .message_list
            .rows()
            .iter()
            .filter(|row| row.has_css_class("line-day-sep-row"))
            .count(),
        1
    );
    assert!(matches!(
        state.history.borrow().phase,
        history::LoadPhase::Exhausted
    ));
    assert!(!*state.stick_bottom.borrow());
    let before = state.message_list.rows().len();
    history::received(
        &state,
        77,
        mid,
        &serde_json::json!({"messages":page,"hasMore":false}),
    );
    assert_eq!(state.message_list.rows().len(), before);
    state.history.borrow_mut().phase = history::LoadPhase::Loading(90);
    history::interrupted(&state);
    assert!(matches!(
        state.history.borrow().phase,
        history::LoadPhase::Failed
    ));
    assert_eq!(state.history.borrow().messages.len(), 82);
    history::received(
        &state,
        90,
        mid,
        &serde_json::json!({"messages":page,"hasMore":false}),
    );
    assert_eq!(state.history.borrow().messages.len(), 82);
    apply_messages(&state, (200..240).map(message).collect());
    history::reset(&state, "u22222222222222222222222222222222");
    *state.current_chat.borrow_mut() = Some("u22222222222222222222222222222222".into());
    apply_messages(&state, vec![message(100)]);
    wait_for(|| !state.history.borrow().rendering);
    assert_eq!(state.seen_msg_ids.borrow().len(), 1);
    history::received(
        &state,
        77,
        mid,
        &serde_json::json!({"messages":page,"hasMore":false}),
    );
    assert_eq!(state.seen_msg_ids.borrow().len(), 1);
    state.window.close();
    let _ = std::fs::remove_dir_all(temp);
}
