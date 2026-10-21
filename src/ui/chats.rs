use super::*;

pub(super) fn sort_chats(chats: &mut [ChatInfo], config: &AppConfig) {
    chats.sort_by_key(|c| {
        (
            !config.chat_pinned(&c.mid),
            std::cmp::Reverse(c.last_activity),
        )
    });
}

pub(super) fn chat_matches(config: &AppConfig, chat: &ChatInfo, query: &str) -> bool {
    !config.chat_hidden(&chat.mid)
        && (query.is_empty()
            || chat.name.to_lowercase().contains(query)
            || chat.preview.to_lowercase().contains(query))
}

pub(super) fn effective_chat_mute(state: &AppState, mid: &str) -> bool {
    let server_muted = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .is_some_and(|c| c.muted);
    state.config.borrow().chat_muted(mid, server_muted)
}

fn refresh_chat_visibility(state: &AppState) {
    let query = if *state.sidebar_compact.borrow() {
        state.compact_search_entry.text()
    } else {
        state.search_entry.text()
    };
    filter_chats(state, &query);
    let count = state
        .chats
        .borrow()
        .iter()
        .filter(|c| !state.config.borrow().chat_hidden(&c.mid))
        .count();
    if count == 0 {
        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
    } else {
        set_side_state(state, "list", None);
    }
    refresh_tray_menu(state);
}

pub(super) fn apply_chats(state: &AppState, mut chats: Vec<ChatInfo>, cached: bool) {
    sort_chats(&mut chats, &state.config.borrow());
    clear_list(&state.chat_list);
    state.chat_avatars.borrow_mut().clear();
    state.chat_previews.borrow_mut().clear();
    state.chat_unread_badges.borrow_mut().clear();

    for chat in &mut chats {
        if !chat.preview.is_empty() {
            chat.preview = localize_preview(&chat.preview);
        }
    }

    for chat in &chats {
        let (row, avatar, preview, badge) = build_chat_row(
            chat,
            *state.sidebar_compact.borrow(),
            &state.config.borrow(),
        );
        state
            .chat_avatars
            .borrow_mut()
            .insert(chat.mid.clone(), avatar);
        state
            .chat_previews
            .borrow_mut()
            .insert(chat.mid.clone(), preview);
        state
            .chat_unread_badges
            .borrow_mut()
            .insert(chat.mid.clone(), badge);
        state.chat_list.append(&row);
    }

    let n = chats.len();
    *state.chats.borrow_mut() = chats;

    if n == 0 {
        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
        state.status.set_text(&crate::i18n::t("status_no_chats"));
    } else {
        set_side_state(state, "list", None);
        let n_str = n.to_string();
        let status = if cached {
            crate::i18n::tf("status_chats_cache", &[("n", &n_str)])
        } else {
            crate::i18n::tf("status_chats_latest", &[("n", &n_str)])
        };
        state.status.set_text(&status);
        // After every rebuild, keep the open chat highlighted in the sidebar.
        select_sidebar_chat(state, state.current_chat.borrow().as_deref());
        maybe_restore_last_chat(state);
    }
    if let Some(mid) = state.current_chat.borrow().as_deref() {
        update_mute_btn(state, effective_chat_mute(state, mid));
    }
    refresh_chat_visibility(state);
}

pub(super) fn select_sidebar_chat(state: &AppState, mid: Option<&str>) {
    let Some(mid) = mid else {
        return;
    };
    if state.config.borrow().chat_hidden(mid) {
        return;
    }
    let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid) else {
        return;
    };
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.select_row(Some(&row));
    }
}

/// Ensure a sidebar row exists for this mid (first message from a new friend).
pub(super) fn ensure_chat_visible(state: &AppState, chat: &ChatInfo) {
    if state.chats.borrow().iter().any(|c| c.mid == chat.mid) {
        return;
    }
    upsert_chat_row(state, chat.clone());
}

