use super::*;
use crate::protocol::LocationInfo;
use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShareMediaType {
    Image,
    Video,
    Audio,
    File,
}

impl ShareMediaType {
    pub(super) fn as_o_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum ShareableKind {
    Text(String),
    Media {
        o_type: ShareMediaType,
        duration_ms: Option<u64>,
    },
    Location(LocationInfo),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum ShareSelection {
    #[default]
    Off,
    Selecting {
        chat_mid: String,
        selected_ids: HashSet<String>,
    },
}

#[derive(Clone)]
pub(super) struct MessageSelectRow {
    pub row: glib::WeakRef<gtk::ListBoxRow>,
    pub check: glib::WeakRef<gtk::CheckButton>,
}

#[derive(Debug, Clone)]
pub(super) struct ShareSourceItem {
    pub msg: MessageInfo,
    pub kind: ShareableKind,
}

#[derive(Debug, Clone)]
pub(super) enum PreparedShareItem {
    Text(String),
    Media {
        o_type: ShareMediaType,
        path: String,
        file_name: String,
        duration_ms: Option<u64>,
    },
    Location(LocationInfo),
}

pub(super) struct ShareBatch {
    pub source_chat_mid: String,
    pub target_chat_mids: Vec<String>,
    pub pending_prep: VecDeque<ShareSourceItem>,
    pub prepared: Vec<PreparedShareItem>,
    pub send_queue: VecDeque<(String, usize)>,
    pub sent_count: usize,
    pub failed_count: usize,
    pub last_error: Option<String>,
    pub queued_sends: bool,
}

pub(super) fn classify_shareable(msg: &MessageInfo) -> Option<ShareableKind> {
    if !is_real_message_id(&msg.id) || msg.flex.is_some() || msg.album.is_some() {
        return None;
    }
    let ct = msg.content_type.trim().to_ascii_uppercase();
    match ct.as_str() {
        "" | "NONE" => {
            if msg.text.trim().is_empty() {
                None
            } else {
                Some(ShareableKind::Text(msg.text.clone()))
            }
        }
        "IMAGE" => Some(ShareableKind::Media {
            o_type: ShareMediaType::Image,
            duration_ms: None,
        }),
        "VIDEO" => Some(ShareableKind::Media {
            o_type: ShareMediaType::Video,
            duration_ms: msg.duration_ms.filter(|&d| d > 0).map(|d| d as u64),
        }),
        "AUDIO" => Some(ShareableKind::Media {
            o_type: ShareMediaType::Audio,
            duration_ms: msg.duration_ms.filter(|&d| d > 0).map(|d| d as u64),
        }),
        "FILE" => Some(ShareableKind::Media {
            o_type: ShareMediaType::File,
            duration_ms: None,
        }),
        "LOCATION" => msg
            .location
            .as_deref()
            .filter(|loc| loc.is_valid())
            .cloned()
            .map(ShareableKind::Location),
        _ => None,
    }
}

fn merge_fresh_message(msg: &MessageInfo, fresh: Option<&MessageInfo>) -> MessageInfo {
    let Some(fresh) = fresh else {
        return msg.clone();
    };
    let mut merged = msg.clone();
    if fresh.image_path.is_some() {
        merged.image_path = fresh.image_path.clone();
    }
    if fresh.audio_path.is_some() {
        merged.audio_path = fresh.audio_path.clone();
    }
    if fresh.file_path.is_some() {
        merged.file_path = fresh.file_path.clone();
    }
    if fresh.file_name.is_some() {
        merged.file_name = fresh.file_name.clone();
    }
    if fresh.file_size.is_some() {
        merged.file_size = fresh.file_size;
    }
    if fresh.duration_ms.is_some() {
        merged.duration_ms = fresh.duration_ms;
    }
    merged
}

pub(super) fn collect_ordered_selection(
    messages: &[MessageInfo],
    fresh_media: &HashMap<String, MessageInfo>,
    selected_ids: &HashSet<String>,
) -> Vec<ShareSourceItem> {
    let mut by_id = HashMap::new();
    for msg in messages {
        if !selected_ids.contains(&msg.id) {
            continue;
        }
        let merged = merge_fresh_message(msg, fresh_media.get(&msg.id));
        if let Some(kind) = classify_shareable(&merged) {
            by_id.insert(merged.id.clone(), ShareSourceItem { msg: merged, kind });
        }
    }
    for (id, fresh) in fresh_media {
        if !selected_ids.contains(id) || by_id.contains_key(id) {
            continue;
        }
        if let Some(kind) = classify_shareable(fresh) {
            by_id.insert(id.clone(), ShareSourceItem { msg: fresh.clone(), kind });
        }
    }
    let mut items: Vec<_> = by_id.into_values().collect();
    items.sort_by(|a, b| {
        a.msg
            .created_time
            .cmp(&b.msg.created_time)
            .then_with(|| {
                a.msg
                    .id
                    .parse::<u128>()
                    .unwrap_or(u128::MAX)
                    .cmp(&b.msg.id.parse::<u128>().unwrap_or(u128::MAX))
            })
            .then_with(|| a.msg.id.cmp(&b.msg.id))
    });
    items
}

pub(super) fn resolve_share_filename(
    msg: &MessageInfo,
    downloaded_name: Option<&str>,
    path: &str,
) -> String {
    if let Some(name) = downloaded_name.and_then(meaningful_file_name) {
        return name.to_string();
    }
    if msg.content_type.eq_ignore_ascii_case("FILE") {
        return suggest_media_name(msg);
    }
    if let Some(name) = msg.file_name.as_deref().and_then(meaningful_file_name) {
        return name.to_string();
    }
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return format!("{}.{ext}", msg.id);
    }
    suggest_media_name(msg)
}

pub(super) fn is_selecting(state: &AppState) -> bool {
    !matches!(*state.share_selection.borrow(), ShareSelection::Off)
}

pub(super) fn wire_controls(state: &AppState) {
    let s = state.clone();
    state.share_select_back.connect_clicked(move |_| {
        exit_selection_mode(&s);
    });
    let s = state.clone();
    state.share_confirm_btn.connect_clicked(move |_| {
        open_picker_for_current_selection(&s);
    });
}

pub(super) fn apply_language(state: &AppState) {
    state
        .share_select_title
        .set_text(&crate::i18n::t("share_select_title"));
    state
        .share_select_back
        .set_tooltip_text(Some(&crate::i18n::t("cancel")));
    refresh_selection_ui(state);
}

pub(super) fn enter_selection_mode(state: &AppState, initial_msg_id: &str) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    finish_voice_record(state, false);
    let mut selected_ids = HashSet::new();
    if !initial_msg_id.is_empty() {
        selected_ids.insert(initial_msg_id.to_string());
    }
    *state.share_selection.borrow_mut() = ShareSelection::Selecting {
        chat_mid,
        selected_ids,
    };
    state.reply_strip.set_visible(false);
    state.attachment_scroll.set_visible(false);
    state.chat_header_stack.set_visible_child_name("select");
    state.composer_stack.set_visible_child_name("share");
    sync_all_row_checks(state);
    refresh_selection_ui(state);
}

