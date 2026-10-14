use super::*;

#[derive(Default)]
pub(super) struct SenderAvatars {
    slots: HashMap<String, Vec<glib::WeakRef<gtk::Picture>>>,
    paths: HashMap<String, String>,
    requested: HashSet<String>,
}

impl SenderAvatars {
    pub fn clear_rows(&mut self) {
        self.slots.clear();
        self.requested.clear();
    }

    pub fn ready(&mut self, sender: &str, path: &str) {
        if !std::path::Path::new(path).is_file() {
            return;
        }
        self.paths.insert(sender.to_owned(), path.to_owned());
        if let Some(slots) = self.slots.get_mut(sender) {
            slots.retain(|slot| {
                if let Some(picture) = slot.upgrade() {
                    load_avatar(&picture, path.to_owned());
                    true
                } else {
                    false
                }
            });
        }
    }
}

fn load_avatar(picture: &gtk::Picture, path: String) {
    let weak = picture.downgrade();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::sticker_anim::load_scaled(&path, 80, false));
    });
    glib::spawn_future_local(async move {
        if let Ok(Some(frames)) = rx.recv().await
            && let Some(picture) = weak.upgrade()
        {
            apply_frames_to_picture(&picture, frames, false);
        }
    });
}

pub(super) fn sender_avatar(state: &AppState, sender: &str) -> gtk::Overlay {
    let frame = gtk::Overlay::builder()
        .width_request(40)
        .height_request(40)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .css_classes(["line-avatar-frame", "line-avatar-sm", "line-message-avatar"])
        .build();
    frame.set_overflow(gtk::Overflow::Hidden);
    let fallback = gtk::Image::builder()
        .icon_name("avatar-default-symbolic")
        .pixel_size(26)
        .width_request(40)
        .height_request(40)
        .build();
    frame.set_child(Some(&fallback));
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(40)
        .height_request(40)
        .css_classes(["line-avatar", "line-avatar-sm"])
        .build();
    frame.add_overlay(&picture);
    if sender.len() != 33
        || !sender.starts_with('u')
        || !sender[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return frame;
    }
    let mut avatars = state.sender_avatars.borrow_mut();
    avatars
        .slots
        .entry(sender.to_owned())
        .or_default()
        .push(picture.downgrade());
    let path = avatars
        .paths
        .get(sender)
        .cloned()
        .or_else(|| {
            let cached = state
                .data_dir
                .join("cache/avatars")
                .join(format!("{sender}.jpg"));
            if cached.is_file() {
                Some(cached.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .or_else(|| {
            state
                .chats
                .borrow()
                .iter()
                .find(|chat| chat.mid == sender)
                .and_then(|chat| chat.avatar_path.clone())
        })
        .or_else(|| {
            state.friends_ui.borrow().as_ref().and_then(|ui| {
                ui.friends
                    .borrow()
                    .iter()
                    .find(|friend| friend.mid == sender)
                    .and_then(|friend| friend.avatar_path.clone())
            })
        });
    if let Some(path) = path.filter(|path| std::path::Path::new(path).is_file()) {
        avatars.paths.insert(sender.to_owned(), path.clone());
        load_avatar(&picture, path);
    } else if !sender.is_empty() && avatars.requested.insert(sender.to_owned()) {
        let _ = state.sidecar.request(
            "resolve_sender_avatar",
            Some(serde_json::json!({"senderMid": sender})),
        );
    }
    frame
}