pub(super) fn upsert_chat_row(state: &AppState, chat: ChatInfo) {
    let mid = chat.mid.clone();
    let existed = state.chats.borrow().iter().any(|c| c.mid == mid);

    if existed {
        if let Some(cur) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
            if !chat.name.is_empty() && chat.name != mid {
                cur.name = chat.name.clone();
            }
            if !chat.kind.is_empty() {
                cur.kind = chat.kind.clone();
            }
            if !chat.preview.is_empty() {
                cur.preview = chat.preview.clone();
            }
            if chat.last_activity > cur.last_activity {
                cur.last_activity = chat.last_activity;
            }
            if let Some(path) = chat.avatar_path.clone() {
                cur.avatar_path = Some(path);
            }
        }
        if let Some(label) = state.chat_previews.borrow().get(&mid)
            && !chat.preview.is_empty()
        {
            label.set_text(&chat.preview);
        }
        // Refresh visible name on the row: rebuild is heavy; update via child labels if needed.
        // Promote to top so new activity is obvious.
        promote_chat_to_top(state, &mid);
        // Update name label on the row (first heading label in the row).
        if let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid)
            && let Some(row) = state.chat_list.row_at_index(pos as i32)
        {
            update_chat_row_name(&row, &state.chats.borrow()[pos].name);
            if let Some(path) = chat.avatar_path.as_deref()
                && std::path::Path::new(path).exists()
                && let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
            {
                attach_texture_async(img, path.to_string(), 72);
            }
        }
        refresh_chat_row_status(state, &mid);
        return;
    }

    let (row, avatar, preview, badge) = build_chat_row(
        &chat,
        *state.sidebar_compact.borrow(),
        &state.config.borrow(),
    );
    state.chat_avatars.borrow_mut().insert(mid.clone(), avatar);
    state
        .chat_previews
        .borrow_mut()
        .insert(mid.clone(), preview);
    state
        .chat_unread_badges
        .borrow_mut()
        .insert(mid.clone(), badge);
    let pos = if state.config.borrow().chat_pinned(&mid) {
        0
    } else {
        state
            .chats
            .borrow()
            .iter()
            .take_while(|c| state.config.borrow().chat_pinned(&c.mid))
            .count()
    };
    state.chats.borrow_mut().insert(pos, chat);
    state.chat_list.insert(&row, pos as i32);
    set_side_state(state, "list", None);
    let n = state.chats.borrow().len();
    state.status.set_text(&crate::i18n::tf(
        "status_chats_live",
        &[("n", &n.to_string())],
    ));
    refresh_chat_visibility(state);
}

fn promote_chat(
    chats: &mut Vec<ChatInfo>,
    config: &AppConfig,
    mid: &str,
) -> Option<(usize, usize)> {
    let pos = chats.iter().position(|c| c.mid == mid)?;
    let target = if config.chat_pinned(mid) {
        0
    } else {
        chats
            .iter()
            .take_while(|c| config.chat_pinned(&c.mid))
            .count()
    };
    if pos != target {
        let chat = chats.remove(pos);
        chats.insert(target, chat);
    }
    Some((pos, target))
}

pub(super) fn promote_chat_to_top(state: &AppState, mid: &str) {
    let moved = promote_chat(&mut state.chats.borrow_mut(), &state.config.borrow(), mid);
    let Some((pos, target)) = moved else {
        return;
    };
    if pos == target {
        refresh_chat_visibility(state);
        return;
    }
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.remove(&row);
        state.chat_list.insert(&row, target as i32);
        if state.config.borrow().animations {
            row.add_css_class("line-chat-bump");
            let row_c = row.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(340), move || {
                row_c.remove_css_class("line-chat-bump");
            });
        }
    }
    // Keep selection on the open chat after row reorder.
    select_sidebar_chat(state, state.current_chat.borrow().as_deref());
    refresh_chat_visibility(state);
}

pub(super) fn update_chat_row_name(row: &gtk::ListBoxRow, name: &str) {
    if name.is_empty() {
        return;
    }
    row.set_tooltip_text(Some(name));
    // row > box > overlay + text_col > top > name label
    let Some(outer) = row.child() else { return };
    let Ok(box_) = outer.downcast::<gtk::Box>() else {
        return;
    };
    let mut child = box_.first_child();
    while let Some(w) = child {
        if w.css_classes().iter().any(|c| c == "line-chat-text")
            && let Ok(inner) = w.clone().downcast::<gtk::Box>()
            && let Some(top) = inner.first_child()
            && let Ok(top_box) = top.downcast::<gtk::Box>()
            && let Some(name_w) = top_box.first_child()
            && let Ok(label) = name_w.downcast::<gtk::Label>()
            && label.css_classes().iter().any(|c| c == "line-chat-name")
        {
            label.set_text(name);
            return;
        }
        child = w.next_sibling();
    }
}

