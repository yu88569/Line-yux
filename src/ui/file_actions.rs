use super::*;

pub(super) fn build_file_actions(state: &AppState, msg: &MessageInfo) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("line-file-actions");
    for (label, action) in [("file_save", "file_save"), ("file_save_as", "file_save_as")] {
        let button = gtk::Button::with_label(&crate::i18n::t(label));
        button.add_css_class("flat");
        button.add_css_class("caption");
        let s = state.clone();
        let msg = msg.clone();
        button.connect_clicked(move |_| request_media_download(&s, &msg, action));
        actions.append(&button);
    }
    let share = gtk::Button::with_label(&crate::i18n::t("file_share"));
    share.add_css_class("flat");
    share.add_css_class("caption");
    let s = state.clone();
    let msg = msg.clone();
    share.connect_clicked(move |_| open_share_picker(&s, &msg));
    actions.append(&share);
    actions
}

pub(super) fn open_share_picker(state: &AppState, msg: &MessageInfo) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let Some(source_chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    let chats = state.chats.borrow().clone();
    if chats.is_empty() {
        toast(state, &crate::i18n::t("no_chats"));
        return;
    }
    let window = gtk::Window::builder()
        .title(crate::i18n::t("file_share"))
        .transient_for(&state.window)
        .modal(true)
        .default_width(380)
        .default_height(460)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();
    let name = gtk::Label::builder()
        .label(suggest_media_name(msg))
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .xalign(0.0)
        .build();
    root.append(&name);
    let search = gtk::SearchEntry::builder()
        .placeholder_text(crate::i18n::t("file_share_search"))
        .build();
    root.append(&search);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("boxed-list");
    for chat in &chats {
        let label = gtk::Label::builder()
            .label(&chat.name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        list.append(&label);
    }
    list.unselect_all();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    root.append(&scroll);
    let empty = gtk::Label::new(Some(&crate::i18n::t("file_share_no_match")));
    empty.set_visible(false);
    root.append(&empty);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&crate::i18n::t("file_share_cancel"));
    let send = gtk::Button::with_label(&crate::i18n::t("send"));
    send.add_css_class("suggested-action");
    send.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&send);
    root.append(&buttons);
    window.set_child(Some(&root));
    let send_selected = send.clone();
    list.connect_row_selected(move |_, row| send_selected.set_sensitive(row.is_some()));
    let filtered_list = list.clone();
    let names: Vec<_> = chats.iter().map(|chat| chat.name.to_lowercase()).collect();
    search.connect_search_changed(move |entry| {
        let query = entry.text().to_lowercase();
        filtered_list.unselect_all();
        empty.set_visible(!names.iter().any(|name| name.contains(&query)));
        let names = names.clone();
        filtered_list.set_filter_func(move |row| {
            names
                .get(row.index() as usize)
                .is_some_and(|name| name.contains(&query))
        });
    });
    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());
    let s = state.clone();
    let msg = msg.clone();
    let send_window = window.clone();
    let sent = std::cell::Cell::new(false);
    send.connect_clicked(move |button| {
        if sent.get() {
            return;
        }
        let Some(chat) = list
            .selected_row()
            .and_then(|row| chats.get(row.index() as usize))
        else {
            return;
        };
        sent.set(true);
        button.set_sensitive(false);
        send_window.close();
        if !*s.session_ready.borrow() {
            toast(&s, &crate::i18n::t("still_restoring"));
            return;
        }
        request_media_action(
            &s,
            &msg,
            PendingMediaAction::ShareFile {
                chat_mid: chat.mid.clone(),
                source_chat_mid: source_chat_mid.clone(),
            },
        );
    });
    window.present();
}

pub(super) fn send_shared_file(state: &AppState, chat_mid: &str, path: &str, name: &str) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    match state.sidecar.share_file(chat_mid, path, name) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::ShareFile {
                    chat_mid: chat_mid.to_string(),
                },
            );
            toast(state, &crate::i18n::t("file_share_sending"));
        }
        Err(error) => toast(
            state,
            &crate::i18n::tf("send_failed", &[("error", &error.to_string())]),
        ),
    }
}

pub(super) fn complete_shared_file(state: &AppState, chat_mid: &str, result: &serde_json::Value) {
    toast(state, &crate::i18n::t("file_share_sent"));
    if state.current_chat.borrow().as_deref() != Some(chat_mid) {
        return;
    }
    if let Ok(msg) =
        serde_json::from_value::<MessageInfo>(result.get("message").cloned().unwrap_or_default())
    {
        set_msg_state(state, "list", None);
        append_message(state, &msg, true);
        pin_messages_to_latest(state);
    }
}
