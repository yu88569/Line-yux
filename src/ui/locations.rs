use super::*;
use crate::protocol::LocationInfo;

#[derive(Clone)]
pub(super) struct LocationDialog {
    window: glib::WeakRef<gtk::Window>,
    send: glib::WeakRef<gtk::Button>,
    error: glib::WeakRef<gtk::Label>,
}

impl LocationDialog {
    pub(super) fn failed(&self, message: &str) {
        if let Some(send) = self.send.upgrade() {
            send.set_sensitive(true);
        }
        if let Some(error) = self.error.upgrade() {
            error.set_text(message);
        }
    }
    pub(super) fn succeeded(&self) {
        if let Some(window) = self.window.upgrade() {
            window.close();
        }
    }
}

fn location_from_entries(coordinates: &str, title: &str, address: &str) -> Option<LocationInfo> {
    let (latitude, longitude) = coordinates.split_once(',')?;
    let location = LocationInfo {
        latitude: latitude.trim().parse().ok()?,
        longitude: longitude.trim().parse().ok()?,
        title: title.trim().to_string(),
        address: address.trim().to_string(),
    };
    location.is_valid().then_some(location)
}

pub(super) fn append_location_card(bubble: &gtk::Box, location: Option<&LocationInfo>) {
    let title = gtk::Label::builder()
        .label(crate::i18n::t("location"))
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    bubble.append(&title);
    let Some(location) = location.filter(|location| location.is_valid()) else {
        let unavailable = gtk::Label::builder()
            .label(crate::i18n::t("location_unavailable"))
            .wrap(true)
            .xalign(0.0)
            .build();
        bubble.append(&unavailable);
        return;
    };
    for text in [
        &location.title,
        &location.address,
        &format!("{}, {}", location.latitude, location.longitude),
    ] {
        if text.is_empty() {
            continue;
        }
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .selectable(true)
            .xalign(0.0)
            .max_width_chars(42)
            .build();
        bubble.append(&label);
    }
    if let Some(url) = location.maps_url() {
        let link = gtk::LinkButton::with_label(&url, &crate::i18n::t("open_google_maps"));
        link.set_halign(gtk::Align::Start);
        bubble.append(&link);
    }
}

pub(super) fn fail_pending_locations(state: &AppState) {
    let ids: Vec<_> = state
        .pending
        .borrow()
        .iter()
        .filter_map(|(id, pending)| matches!(pending, Pending::LocationSend { .. }).then_some(*id))
        .collect();
    for id in ids {
        if let Some(Pending::LocationSend { ui, .. }) = state.pending.borrow_mut().remove(&id) {
            ui.failed(&crate::i18n::t("protocol_offline"));
        }
    }
}

pub(super) fn open_location_dialog(state: &AppState) {
    if block_reply_attachment(state) {
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let account = state.self_mid.borrow().clone();
    let window = gtk::Window::builder()
        .application(&state.app)
        .transient_for(&state.window)
        .modal(true)
        .title(crate::i18n::t("share_location"))
        .default_width(420)
        .resizable(false)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .margin_bottom(20)
        .build();
    let coordinates = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("location_coordinates"))
        .build();
    let title = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("location_title"))
        .build();
    let address = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("location_address"))
        .build();
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("location_hint"))
        .wrap(true)
        .xalign(0.0)
        .build();
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .build();
    let preview = gtk::LinkButton::with_label("", &crate::i18n::t("open_google_maps"));
    preview.set_sensitive(false);
    let send = gtk::Button::with_label(&crate::i18n::t("send"));
    send.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&crate::i18n::t("cancel"));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    actions.append(&cancel);
    actions.append(&send);
    for child in [
        hint.upcast_ref::<gtk::Widget>(),
        coordinates.upcast_ref(),
        title.upcast_ref(),
        address.upcast_ref(),
        preview.upcast_ref(),
        error.upcast_ref(),
        actions.upcast_ref(),
    ] {
        root.append(child);
    }
    window.set_child(Some(&root));
    coordinates.connect_changed({
        let preview = preview.downgrade();
        move |entry| {
            if let Some(preview) = preview.upgrade() {
                if let Some(url) =
                    location_from_entries(&entry.text(), "", "").and_then(|l| l.maps_url())
                {
                    preview.set_uri(&url);
                    preview.set_sensitive(true);
                } else {
                    preview.set_sensitive(false);
                }
            }
        }
    });
    cancel.connect_clicked({
        let window = window.downgrade();
        move |_| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        }
    });
    let ui = LocationDialog {
        window: window.downgrade(),
        send: send.downgrade(),
        error: error.downgrade(),
    };
    send.connect_clicked({
        let state = state.clone();
        move |button| {
            if block_reply_attachment(&state) {
                return;
            }
            if *state.self_mid.borrow() != account {
                ui.succeeded();
                return;
            }
            if !*state.session_ready.borrow() {
                ui.failed(&crate::i18n::t("still_restoring"));
                return;
            }
            let Some(location) =
                location_from_entries(&coordinates.text(), &title.text(), &address.text())
            else {
                ui.failed(&crate::i18n::t("location_invalid"));
                return;
            };
            match state.sidecar.send_location(&chat_mid, &location) {
                Ok(id) => {
                    button.set_sensitive(false);
                    if let Some(error) = ui.error.upgrade() {
                        error.set_text("");
                    }
                    state.pending.borrow_mut().insert(
                        id,
                        Pending::LocationSend {
                            chat_mid: chat_mid.clone(),
                            ui: ui.clone(),
                        },
                    );
                }
                Err(error) => ui.failed(&error.to_string()),
            }
        }
    });
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_exact_point_and_rejects_invalid_coordinates() {
        let point = location_from_entries("0, -120.123456789", " Place ", "Address").unwrap();
        assert_eq!(point.title, "Place");
        assert_eq!(
            point.maps_url().unwrap(),
            "https://www.google.com/maps/search/?api=1&query=0%2C-120.123456789"
        );
        for input in ["", "1", "91,0", "0,181", "NaN,0", "0,inf", "0,0,0"] {
            assert!(location_from_entries(input, "", "").is_none(), "{input}");
        }
    }
}