pub(super) fn maybe_restore_last_chat(state: &AppState) {
    if *state.restored_last_chat.borrow() {
        return;
    }
    if state.current_chat.borrow().is_some() {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    if !*state.session_ready.borrow() {
        return;
    }
    let last = state.config.borrow().last_chat_mid.clone();
    if last.is_empty() || state.config.borrow().chat_hidden(&last) {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    let chats = state.chats.borrow();
    let Some(chat) = chats.iter().find(|c| c.mid == last).cloned() else {
        return; // keep trying until the mid appears in a later refresh
    };
    drop(chats);
    *state.restored_last_chat.borrow_mut() = true;
    open_chat(state, &chat);
    select_sidebar_chat(state, Some(chat.mid.as_str()));
}

pub(super) fn build_chat_row(
    chat: &ChatInfo,
    compact: bool,
    config: &AppConfig,
) -> (gtk::ListBoxRow, gtk::Picture, gtk::Label, gtk::Label) {
    let row = gtk::ListBoxRow::new();
    row.set_tooltip_text(Some(&chat.name));
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(if compact { 6 } else { 12 })
        .margin_start(if compact { 0 } else { 8 })
        .margin_end(if compact { 0 } else { 8 })
        .margin_top(8)
        .margin_bottom(8)
        .hexpand(!compact)
        .halign(if compact {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        })
        .css_classes(["line-chat-row"])
        .build();

    let avatar_px = if compact {
        AVATAR_COMPACT_PX
    } else {
        AVATAR_CHAT_PX
    };
    let avatar_frame = gtk::Box::builder()
        .width_request(avatar_px)
        .height_request(avatar_px)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-frame"])
        .build();
    let avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(avatar_px)
        .height_request(avatar_px)
        .css_classes(["line-avatar"])
        .build();
    if let Some(path) = chat.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
    {
        attach_texture_async(avatar.clone(), path.to_string(), avatar_px * 2);
    }
    avatar_frame.append(&avatar);

    let avatar_overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-overlay"])
        .build();
    avatar_overlay.set_child(Some(&avatar_frame));

    let badge = gtk::Label::builder()
        .label(if chat.unread > 0 {
            chat.unread.to_string()
        } else {
            String::new()
        })
        .css_classes(["line-unread", "line-unread-overlay"])
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .visible(chat.unread > 0)
        .build();
    avatar_overlay.add_overlay(&badge);

    let text_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .visible(!compact)
        .css_classes(["line-chat-text"])
        .build();

    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let name = gtk::Label::builder()
        .label(&chat.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-name"])
        .build();
    let time = gtk::Label::builder()
        .label(format_activity(chat.last_activity))
        .css_classes(["dim-label", "caption"])
        .build();
    top.append(&name);
    top.append(&time);

    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let preview_text = if chat.preview.is_empty() {
        if chat.last_activity > 0 {
            crate::i18n::t("loading_last")
        } else {
            crate::i18n::t("no_recent")
        }
    } else {
        localize_preview(&chat.preview)
    };
    let preview = gtk::Label::builder()
        .label(&preview_text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-preview"])
        .build();
    bottom.append(&preview);

    text_col.append(&top);
    text_col.append(&bottom);
    box_.append(&avatar_overlay);
    box_.append(&text_col);
    let status = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .valign(gtk::Align::Center)
        .css_classes(["line-chat-status", "dim-label"])
        .build();
    for (name, class) in [
        ("view-pin-symbolic", "line-chat-pinned"),
        ("notifications-disabled-symbolic", "line-chat-muted"),
    ] {
        let icon = gtk::Image::from_icon_name(name);
        icon.set_pixel_size(14);
        icon.add_css_class(class);
        status.append(&icon);
    }
    box_.append(&status);
    row.set_child(Some(&box_));
    update_chat_row_status(
        &row,
        config.chat_pinned(&chat.mid),
        config.chat_muted(&chat.mid, chat.muted),
    );
    (row, avatar, preview, badge)
}

fn update_chat_row_status(row: &gtk::ListBoxRow, pinned: bool, muted: bool) {
    let Some(outer) = row.child() else {
        return;
    };
    let mut child = outer.first_child();
    while let Some(status) = child {
        if status.has_css_class("line-chat-status") {
            status.set_visible(pinned || muted);
            let mut icon = status.first_child();
            while let Some(w) = icon {
                let (visible, label) = if w.has_css_class("line-chat-pinned") {
                    (pinned, crate::i18n::t("chat_pinned_status"))
                } else {
                    (muted, crate::i18n::t("chat_muted_status"))
                };
                w.set_visible(visible);
                w.set_tooltip_text(Some(&label));
                w.update_property(&[gtk::accessible::Property::Label(&label)]);
                icon = w.next_sibling();
            }
            return;
        }
        child = status.next_sibling();
    }
}

pub(super) fn refresh_chat_row_status(state: &AppState, mid: &str) {
    let chats = state.chats.borrow();
    let Some(index) = chats.iter().position(|c| c.mid == mid) else {
        return;
    };
    let Some(row) = state.chat_list.row_at_index(index as i32) else {
        return;
    };
    let config = state.config.borrow();
    update_chat_row_status(
        &row,
        config.chat_pinned(mid),
        config.chat_muted(mid, chats[index].muted),
    );
}

pub(super) fn format_activity(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let secs = ts / 1000;
    let now = chrono::Utc::now().timestamp();
    let diff = (now - secs).max(0);
    if diff < 60 {
        "now".into()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 86400 * 7 {
        format!("{}d", diff / 86400)
    } else {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.format("%m/%d").to_string())
            .unwrap_or_default()
    }
}

pub(super) fn clear_list(list: &gtk::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}

pub(super) fn msg_id_le(a: &str, b: &str) -> bool {
    match (a.parse::<u128>(), b.parse::<u128>()) {
        (Ok(x), Ok(y)) => x <= y,
        _ => a <= b,
    }
}

pub(super) fn mark_chat_read(state: &AppState, chat_mid: &str) {
    if !state.config.borrow().auto_mark_read {
        return;
    }
    let Some(last_id) = state.last_incoming_id.borrow().clone() else {
        return;
    };
    if last_id.is_empty() {
        return;
    }
    let _ = state.sidecar.mark_read(chat_mid, &last_id);
}

pub(super) fn clear_unread_badge(state: &AppState, chat_mid: &str) {
    if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = 0;
    }
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text("");
        badge.set_visible(false);
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid) {
        state
            .chat_subtitle
            .set_text(&crate::i18n::t("conversation"));
    }
    refresh_tray_menu(state);
}

pub(super) fn bump_unread(state: &AppState, chat_mid: &str) {
    let n = if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = chat.unread.saturating_add(1);
        chat.unread
    } else {
        1
    };
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text(&n.to_string());
        badge.set_visible(true);
    }
    refresh_tray_menu(state);
}