pub(super) fn exit_selection_mode(state: &AppState) {
    if !is_selecting(state) {
        return;
    }
    *state.share_selection.borrow_mut() = ShareSelection::Off;
    state.chat_header_stack.set_visible_child_name("normal");
    state.composer_stack.set_visible_child_name("compose");
    state
        .reply_strip
        .set_visible(state.reply_target.borrow().is_some());
    refresh_attachment_drafts(state);
    sync_all_row_checks(state);
    refresh_selection_ui(state);
}

fn sync_all_row_checks(state: &AppState) {
    let current = state.current_chat.borrow().clone();
    let (visible, selected_ids) = match &*state.share_selection.borrow() {
        ShareSelection::Selecting {
            chat_mid,
            selected_ids,
        } if current.as_deref() == Some(chat_mid.as_str()) => (true, Some(selected_ids.clone())),
        _ => (false, None),
    };
    let mut rows = state.share_row_checks.borrow_mut();
    rows.retain(|id, entry| {
        let Some(row) = entry.row.upgrade() else {
            return false;
        };
        let Some(check) = entry.check.upgrade() else {
            return false;
        };
        let active = selected_ids.as_ref().is_some_and(|ids| ids.contains(id));
        check.set_visible(visible);
        if check.is_active() != active {
            check.set_active(active);
        }
        if visible && active {
            row.add_css_class("line-msg-selected");
        } else {
            row.remove_css_class("line-msg-selected");
        }
        true
    });
}

