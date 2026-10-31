use super::*;
use crate::protocol::ReplyTarget;

pub(super) fn setup_reply_strip(state: &AppState) {
    state.reply_back.set_label(&crate::i18n::t("reply_back"));
    state.reply_back.set_visible(false);
    let s = state.clone();
    state.reply_back.connect_clicked(move |_| {
        let jump = s.reply_jump.borrow_mut().take();
        s.reply_back.set_visible(false);
        if let Some((chat, _, origin)) = jump {
            if s.current_chat.borrow().as_deref() == Some(&chat) {
                reveal_message(&s, &origin);
            }
        }
    });
    state
        .conversation
        .insert_child_after(&state.reply_back, Some(&state.msg_stack));
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
    let reaction_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    for (kind, icon) in [
        (2, "👍"),
        (3, "❤️"),
        (4, "😆"),
        (5, "😮"),
        (6, "😢"),
        (7, "😲"),
    ] {
        let selected = msg
            .reactions
            .iter()
            .any(|r| r.kind == kind && state.self_mid.borrow().as_deref() == Some(r.from.as_str()));
        let button = gtk::Button::with_label(icon);
        button.add_css_class("flat");
        if selected {
            button.add_css_class("suggested-action");
        }
        let s = state.clone();
        let id = msg.id.clone();
        let chat_mid = target.chat_mid.clone();
        let weak_menu = menu.downgrade();
        button.connect_clicked(move |_| {
            if let Some(menu) = weak_menu.upgrade() {
                menu.popdown();
            }
            match s
                .sidecar
                .react_message(&chat_mid, &id, if selected { 1 } else { kind })
            {
                Ok(request) => {
                    s.pending.borrow_mut().insert(
                        request,
                        Pending::React {
                            chat_mid: chat_mid.clone(),
                        },
                    );
                }
                Err(e) => toast(&s, &e.to_string()),
            }
        });
        reaction_row.append(&button);
    }
    actions.append(&reaction_row);
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
        let label = gtk::Label::builder()
            .label(format!("↪ {}\n{}", reply.sender, preview))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(38)
            .css_classes(["dim-label"])
            .build();
        let quote = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["line-reply-quote"])
            .build();
        let thumbnail = gtk::Picture::builder()
            .width_request(48)
            .height_request(48)
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .visible(false)
            .build();
        quote.append(&thumbnail);
        quote.append(&label);
        state
            .reply_thumbnails
            .borrow_mut()
            .entry(reply.id.clone())
            .or_default()
            .push(thumbnail.downgrade());
        let target_id = reply.id.clone();
        if let Some(original) = state.media_msgs.borrow().get(&target_id).cloned() {
            update_reply_thumbnail(state, &original);
        }
        let origin_id = msg.id.clone();
        let s = state.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let Some(chat) = s.current_chat.borrow().clone() else {
                return;
            };
            *s.reply_jump.borrow_mut() = Some((chat, target_id.clone(), origin_id.clone()));
            s.reply_back.set_visible(true);
            continue_reply_jump(&s);
        });
        quote.set_cursor_from_name(Some("pointer"));
        quote.add_controller(click);
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

fn reveal_message(state: &AppState, id: &str) -> bool {
    if let Some(row) = state
        .message_list
        .rows()
        .into_iter()
        .find(|row| row.has_css_class("line-history-message") && row.widget_name() == id)
    {
        *state.stick_bottom.borrow_mut() = false;
        state.message_list.reveal(&row);
        true
    } else {
        false
    }
}

pub(super) fn continue_reply_jump(state: &AppState) {
    let jump = state.reply_jump.borrow().clone();
    let Some((chat, target, _)) = jump else {
        return;
    };
    if state.current_chat.borrow().as_deref() != Some(&chat) {
        *state.reply_jump.borrow_mut() = None;
        state.reply_back.set_visible(false);
        return;
    }
    if target.is_empty() {
        return;
    }
    if reveal_message(state, &target) {
        if let Some(jump) = state.reply_jump.borrow_mut().as_mut() {
            jump.1.clear();
        }
        return;
    }
    let phase = {
        let h = state.history.borrow();
        match h.phase {
            history::LoadPhase::Ready if h.before.is_some() => 0,
            history::LoadPhase::Loading(_) => 1,
            _ => 2,
        }
    };
    match phase {
        0 => history::load(state),
        1 => {}
        _ => {
            toast(state, &crate::i18n::t("reply_not_found"));
        }
    }
}

pub(super) fn update_reply_thumbnail(state: &AppState, original: &MessageInfo) {
    if !original.content_type.eq_ignore_ascii_case("image") {
        return;
    }
    let path = original
        .image_path
        .as_ref()
        .filter(|p| std::path::Path::new(p).is_file())
        .cloned()
        .or_else(|| {
            state
                .media_ready_paths
                .borrow()
                .get(&original.id)
                .filter(|p| std::path::Path::new(p).is_file())
                .cloned()
        });
    let Some(path) = path else { return };
    if let Some(pictures) = state.reply_thumbnails.borrow_mut().get_mut(&original.id) {
        pictures.retain(|weak| {
            if let Some(picture) = weak.upgrade() {
                picture.set_visible(true);
                attach_texture_async(picture, path.clone(), 96);
                true
            } else {
                false
            }
        });
    }
}