pub(super) fn update_mute_btn(state: &AppState, muted: bool) {
    if muted {
        state.mute_btn.set_icon_name("audio-volume-muted-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("unmute_chat")));
    } else {
        state.mute_btn.set_icon_name("audio-volume-high-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("mute_chat")));
    }
}

pub(super) fn toggle_chat_mute(state: &AppState) {
    let mid = state.current_chat.borrow().clone();
    if let Some(mid) = mid {
        toggle_chat_mute_for(state, &mid);
    }
}

fn toggle_chat_mute_for(state: &AppState, mid: &str) {
    let next = !effective_chat_mute(state, mid);
    {
        let mut config = state.config.borrow_mut();
        config
            .chat_preferences
            .entry(mid.to_string())
            .or_default()
            .muted = Some(next);
        config.save(&state.data_dir);
    }
    if next {
        state
            .notif_pending
            .borrow_mut()
            .retain(|_, n| n.chat_mid != mid);
    }
    if state.current_chat.borrow().as_deref() == Some(mid) {
        update_mute_btn(state, next);
    }
    refresh_chat_row_status(state, mid);
}

pub(super) fn set_chat_hidden(state: &AppState, mid: &str, hidden: bool) {
    if hidden && state.current_chat.borrow().as_deref() == Some(mid) {
        finish_voice_record(state, false);
        *state.current_chat.borrow_mut() = None;
        refresh_attachment_drafts(state);
        state.chat_list.unselect_all();
        state.chat_title.set_text("");
        state.chat_subtitle.set_text("");
        state.composer.set_text("");
        state.media_queue.borrow_mut().clear();
        hide_upload_progress(state);
        clear_messages(state);
        set_msg_state(state, "idle", None);
        state.mute_btn.set_sensitive(false);
        refresh_call_controls(state);
        sync_discord_rpc(state);
    }
    {
        let mut config = state.config.borrow_mut();
        config
            .chat_preferences
            .entry(mid.to_string())
            .or_default()
            .hidden = hidden;
        if hidden && config.last_chat_mid == mid {
            config.last_chat_mid.clear();
        }
        config.save(&state.data_dir);
    }
    if !hidden && !state.chats.borrow().iter().any(|c| c.mid == mid) {
        upsert_chat_row(
            state,
            ChatInfo {
                mid: mid.to_string(),
                name: mid.to_string(),
                kind: String::new(),
                avatar_path: None,
                last_activity: 0,
                unread: 0,
                preview: String::new(),
                muted: false,
            },
        );
    }
    refresh_chat_visibility(state);
}

pub(super) fn wire_chat_context_menu(state: &AppState) {
    for name in ["hide-chat", "mute-chat", "pin-chat"] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let s = state.clone();
        action.connect_activate(move |_, target| {
            let Some(mid) = target.and_then(|v| v.str()) else {
                return;
            };
            match name {
                "hide-chat" => set_chat_hidden(&s, mid, true),
                "mute-chat" => toggle_chat_mute_for(&s, mid),
                _ => {
                    {
                        let mut config = s.config.borrow_mut();
                        let prefs = config.chat_preferences.entry(mid.to_string()).or_default();
                        prefs.pinned = !prefs.pinned;
                        config.save(&s.data_dir);
                    }
                    let chats = s.chats.borrow().clone();
                    apply_chats(&s, chats, false);
                }
            }
        });
        state.app.add_action(&action);
    }
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = state.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let Some(row) = s.chat_list.row_at_y(y as i32) else {
            return;
        };
        let Some(chat) = s.chats.borrow().get(row.index() as usize).cloned() else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let menu = chat_context_menu(&s.config.borrow(), &chat);
        show_chat_context_popover(&s.chat_list, &menu, x, y);
    });
    state.chat_list.add_controller(gesture);
}

