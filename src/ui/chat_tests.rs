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

#[test]
#[ignore = "requires a GTK display"]
fn gtk_menu_actions_mutate_real_state_and_show_status_icons() {
    libadwaita::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    let temp =
        std::env::temp_dir().join(format!("line-gtk-menu-interaction-{}", std::process::id()));
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
        .application_id("dev.linegtk.ChatInteractionFixture")
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
    wire_actions(&state);
    *state.session_ready.borrow_mut() = true;
    *state.restored_last_chat.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.window.set_default_size(1000, 700);
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