fn toggle_message_selection(state: &AppState, msg_id: &str) {
    let (next_active, row_opt, check_opt) = {
        let mut sel = state.share_selection.borrow_mut();
        let ShareSelection::Selecting { selected_ids, .. } = &mut *sel else {
            return;
        };
        let active = if selected_ids.contains(msg_id) {
            selected_ids.remove(msg_id);
            false
        } else {
            selected_ids.insert(msg_id.to_string());
            true
        };
        let entry = state.share_row_checks.borrow().get(msg_id).cloned();
        (
            active,
            entry.as_ref().and_then(|e| e.row.upgrade()),
            entry.as_ref().and_then(|e| e.check.upgrade()),
        )
    };
    if let Some(check) = check_opt
        && check.is_active() != next_active
    {
        check.set_active(next_active);
    }
    if let Some(row) = row_opt {
        if next_active {
            row.add_css_class("line-msg-selected");
        } else {
            row.remove_css_class("line-msg-selected");
        }
    }
    refresh_selection_ui(state);
}

pub(super) fn refresh_selection_ui(state: &AppState) {
    let count = match &*state.share_selection.borrow() {
        ShareSelection::Selecting { selected_ids, .. } => selected_ids.len(),
        ShareSelection::Off => 0,
    };
    let label = if count > 0 {
        crate::i18n::tf("share_button_count", &[("n", &count.to_string())])
    } else {
        crate::i18n::t("share_message")
    };
    state.share_confirm_btn.set_label(&label);
    state.share_confirm_btn.set_sensitive(count > 0);
}

pub(super) fn attach_row_selection(
    state: &AppState,
    row: &gtk::ListBoxRow,
    outer: &gtk::Box,
    msg: &MessageInfo,
) {
    if classify_shareable(msg).is_none() {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let s = state.clone();
        click.connect_pressed(move |gesture, _, _, _| {
            if is_selecting(&s) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        row.add_controller(click);
        return;
    }

    let check = gtk::CheckButton::builder()
        .valign(gtk::Align::Center)
        .margin_start(4)
        .margin_end(4)
        .css_classes(["line-msg-select-check"])
        .build();

    let current = state.current_chat.borrow().clone();
    let (visible, active) = match &*state.share_selection.borrow() {
        ShareSelection::Selecting {
            chat_mid,
            selected_ids,
        } if current.as_deref() == Some(chat_mid.as_str()) => {
            (true, selected_ids.contains(&msg.id))
        }
        _ => (false, false),
    };
    check.set_visible(visible);
    check.set_active(active);
    if visible && active {
        row.add_css_class("line-msg-selected");
    }
    outer.append(&check);

    state.share_row_checks.borrow_mut().insert(
        msg.id.clone(),
        MessageSelectRow {
            row: row.downgrade(),
            check: check.downgrade(),
        },
    );

    let s = state.clone();
    let msg_id = msg.id.clone();
    let weak_row = row.downgrade();
    check.connect_toggled(move |btn| {
        let active = btn.is_active();
        {
            let mut sel = s.share_selection.borrow_mut();
            let ShareSelection::Selecting { selected_ids, .. } = &mut *sel else {
                return;
            };
            if selected_ids.contains(&msg_id) == active {
                return;
            }
            if active {
                selected_ids.insert(msg_id.clone());
            } else {
                selected_ids.remove(&msg_id);
            }
        }
        if let Some(row) = weak_row.upgrade() {
            if active {
                row.add_css_class("line-msg-selected");
            } else {
                row.remove_css_class("line-msg-selected");
            }
        }
        refresh_selection_ui(&s);
    });

    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = state.clone();
    let msg_id = msg.id.clone();
    let weak_row = row.downgrade();
    let weak_check = check.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        if !is_selecting(&s) {
            return;
        }
        if let (Some(row), Some(check)) = (weak_row.upgrade(), weak_check.upgrade())
            && let Some(bounds) = check.compute_bounds(&row)
            && bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))
        {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        toggle_message_selection(&s, &msg_id);
    });
    row.add_controller(click);
}