fn show_chat_context_popover(
    list: &gtk::ListBox,
    menu: &gio::Menu,
    x: f64,
    y: f64,
) -> gtk::PopoverMenu {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_has_arrow(false);
    popover.set_parent(list);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    let weak = popover.downgrade();
    let destroy_handler = RefCell::new(Some(list.connect_destroy(move |_| {
        if let Some(p) = weak.upgrade()
            && p.parent().is_some()
        {
            p.unparent();
        }
    })));
    let weak_list = list.downgrade();
    popover.connect_closed(move |p| {
        if let Some(handler) = destroy_handler.borrow_mut().take()
            && let Some(list) = weak_list.upgrade()
        {
            list.disconnect(handler);
        }
        // GtkModelButton closes its popover before resolving the action through
        // widget ancestors. Keep that ancestry until dispatch has completed.
        let p = p.clone();
        glib::idle_add_local_once(move || {
            if p.parent().is_some() {
                p.unparent();
            }
        });
    });
    popover.popup();
    popover
}

fn chat_context_menu(config: &AppConfig, chat: &ChatInfo) -> gio::Menu {
    let menu = gio::Menu::new();
    for (label, action) in [
        ("hide_chat", "app.hide-chat"),
        (
            if config.chat_muted(&chat.mid, chat.muted) {
                "unmute_chat"
            } else {
                "mute_chat"
            },
            "app.mute-chat",
        ),
        (
            if config.chat_pinned(&chat.mid) {
                "unpin_chat"
            } else {
                "pin_chat"
            },
            "app.pin-chat",
        ),
    ] {
        let item = gio::MenuItem::new(Some(&crate::i18n::t(label)), None);
        item.set_action_and_target_value(Some(action), Some(&chat.mid.to_variant()));
        menu.append_item(&item);
    }
    menu
}

