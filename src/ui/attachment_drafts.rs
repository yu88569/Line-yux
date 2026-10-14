use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DraftStatus {
    Queued,
    Sending,
    Failed,
}

#[derive(Clone, Debug)]
pub(super) struct DraftAttachment {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub o_type: &'static str,
    pub duration_ms: Option<u64>,
    pub status: DraftStatus,
}

pub(super) fn stage_attachment(state: &AppState, chat: &str, path: PathBuf) -> bool {
    let result = (|| -> anyhow::Result<DraftAttachment> {
        let metadata = path.metadata()?;
        anyhow::ensure!(metadata.is_file(), "not a file");
        let id = {
            let mut next = state.next_attachment_id.borrow_mut();
            *next += 1;
            *next
        };
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let o_type = guess_media_o_type(&path);
        let duration_ms = if matches!(o_type, "audio" | "video") {
            ffprobe_duration_ms(&path)
        } else {
            None
        };
        let directory = state.data_dir.join("cache/media");
        std::fs::create_dir_all(&directory)?;
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| ext.len() <= 16 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("bin");
        let cached = directory.join(format!(
            "draft-{}-{id}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::copy(&path, &cached)?;
        Ok(DraftAttachment {
            id,
            path: cached,
            name,
            size: metadata.len(),
            o_type,
            duration_ms,
            status: DraftStatus::Queued,
        })
    })();
    match result {
        Ok(item) => {
            state
                .attachment_drafts
                .borrow_mut()
                .entry(chat.to_string())
                .or_default()
                .push(item);
            refresh_attachment_drafts(state);
            true
        }
        Err(error) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &error.to_string())]),
            );
            false
        }
    }
}

pub(super) fn remove_attachment(state: &AppState, chat: &str, id: u64) {
    if let Some(items) = state.attachment_drafts.borrow_mut().get_mut(chat) {
        items.retain(|item| {
            if item.id == id && item.status != DraftStatus::Sending {
                let _ = std::fs::remove_file(&item.path);
                false
            } else {
                true
            }
        });
    }
    refresh_attachment_drafts(state);
}