fn open_picker_for_current_selection(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let (source_chat_mid, selected_ids) = match &*state.share_selection.borrow() {
        ShareSelection::Selecting {
            chat_mid,
            selected_ids,
        } if !selected_ids.is_empty() => (chat_mid.clone(), selected_ids.clone()),
        _ => return,
    };
    let items = collect_ordered_selection(
        &state.history.borrow().messages,
        &state.media_msgs.borrow(),
        &selected_ids,
    );
    if items.is_empty() {
        return;
    }
    open_multi_share_picker(state, source_chat_mid, items);
}

pub(super) fn open_multi_share_picker(
    state: &AppState,
    source_chat_mid: String,
    items: Vec<ShareSourceItem>,
) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let mut chats: Vec<ChatInfo> = {
        let cfg = state.config.borrow();
        state
            .chats
            .borrow()
            .iter()
            .filter(|c| !cfg.chat_hidden(&c.mid))
            .cloned()
            .collect()
    };
    if chats.is_empty() {
        toast(state, &crate::i18n::t("no_chats"));
        return;
    }
    sort_chats(&mut chats, &state.config.borrow());

    let window = gtk::Window::builder()
        .title(crate::i18n::t("share_choose_chats"))
        .transient_for(&state.window)
        .modal(true)
        .default_width(420)
        .default_height(540)
        .build();

    {
        let weak_win = window.downgrade();
        let esc = gtk::EventControllerKey::new();
        esc.set_propagation_phase(gtk::PropagationPhase::Capture);
        esc.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(w) = weak_win.upgrade() {
                    w.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(esc);
    }

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();

    let summary_text = if items.len() == 1 {
        crate::i18n::t("share_selected_summary_one")
    } else {
        crate::i18n::tf(
            "share_selected_summary",
            &[("n", &items.len().to_string())],
        )
    };
    let summary = gtk::Label::builder()
        .label(summary_text)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    root.append(&summary);

    let search = gtk::SearchEntry::builder()
        .placeholder_text(crate::i18n::t("file_share_search"))
        .build();
    root.append(&search);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    struct PickerRow {
        mid: String,
        lower_name: String,
        row: gtk::ListBoxRow,
        check: gtk::CheckButton,
    }

    let selected_mids: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let row_widgets: Rc<RefCell<Vec<PickerRow>>> = Rc::new(RefCell::new(Vec::new()));

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&crate::i18n::t("cancel"));
    let send = gtk::Button::with_label(&crate::i18n::t("send"));
    send.add_css_class("suggested-action");
    send.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&send);

    let update_send_btn = {
        let selected_mids = selected_mids.clone();
        let send = send.clone();
        Rc::new(move || {
            let count = selected_mids.borrow().len();
            let label = if count > 0 {
                crate::i18n::tf("share_send_count", &[("n", &count.to_string())])
            } else {
                crate::i18n::t("send")
            };
            send.set_label(&label);
            send.set_sensitive(count > 0);
        })
    };

    for chat in &chats {
        let row = gtk::ListBoxRow::builder()
            .activatable(true)
            .selectable(false)
            .build();
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(10)
            .margin_end(10)
            .build();

        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .css_classes(["line-msg-select-check"])
            .build();

        let avatar_frame = gtk::Box::builder()
            .width_request(36)
            .height_request(36)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["line-avatar-frame", "line-avatar-xs"])
            .build();
        let avatar = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .width_request(36)
            .height_request(36)
            .css_classes(["line-avatar", "line-avatar-xs"])
            .build();
        if let Some(path) = chat.avatar_path.as_deref()
            && std::path::Path::new(path).exists()
        {
            attach_texture_async(avatar.clone(), path.to_string(), 72);
        }
        avatar_frame.append(&avatar);

        let name = gtk::Label::builder()
            .label(&chat.name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["line-chat-name"])
            .build();

        hbox.append(&check);
        hbox.append(&avatar_frame);
        hbox.append(&name);
        row.set_child(Some(&hbox));
        list.append(&row);

        let mid = chat.mid.clone();
        let sel = selected_mids.clone();
        let update = update_send_btn.clone();
        check.connect_toggled(move |btn| {
            let active = btn.is_active();
            {
                let mut set = sel.borrow_mut();
                if set.contains(&mid) == active {
                    return;
                }
                if active {
                    set.insert(mid.clone());
                } else {
                    set.remove(&mid);
                }
            }
            update();
        });

        row_widgets.borrow_mut().push(PickerRow {
            mid: chat.mid.clone(),
            lower_name: chat.name.to_lowercase(),
            row,
            check,
        });
    }

    {
        let rows = row_widgets.clone();
        list.connect_row_activated(move |_, activated| {
            let idx = activated.index();
            if idx < 0 {
                return;
            }
            if let Some(entry) = rows.borrow().get(idx as usize) {
                entry.check.set_active(!entry.check.is_active());
            }
        });
    }

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    root.append(&scroll);

    let empty = gtk::Label::new(Some(&crate::i18n::t("file_share_no_match")));
    empty.set_visible(false);
    root.append(&empty);
    root.append(&buttons);
    window.set_child(Some(&root));

    {
        let rows = row_widgets.clone();
        let empty = empty.clone();
        search.connect_search_changed(move |entry| {
            let query = entry.text().trim().to_lowercase();
            let mut any_visible = false;
            for entry in rows.borrow().iter() {
                let show = query.is_empty() || entry.lower_name.contains(&query);
                entry.row.set_visible(show);
                if show {
                    any_visible = true;
                }
            }
            empty.set_visible(!any_visible);
        });
    }

    {
        let weak_win = window.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(w) = weak_win.upgrade() {
                w.close();
            }
        });
    }

    {
        let s = state.clone();
        let weak_win = window.downgrade();
        let rows = row_widgets.clone();
        let sel = selected_mids.clone();
        let sent = Cell::new(false);
        send.connect_clicked(move |button| {
            if sent.get() {
                return;
            }
            let target_mids: Vec<String> = {
                let set = sel.borrow();
                rows.borrow()
                    .iter()
                    .filter(|entry| set.contains(&entry.mid))
                    .map(|entry| entry.mid.clone())
                    .collect()
            };
            if target_mids.is_empty() {
                return;
            }
            sent.set(true);
            button.set_sensitive(false);
            if let Some(w) = weak_win.upgrade() {
                w.close();
            }
            exit_selection_mode(&s);
            if !*s.session_ready.borrow() {
                toast(&s, &crate::i18n::t("still_restoring"));
                return;
            }
            start_share_batch(&s, source_chat_mid.clone(), items.clone(), target_mids);
        });
    }

    window.present();
}

