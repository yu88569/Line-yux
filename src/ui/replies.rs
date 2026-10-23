use super::*;
use crate::protocol::ReplyTarget;

pub(super) fn setup_reply_strip(state: &AppState) {
    state.reply_strip.add_css_class("card");
    state.reply_strip.set_margin_start(12);
    state.reply_strip.set_margin_end(12);
    state.reply_label.set_xalign(0.0);
    state.reply_label.set_hexpand(true);
    state
        .reply_label
        .set_ellipsize(gtk::pango::EllipsizeMode::End);
    state.reply_strip.append(&state.reply_label);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_tooltip_text(Some(&crate::i18n::t("cancel_reply")));
    let s = state.clone();
    close.connect_clicked(move |_| clear_reply(&s));
    state.reply_strip.append(&close);
    state
        .conversation
        .insert_child_after(&state.reply_strip, Some(&state.attachment_scroll));
    state.reply_strip.set_visible(false);
}

pub(super) fn clear_reply(state: &AppState) {
    set_reply(state, None);
}

pub(super) fn set_reply(state: &AppState, reply: Option<ReplyTarget>) {
    let reply = reply.filter(|r| state.current_chat.borrow().as_deref() == Some(&r.chat_mid));
    if let Some(r) = &reply {
        state.reply_label.set_text(&format!(
            "{} · {}\n{}",
            crate::i18n::t("reply"),
            r.sender,
            r.preview
        ));
    }
    state.reply_strip.set_visible(reply.is_some());
    *state.reply_target.borrow_mut() = reply;
}

pub(super) fn reply_target(state: &AppState, msg: &MessageInfo) -> Option<ReplyTarget> {
    if msg.id.is_empty() || !msg.id.bytes().all(|b| b.is_ascii_digit()) || msg.chat_mid().is_empty()
    {
        return None;
    }
    Some(ReplyTarget {
        chat_mid: msg.chat_mid().into(),
        id: msg.id.clone(),
        sender: reply_sender(state, msg),
        preview: preview_body_ui(msg).chars().take(160).collect(),
    })
}

pub(super) fn attach_reply_menu(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo) {
    let Some(target) = reply_target(state, msg) else {
        return;
    };
    let menu = gtk::Popover::new();
    menu.set_parent(bubble);
    menu.set_has_arrow(false);
    let action = gtk::Button::with_label(&crate::i18n::t("reply"));
    action.add_css_class("flat");
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
    actions.append(&action);
    if !msg.text.is_empty() {
        let copy = gtk::Button::with_label(&crate::i18n::t("qr_copy"));
        copy.add_css_class("flat");
        let text = msg.text.clone();
        let weak_menu = menu.downgrade();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&text);
            if let Some(menu) = weak_menu.upgrade() {
                menu.popdown();
            }
        });
        actions.append(&copy);
    }
    menu.set_child(Some(&actions));
    let s = state.clone();
    let weak_menu = menu.downgrade();
    action.connect_clicked(move |_| {
        set_reply(&s, Some(target.clone()));
        if let Some(menu) = weak_menu.upgrade() {
            menu.popdown();
        }
        s.composer.grab_focus();
    });
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_menu = menu.downgrade();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(menu) = weak_menu.upgrade() {
            menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.popup();
        }
    });
    bubble.add_controller(gesture);
    bubble.connect_destroy(move |_| menu.unparent());
}

pub(super) fn append_reply_quote(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo) {
    if let Some(reply) = msg.reply.as_deref() {
        let context = state.reply_context.borrow();
        let reply = if reply.preview.is_empty() {
            context.get(&reply.id).unwrap_or(reply)
        } else {
            reply
        };
        let preview = if reply.preview.is_empty() {
            crate::i18n::t("reply_message")
        } else {
            reply.preview.clone()
        };
        let quote = gtk::Label::builder()
            .label(format!("↪ {}\n{}", reply.sender, preview))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(38)
            .css_classes(["dim-label", "line-reply-quote"])
            .build();
        bubble.append(&quote);
    }
}

pub(super) fn block_reply_attachment(state: &AppState) -> bool {
    if state.reply_target.borrow().is_some() {
        toast(state, &crate::i18n::t("reply_text_only"));
        true
    } else {
        false
    }
}

pub(super) fn reply_sender(state: &AppState, msg: &MessageInfo) -> String {
    if msg.mine {
        return state.self_display_name.borrow().clone();
    }
    state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == msg.from)
        .map(|c| c.name.clone())
        .unwrap_or_default()
}