pub(super) fn refresh_attachment_drafts(state: &AppState) {
    while let Some(child) = state.attachment_strip.first_child() {
        state.attachment_strip.remove(&child);
    }
    let chat = state.current_chat.borrow().clone().unwrap_or_default();
    let items = state
        .attachment_drafts
        .borrow()
        .get(&chat)
        .cloned()
        .unwrap_or_default();
    state.attachment_scroll.set_visible(!items.is_empty());
    state
        .send_btn
        .set_sensitive(!items.iter().any(|item| item.status == DraftStatus::Sending));
    for item in items {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(8)
            .css_classes(["card"])
            .hexpand(false)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        card.set_size_request(180, 145);
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(crate::i18n::t("attachment_remove"))
            .halign(gtk::Align::End)
            .css_classes(["flat"])
            .sensitive(item.status != DraftStatus::Sending)
            .build();
        let s = state.clone();
        let chat = chat.clone();
        let id = item.id;
        remove.connect_clicked(move |_| remove_attachment(&s, &chat, id));
        card.append(&remove);
        if matches!(item.o_type, "image" | "gif") {
            let image = gtk::Picture::new();
            if let Ok(preview) = Pixbuf::from_file_at_scale(&item.path, 164, 80, true) {
                image.set_paintable(Some(&gdk::Texture::for_pixbuf(&preview)));
            }
            image.set_content_fit(gtk::ContentFit::Contain);
            image.set_can_shrink(true);
            image.set_size_request(164, 80);
            card.append(&image);
        } else {
            let icon = gio::content_type_guess(Some(&item.name), None::<&[u8]>).0;
            let image = gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(&icon));
            image.set_pixel_size(48);
            image.set_size_request(164, 80);
            card.append(&image);
        }
        let label = gtk::Label::builder()
            .label(&item.name)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(18)
            .tooltip_text(&item.name)
            .build();
        card.append(&label);
        let status = match item.status {
            DraftStatus::Queued => format_bytes(item.size),
            DraftStatus::Sending => crate::i18n::t("attachment_sending"),
            DraftStatus::Failed => crate::i18n::t("attachment_retry"),
        };
        card.append(
            &gtk::Label::builder()
                .label(status)
                .css_classes(["dim-label"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(18)
                .build(),
        );
        state.attachment_strip.append(&card);
    }
}

pub(super) fn attachment_batch_pending(state: &AppState, chat: &str) -> bool {
    state
        .attachment_drafts
        .borrow()
        .get(chat)
        .is_some_and(|items| items.iter().any(|item| item.status == DraftStatus::Sending))
}

pub(super) fn confirm_attachments(state: &AppState, chat: &str) {
    let items = {
        let mut drafts = state.attachment_drafts.borrow_mut();
        let Some(items) = drafts.get_mut(chat) else {
            return;
        };
        if items.iter().any(|item| item.status == DraftStatus::Sending) {
            return;
        }
        for item in items.iter_mut() {
            item.status = DraftStatus::Sending;
        }
        items.clone()
    };
    for item in items {
        let path = item.path.to_string_lossy().to_string();
        match state
            .sidecar
            .send_media_named(chat, &path, item.o_type, item.duration_ms, &item.name)
        {
            Ok(request) => {
                state
                    .attachment_requests
                    .borrow_mut()
                    .insert(request, (chat.to_string(), item.id));
                let (content_type, text) = match item.o_type {
                    "image" | "gif" => ("IMAGE", "[Image]"),
                    "video" => ("VIDEO", "[Video]"),
                    "audio" => ("AUDIO", "Voice message"),
                    _ => ("FILE", item.name.as_str()),
                };
                begin_optimistic_send(
                    state,
                    request,
                    chat,
                    MessageInfo {
                        id: String::new(),
                        text: text.into(),
                        from: String::new(),
                        to: chat.into(),
                        mine: true,
                        created_time: now_ms(),
                        content_type: content_type.into(),
                        image_path: matches!(item.o_type, "image" | "gif").then(|| path.clone()),
                        audio_path: (item.o_type == "audio").then(|| path.clone()),
                        file_name: Some(item.name),
                        file_size: Some(item.size),
                        file_path: Some(path),
                        duration_ms: item.duration_ms.map(|v| v as i64),
                        flex: None,
                    },
                );
                show_upload_progress(state, 0.02, &crate::i18n::t("media_uploading"));
            }
            Err(error) => {
                if let Some(items) = state.attachment_drafts.borrow_mut().get_mut(chat) {
                    if let Some(draft) = items.iter_mut().find(|draft| draft.id == item.id) {
                        draft.status = DraftStatus::Failed;
                    }
                }
                toast(state, &error.to_string());
            }
        }
    }
    refresh_attachment_drafts(state);
}

pub(super) fn settle_attachment(state: &AppState, request: u64, success: bool) {
    let Some((chat, id)) = state.attachment_requests.borrow_mut().remove(&request) else {
        return;
    };
    if let Some(items) = state.attachment_drafts.borrow_mut().get_mut(&chat) {
        if success {
            items.retain(|item| item.id != id);
        } else if let Some(item) = items.iter_mut().find(|item| item.id == id) {
            item.status = DraftStatus::Failed;
        }
    }
    refresh_attachment_drafts(state);
}

pub(super) fn restore_attachment_requests(state: &AppState) {
    let requests: Vec<_> = state.attachment_requests.borrow().keys().copied().collect();
    for request in requests {
        settle_attachment(state, request, false);
        if let Some(Pending::Send { placeholder_id, .. }) =
            state.pending.borrow_mut().remove(&request)
        {
            remove_pending_placeholder(state, &placeholder_id);
        }
    }
}

pub(super) fn invalidate_attachment_session(state: &AppState) {
    *state.attachment_epoch.borrow_mut() += 1;
    restore_attachment_requests(state);
}