pub(super) fn start_share_batch(
    state: &AppState,
    source_chat_mid: String,
    items: Vec<ShareSourceItem>,
    target_chat_mids: Vec<String>,
) {
    if items.is_empty() || target_chat_mids.is_empty() {
        return;
    }
    let batch_id = {
        let mut next = state.next_share_batch_id.borrow_mut();
        *next = next.saturating_add(1);
        *next
    };
    state.share_batches.borrow_mut().insert(
        batch_id,
        ShareBatch {
            source_chat_mid,
            target_chat_mids,
            pending_prep: VecDeque::from(items),
            prepared: Vec::new(),
            send_queue: VecDeque::new(),
            sent_count: 0,
            failed_count: 0,
            last_error: None,
            queued_sends: false,
        },
    );
    toast(state, &crate::i18n::t("share_sending"));
    advance_batch(state, batch_id);
}

enum NextStep {
    PrepareDownload {
        source_chat_mid: String,
        message_id: String,
        o_type: ShareMediaType,
        suggest_name: String,
        duration_ms: Option<u64>,
    },
    SendItem {
        chat_mid: String,
        item: PreparedShareItem,
    },
    Done,
}

fn next_batch_step(state: &AppState, batch_id: u64) -> Option<NextStep> {
    loop {
        let mut batches = state.share_batches.borrow_mut();
        let batch = batches.get_mut(&batch_id)?;

        if let Some(item) = batch.pending_prep.pop_front() {
            match item.kind {
                ShareableKind::Text(text) => {
                    batch.prepared.push(PreparedShareItem::Text(text));
                    continue;
                }
                ShareableKind::Location(location) => {
                    batch.prepared.push(PreparedShareItem::Location(location));
                    continue;
                }
                ShareableKind::Media {
                    o_type,
                    duration_ms,
                } => {
                    let fresh = state
                        .media_msgs
                        .borrow()
                        .get(&item.msg.id)
                        .cloned()
                        .unwrap_or_else(|| item.msg.clone());
                    let needs_full =
                        matches!(o_type, ShareMediaType::Image | ShareMediaType::Video);
                    if let Some(path) = local_media_path(&fresh, needs_full) {
                        let file_name = resolve_share_filename(&fresh, None, &path);
                        batch.prepared.push(PreparedShareItem::Media {
                            o_type,
                            path,
                            file_name,
                            duration_ms,
                        });
                        continue;
                    }
                    return Some(NextStep::PrepareDownload {
                        source_chat_mid: batch.source_chat_mid.clone(),
                        message_id: fresh.id.clone(),
                        o_type,
                        suggest_name: suggest_media_name(&fresh),
                        duration_ms,
                    });
                }
            }
        }

        if !batch.queued_sends {
            batch.queued_sends = true;
            for target_mid in &batch.target_chat_mids {
                for idx in 0..batch.prepared.len() {
                    batch.send_queue.push_back((target_mid.clone(), idx));
                }
            }
        }

        if let Some((chat_mid, idx)) = batch.send_queue.pop_front() {
            if !*state.session_ready.borrow() {
                let remaining = 1 + batch.send_queue.len();
                batch.failed_count += remaining;
                batch.last_error = Some(crate::i18n::t("still_restoring"));
                batch.send_queue.clear();
                return Some(NextStep::Done);
            }
            if let Some(item) = batch.prepared.get(idx).cloned() {
                return Some(NextStep::SendItem { chat_mid, item });
            }
            continue;
        }

        return Some(NextStep::Done);
    }
}

