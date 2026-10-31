use super::*;
use crate::protocol::HistoryCursor;

#[derive(Default, PartialEq)]
pub(super) enum LoadPhase {
    #[default]
    Ready,
    Loading(u64),
    Failed,
    Exhausted,
}

#[derive(Default)]
pub(super) struct ChatHistory {
    pub chat_mid: String,
    pub messages: Vec<MessageInfo>,
    pub before: Option<HistoryCursor>,
    pub phase: LoadPhase,
    pub generation: u64,
    pub rendering: bool,
    anchor: Option<(gtk::Widget, f32)>,
    pub status: Option<gtk::ListBoxRow>,
}

pub(super) fn reset(state: &AppState, mid: &str) {
    let generation = state.history.borrow().generation + 1;
    *state.history.borrow_mut() = ChatHistory {
        chat_mid: mid.into(),
        generation,
        ..Default::default()
    };
}

pub(super) fn ensure_chat(state: &AppState) {
    let mid = state.current_chat.borrow().clone().unwrap_or_default();
    if state.history.borrow().chat_mid != mid {
        reset(state, &mid);
    }
}

pub(super) fn interrupted(state: &AppState) {
    {
        let mut history = state.history.borrow_mut();
        history.generation += 1;
        history.rendering = false;
        history.anchor = None;
        if matches!(history.phase, LoadPhase::Loading(_)) {
            history.phase = LoadPhase::Failed;
        }
    }
    state
        .pending
        .borrow_mut()
        .retain(|_, pending| !matches!(pending, Pending::FetchOlder { .. }));
    update_status(state);
}

pub(super) fn observe(state: &AppState, message: &MessageInfo) {
    ensure_chat(state);
    let mut history = state.history.borrow_mut();
    if let Some(old) = history.messages.iter_mut().find(|m| m.id == message.id) {
        *old = message.clone();
    } else {
        history.messages.push(message.clone());
    }
    let oldest = history
        .messages
        .iter()
        .filter(|m| m.history_cursor.is_some())
        .min_by_key(|m| {
            let cursor = m.history_cursor.as_ref().unwrap();
            (
                cursor.delivered_time.parse::<u128>().unwrap_or(u128::MAX),
                cursor.message_id.parse::<u128>().unwrap_or(u128::MAX),
            )
        });
    if let Some(oldest) = oldest {
        history.before = oldest.history_cursor.clone();
    }
}

pub(super) fn merge_current(state: &AppState, incoming: Vec<MessageInfo>) -> Vec<MessageInfo> {
    let mut merged = state.history.borrow().messages.clone();
    for message in incoming {
        if let Some(old) = merged.iter_mut().find(|m| m.id == message.id) {
            *old = message;
        } else {
            merged.push(message);
        }
    }
    merged
}

pub(super) fn release_anchor(state: &AppState) {
    state.history.borrow_mut().anchor = None;
}

fn capture_anchor(state: &AppState) -> Option<(gtk::Widget, f32)> {
    state
        .message_list
        .rows()
        .into_iter()
        .filter(|row| row.is_mapped() && row.has_css_class("line-history-message"))
        .find_map(|row| {
            let bounds = row.compute_bounds(&state.message_scroll)?;
            (bounds.y() + bounds.height() > 0.0
                && bounds.y() < state.message_scroll.height() as f32)
                .then_some((row, bounds.y()))
        })
}

pub(super) fn restore_anchor(state: &AppState) {
    let anchor = state.history.borrow().anchor.clone();
    let Some((row, offset)) = anchor else {
        return;
    };
    if !row.is_mapped() {
        state.message_list.reveal(&row);
        return;
    }
    let Some(bounds) = row.compute_bounds(&state.message_scroll) else {
        return;
    };
    let delta = f64::from(bounds.y() - offset);
    if delta.abs() > 0.5 {
        let pinning = *state.scroll_pinning.borrow();
        *state.scroll_pinning.borrow_mut() = true;
        let adjustment = state.message_scroll.vadjustment();
        adjustment.set_value(adjustment.value() + delta);
        *state.scroll_pinning.borrow_mut() = pinning;
    }
}

pub(super) fn update_status(state: &AppState) {
    let previous = state.history.borrow_mut().status.take();
    if let Some(row) = previous {
        state.message_list.remove(&row);
    }
    let history = state.history.borrow();
    if history.before.is_none() {
        return;
    }
    let key = match history.phase {
        LoadPhase::Ready => "history_load",
        LoadPhase::Loading(_) => "history_loading",
        LoadPhase::Failed => "history_retry",
        LoadPhase::Exhausted => "history_end",
    };
    let button = gtk::Button::with_label(&crate::i18n::t(key));
    button.add_css_class("flat");
    button.set_sensitive(matches!(
        history.phase,
        LoadPhase::Ready | LoadPhase::Failed
    ));
    let row = gtk::ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .child(&button)
        .build();
    let state_c = state.clone();
    button.connect_clicked(move |_| load(&state_c));
    drop(history);
    state
        .message_list
        .prepend_with(|| state.message_list.append(&row));
    state.history.borrow_mut().status = Some(row);
}