pub(super) fn apply_peer_read(state: &AppState, chat_mid: &str, message_id: &str) {
    if chat_mid.is_empty() || message_id.is_empty() {
        return;
    }
    let prev = state.read_upto.borrow().get(chat_mid).cloned();
    if let Some(p) = prev.as_deref()
        && msg_id_le(message_id, p)
    {
        return;
    }
    state
        .read_upto
        .borrow_mut()
        .insert(chat_mid.to_string(), message_id.to_string());

    if state.current_chat.borrow().as_deref() != Some(chat_mid) {
        return;
    }
    let times = state.msg_created.borrow().clone();
    for (id, label) in state.receipt_slots.borrow().iter() {
        if msg_id_le(id, message_id) {
            let ts = times.get(id).copied().unwrap_or(0);
            label.set_text(&format_outgoing_status(true, ts));
            label.set_tooltip_text(Some(&crate::i18n::t("status_read")));
            label.add_css_class("line-msg-status-read");
        }
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;

    fn chat(mid: &str, activity: i64) -> ChatInfo {
        serde_json::from_value(
            serde_json::json!({"mid": mid, "name": mid, "lastActivity": activity}),
        )
        .unwrap()
    }

    #[test]
    fn context_menu_targets_clicked_chat_and_uses_fresh_preferences() {
        let mut cfg = AppConfig::default();
        let mut group = chat("c-group", 10);
        group.muted = true;
        let menu = chat_context_menu(&cfg, &group);
        assert_eq!(menu.n_items(), 3);
        for (index, action) in ["app.hide-chat", "app.mute-chat", "app.pin-chat"]
            .iter()
            .enumerate()
        {
            assert_eq!(
                menu.item_attribute_value(index as i32, "action", None)
                    .unwrap()
                    .str(),
                Some(*action)
            );
            assert_eq!(
                menu.item_attribute_value(index as i32, "target", None)
                    .unwrap()
                    .str(),
                Some("c-group")
            );
        }
        let prefs = cfg.chat_preferences.entry("c-group".into()).or_default();
        prefs.muted = Some(false);
        prefs.pinned = true;
        let updated = chat_context_menu(&cfg, &group);
        assert_eq!(
            updated
                .item_attribute_value(1, "label", None)
                .unwrap()
                .str(),
            Some(crate::i18n::t("mute_chat").as_str())
        );
        assert_eq!(
            updated
                .item_attribute_value(2, "label", None)
                .unwrap()
                .str(),
            Some(crate::i18n::t("unpin_chat").as_str())
        );
        assert_eq!(
            menu.item_attribute_value(1, "label", None).unwrap().str(),
            Some(crate::i18n::t("unmute_chat").as_str())
        );
    }

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(w) = child {
            result.extend(descendants(&w));
            child = w.next_sibling();
        }
        result
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn gtk_visible_menu_activates_application_action() {
        libadwaita::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        let app = Application::builder()
            .application_id("dev.linegtk.ContextMenuRegression")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = ApplicationWindow::builder().application(&app).build();
        let list = gtk::ListBox::new();
        window.set_content(Some(&list));
        let (row, _, _, _) = build_chat_row(&chat("c-target", 10), false, &AppConfig::default());
        list.append(&row);
        let hits = Rc::new(RefCell::new(Vec::new()));
        for name in ["hide-chat", "mute-chat", "pin-chat"] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let hits = hits.clone();
            action.connect_activate(move |_, target| {
                hits.borrow_mut()
                    .push((name.to_string(), target.unwrap().str().unwrap().to_string()))
            });
            app.add_action(&action);
        }
        window.present();
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        let popover = show_chat_context_popover(
            &list,
            &chat_context_menu(&AppConfig::default(), &chat("c-target", 10)),
            10.0,
            10.0,
        );
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        let widgets = descendants(popover.upcast_ref());
        let button = widgets
            .iter()
            .find(|w| w.type_().name() == "GtkModelButton")
            .unwrap();
        assert!(button.is_sensitive());
        assert!(button.activate());
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert_eq!(
            *hits.borrow(),
            vec![("hide-chat".to_string(), "c-target".to_string())]
        );
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn gtk_context_menu_preserves_selection_and_unparents_on_close() {
        gtk::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        let window = gtk::Window::new();
        let list = gtk::ListBox::new();
        window.set_child(Some(&list));
        for compact in [false, true] {
            let (selected, _, _, _) =
                build_chat_row(&chat("u-selected", 20), compact, &AppConfig::default());
            let (clicked, _, _, _) =
                build_chat_row(&chat("c-clicked", 10), compact, &AppConfig::default());
            list.append(&selected);
            list.append(&clicked);
            list.select_row(Some(&selected));
            let activations = Rc::new(std::cell::Cell::new(0));
            let count = activations.clone();
            let handler = list.connect_row_activated(move |_, _| count.set(count.get() + 1));
            window.present();
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            let menu = chat_context_menu(&AppConfig::default(), &chat("c-clicked", 10));
            let popover = show_chat_context_popover(&list, &menu, 10.0, 10.0);
            assert_eq!(list.selected_row(), Some(selected.clone()));
            assert_eq!(activations.get(), 0);
            assert!(popover.parent().is_some());
            popover.popdown();
            popover.emit_by_name::<()>("closed", &[]);
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            assert!(popover.parent().is_none());
            drop(popover);
            list.disconnect(handler);
            clear_list(&list);
        }
        window.close();
    }

    #[test]
    fn pins_stay_above_new_activity_and_hidden_chats_stay_hidden_after_refresh() {
        let mut cfg = AppConfig::default();
        cfg.chat_preferences
            .entry("c-pinned".into())
            .or_default()
            .pinned = true;
        cfg.chat_preferences
            .entry("u-hidden".into())
            .or_default()
            .hidden = true;
        let mut chats = vec![
            chat("u-old", 10),
            chat("u-hidden", 900),
            chat("c-pinned", 1),
            chat("u-new", 100),
        ];
        sort_chats(&mut chats, &cfg);
        assert_eq!(
            chats.iter().map(|c| c.mid.as_str()).collect::<Vec<_>>(),
            ["c-pinned", "u-hidden", "u-new", "u-old"]
        );
        assert!(!chat_matches(&cfg, &chats[1], ""));
        assert!(!chat_matches(&cfg, &chats[1], "hidden"));
        chats[3].last_activity = 1000;
        assert_eq!(promote_chat(&mut chats, &cfg, "u-old"), Some((3, 1)));
        assert_eq!(chats[0].mid, "c-pinned");
        assert_eq!(chats[1].mid, "u-old");
        assert!(!chat_matches(
            &cfg,
            chats.iter().find(|c| c.mid == "u-hidden").unwrap(),
            ""
        ));
        cfg.chat_preferences.get_mut("u-hidden").unwrap().hidden = false;
        assert!(chat_matches(
            &cfg,
            chats.iter().find(|c| c.mid == "u-hidden").unwrap(),
            "hidden"
        ));
        assert!(!chat_matches(&cfg, &chats[0], "does-not-match"));
        let refreshed = chats.iter().map(|c| c.mid.clone()).collect::<Vec<_>>();
        sort_chats(&mut chats, &cfg);
        assert_eq!(
            chats.iter().map(|c| c.mid.clone()).collect::<Vec<_>>(),
            refreshed
        );
        cfg.chat_preferences
            .entry("u-new".into())
            .or_default()
            .pinned = true;
        sort_chats(&mut chats, &cfg);
        assert_eq!(promote_chat(&mut chats, &cfg, "c-pinned"), Some((1, 0)));
        assert_eq!(chats[0].mid, "c-pinned");
        assert_eq!(chats[1].mid, "u-new");
        assert_eq!(promote_chat(&mut chats, &cfg, "u-old"), Some((2, 2)));
        assert_eq!(promote_chat(&mut chats, &cfg, "missing"), None);
    }
}