fn record_batch_error(state: &AppState, batch_id: u64, count: usize, error: String) {
    if let Some(batch) = state.share_batches.borrow_mut().get_mut(&batch_id) {
        batch.failed_count += count;
        batch.last_error = Some(error);
    }
}

fn advance_batch(state: &AppState, batch_id: u64) {
    loop {
        let Some(step) = next_batch_step(state, batch_id) else {
            return;
        };
        match step {
            NextStep::PrepareDownload {
                source_chat_mid,
                message_id,
                o_type,
                suggest_name,
                duration_ms,
            } => match state.sidecar.download_media(&source_chat_mid, &message_id) {
                Ok(req_id) => {
                    state.pending.borrow_mut().insert(
                        req_id,
                        Pending::SharePrepareMedia {
                            batch_id,
                            message_id,
                            o_type,
                            suggest_name,
                            duration_ms,
                        },
                    );
                    return;
                }
                Err(error) => {
                    let targets = state
                        .share_batches
                        .borrow()
                        .get(&batch_id)
                        .map_or(1, |b| b.target_chat_mids.len());
                    record_batch_error(state, batch_id, targets, error.to_string());
                }
            },
            NextStep::SendItem { chat_mid, item } => {
                let req = match &item {
                    PreparedShareItem::Text(text) => state.sidecar.send_message(&chat_mid, text),
                    PreparedShareItem::Location(loc) => state.sidecar.send_location(&chat_mid, loc),
                    PreparedShareItem::Media {
                        o_type,
                        path,
                        file_name,
                        duration_ms,
                    } => state.sidecar.send_media_named(
                        &chat_mid,
                        path,
                        o_type.as_o_type(),
                        *duration_ms,
                        file_name,
                    ),
                };
                match req {
                    Ok(req_id) => {
                        state.pending.borrow_mut().insert(
                            req_id,
                            Pending::ShareSend {
                                batch_id,
                                chat_mid,
                            },
                        );
                        return;
                    }
                    Err(error) => {
                        record_batch_error(state, batch_id, 1, error.to_string());
                    }
                }
            }
            NextStep::Done => {
                finish_batch(state, batch_id);
                return;
            }
        }
    }
}