pub(super) fn maybe_load(state: &AppState) {
    if *state.scroll_pinning.borrow()
        || state.history.borrow().rendering
        || *state.stick_bottom.borrow()
    {
        return;
    }
    if state.message_scroll.vadjustment().value() <= 48.0
        && matches!(state.history.borrow().phase, LoadPhase::Ready)
    {
        load(state);
    }
}

pub(super) fn load(state: &AppState) {
    if state.history.borrow().rendering
        || !matches!(
            state.history.borrow().phase,
            LoadPhase::Ready | LoadPhase::Failed
        )
    {
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let Some(before) = state.history.borrow().before.clone() else {
        return;
    };
    *state.stick_bottom.borrow_mut() = false;
    match state.sidecar.fetch_older_messages(&chat_mid, &before) {
        Ok(request) => {
            state.history.borrow_mut().phase = LoadPhase::Loading(request);
            state
                .pending
                .borrow_mut()
                .insert(request, Pending::FetchOlder { chat_mid });
        }
        Err(_) => state.history.borrow_mut().phase = LoadPhase::Failed,
    }
    update_status(state);
}

pub(super) fn failed(state: &AppState, id: u64, chat_mid: &str) {
    if state.current_chat.borrow().as_deref() != Some(chat_mid)
        || state.history.borrow().phase != LoadPhase::Loading(id)
    {
        return;
    }
    state.history.borrow_mut().phase = LoadPhase::Failed;
    update_status(state);
}

pub(super) fn received(state: &AppState, id: u64, chat_mid: &str, result: &serde_json::Value) {
    if state.current_chat.borrow().as_deref() != Some(chat_mid)
        || state.history.borrow().phase != LoadPhase::Loading(id)
    {
        return;
    }
    if state.history.borrow().rendering {
        let state = state.clone();
        let chat_mid = chat_mid.to_string();
        let result = result.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(16), move || {
            received(&state, id, &chat_mid, &result)
        });
        return;
    }
    let messages: Vec<MessageInfo> = result
        .get("messages")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let page: Vec<_> = messages
        .into_iter()
        .filter(|m| !state.seen_msg_ids.borrow().contains(&m.id))
        .collect();
    let old_day = state
        .history
        .borrow()
        .messages
        .iter()
        .filter(|m| m.created_time > 0)
        .min_by_key(|m| m.created_time)
        .map(|m| day_key(m.created_time));
    let anchor = capture_anchor(state);
    state.history.borrow_mut().anchor = anchor;
    let old_status = state.history.borrow_mut().status.take();
    if let Some(row) = old_status {
        state.message_list.remove(&row);
    }
    if let Some(last) = page.iter().max_by_key(|m| m.created_time)
        && old_day.as_deref() == Some(day_key(last.created_time).as_str())
        && let Some(first) = state.message_list.first_child()
        && first.has_css_class("line-day-sep-row")
    {
        state.message_list.remove(&first);
    }
    let previous_day = state.last_msg_day.borrow_mut().take();
    let previous_incoming = state.last_incoming_id.borrow().clone();
    *state.scroll_pinning.borrow_mut() = true;
    let mut page = page;
    page.sort_by_key(|m| m.created_time);
    state.message_list.prepend_with(|| {
        for message in &page {
            append_message(state, message, false);
        }
    });
    *state.last_msg_day.borrow_mut() = previous_day;
    *state.last_incoming_id.borrow_mut() = previous_incoming;
    {
        let mut history = state.history.borrow_mut();
        history.before = result
            .get("before")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .or(history.before.clone());
        history.phase = if result
            .get("hasMore")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            LoadPhase::Ready
        } else {
            LoadPhase::Exhausted
        };
    }
    update_status(state);
    *state.scroll_pinning.borrow_mut() = false;
    let state_c = state.clone();
    let generation = state.history.borrow().generation;
    let frames = std::cell::Cell::new(0);
    state.message_list.widget().add_tick_callback(move |_, _| {
        if state_c.history.borrow().generation != generation {
            return glib::ControlFlow::Break;
        }
        restore_anchor(&state_c);
        frames.set(frames.get() + 1);
        if frames.get() >= 30 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    state.chat_subtitle.set_text(&crate::i18n::tf(
        "messages_count",
        &[("n", &state.history.borrow().messages.len().to_string())],
    ));
}