pub(super) fn on_media_prepared(
    state: &AppState,
    batch_id: u64,
    message_id: &str,
    o_type: ShareMediaType,
    suggest_name: &str,
    duration_ms: Option<u64>,
    result: &serde_json::Value,
) {
    let Some(path) = result.get("path").and_then(|v| v.as_str()) else {
        on_media_prepare_failed(state, batch_id, None);
        return;
    };
    let downloaded_name = result
        .get("fileName")
        .and_then(|v| v.as_str())
        .and_then(meaningful_file_name);
    let mut updated_msg = None;
    {
        let mut messages = state.media_msgs.borrow_mut();
        if let Some(meta) = messages.get_mut(message_id) {
            meta.file_path = Some(path.to_string());
            if let Some(name) = downloaded_name {
                meta.file_name = Some(name.to_string());
            }
            meta.file_size = result
                .get("bytes")
                .and_then(|v| v.as_u64())
                .or(meta.file_size);
            if o_type == ShareMediaType::Image {
                meta.image_path = Some(path.to_string());
            } else if o_type == ShareMediaType::Audio {
                meta.audio_path = Some(path.to_string());
            }
            updated_msg = Some(meta.clone());
        }
    }
    let file_name = if let Some(name) = downloaded_name {
        name.to_string()
    } else if let Some(meta) = updated_msg.as_ref() {
        resolve_share_filename(meta, None, path)
    } else if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        format!("{message_id}.{ext}")
    } else {
        suggest_name.to_string()
    };

    if let Some(batch) = state.share_batches.borrow_mut().get_mut(&batch_id) {
        batch.prepared.push(PreparedShareItem::Media {
            o_type,
            path: path.to_string(),
            file_name,
            duration_ms,
        });
    } else {
        return;
    }
    advance_batch(state, batch_id);
}

pub(super) fn on_media_prepare_failed(state: &AppState, batch_id: u64, error: Option<&str>) {
    let message = error
        .map(str::to_string)
        .unwrap_or_else(|| crate::i18n::t("media_download_failed"));
    let targets = state
        .share_batches
        .borrow()
        .get(&batch_id)
        .map_or(1, |b| b.target_chat_mids.len());
    record_batch_error(state, batch_id, targets, message);
    advance_batch(state, batch_id);
}

pub(super) fn on_item_sent(
    state: &AppState,
    batch_id: u64,
    chat_mid: &str,
    result: &serde_json::Value,
) {
    {
        let mut batches = state.share_batches.borrow_mut();
        let Some(batch) = batches.get_mut(&batch_id) else {
            return;
        };
        batch.sent_count += 1;
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid)
        && let Ok(msg) =
            serde_json::from_value::<MessageInfo>(result.get("message").cloned().unwrap_or_default())
    {
        set_msg_state(state, "list", None);
        append_message(state, &msg, true);
        pin_messages_to_latest(state);
    }
    advance_batch(state, batch_id);
}

pub(super) fn on_item_send_failed(state: &AppState, batch_id: u64, error: Option<&str>) {
    let message = error
        .map(str::to_string)
        .unwrap_or_else(|| "request failed".to_string());
    record_batch_error(state, batch_id, 1, message);
    advance_batch(state, batch_id);
}

fn finish_batch(state: &AppState, batch_id: u64) {
    let Some(batch) = state.share_batches.borrow_mut().remove(&batch_id) else {
        return;
    };
    let msg = if batch.failed_count == 0 && batch.sent_count > 0 {
        let chat_count = batch.target_chat_mids.len();
        if chat_count <= 1 {
            if batch.sent_count == 1 {
                crate::i18n::t("share_sent_one")
            } else {
                crate::i18n::tf(
                    "share_sent_many",
                    &[("count", &batch.sent_count.to_string())],
                )
            }
        } else if batch.prepared.len() == 1 {
            crate::i18n::tf(
                "share_sent_one_multi_chats",
                &[("chats", &chat_count.to_string())],
            )
        } else {
            crate::i18n::tf(
                "share_sent_multi_chats",
                &[
                    ("messages", &batch.prepared.len().to_string()),
                    ("chats", &chat_count.to_string()),
                ],
            )
        }
    } else if batch.sent_count > 0 {
        crate::i18n::tf(
            "share_partial",
            &[
                ("sent", &batch.sent_count.to_string()),
                ("failed", &batch.failed_count.to_string()),
            ],
        )
    } else {
        let err = batch
            .last_error
            .unwrap_or_else(|| crate::i18n::t("media_download_failed"));
        crate::i18n::tf("share_failed", &[("error", &err)])
    };
    toast(state, &msg);
}

pub(super) fn fail_pending_shares(state: &AppState) {
    state.pending.borrow_mut().retain(|_, pending| {
        !matches!(
            pending,
            Pending::SharePrepareMedia { .. } | Pending::ShareSend { .. }
        )
    });
    let had_batches = !state.share_batches.borrow().is_empty();
    state.share_batches.borrow_mut().clear();
    if had_batches {
        toast(
            state,
            &crate::i18n::tf("share_failed", &[("error", &crate::i18n::t("protocol_offline"))]),
        );
    }
}
