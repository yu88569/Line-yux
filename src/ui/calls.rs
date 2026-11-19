use super::*;

pub(super) fn refresh_call_controls(state: &AppState) {
    let unlocked = state.config.borrow().experimental_calls;
    if !unlocked {
        for btn in [&state.call_btn, &state.video_call_btn] {
            btn.set_visible(false);
            btn.set_sensitive(false);
            btn.remove_css_class("line-call-locked");
            btn.set_tooltip_text(None);
        }
        return;
    }
    state.call_btn.set_visible(true);
    state.call_btn.remove_css_class("line-call-locked");
    state.video_call_btn.set_visible(true);
    state.video_call_btn.remove_css_class("line-call-locked");
    let can_call = state
        .current_chat
        .borrow()
        .as_ref()
        .and_then(|mid| {
            state
                .chats
                .borrow()
                .iter()
                .find(|c| &c.mid == mid)
                .map(|c| c.mid.starts_with('u') && c.kind != "bot")
        })
        .unwrap_or(false);
    state.call_btn.set_sensitive(can_call);
    state.call_btn.set_tooltip_text(Some(&if can_call {
        format!(
            "{}: {}",
            crate::i18n::t("voice_call"),
            crate::i18n::t("voice_call_note")
        )
    } else {
        crate::i18n::t("voice_call_unavailable")
    }));
    state.video_call_btn.set_sensitive(can_call);
    state.video_call_btn.set_tooltip_text(Some(&if can_call {
        format!(
            "{}: {}",
            crate::i18n::t("video_call"),
            crate::i18n::t("voice_call_note")
        )
    } else {
        crate::i18n::t("video_call_unavailable")
    }));
}

pub(super) fn calls_experimental_enabled(state: &AppState) -> bool {
    state.config.borrow().experimental_calls
}

pub(super) fn call_error_toast(state: &AppState, err: &str) {
    let msg = if err.contains("relogin_android_required") {
        crate::i18n::t("relogin_android_required")
    } else {
        crate::i18n::tf("call_failed", &[("error", err)])
    };
    toast(state, &msg);
}

pub(super) fn open_voice_call_dialog(state: &AppState) {
    open_call_dialog(state, "AUDIO");
}

pub(super) fn open_video_call_dialog(state: &AppState) {
    open_call_dialog(state, "VIDEO");
}

fn open_call_dialog(state: &AppState, kind: &str) {
    let is_video = kind == "VIDEO";
    if !calls_experimental_enabled(state) {
        toast(state, &crate::i18n::t("call_experimental_toast"));
        return;
    }
    let Some(mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !mid.starts_with('u') {
        toast(
            state,
            &crate::i18n::t(if is_video {
                "video_call_unavailable"
            } else {
                "voice_call_unavailable"
            }),
        );
        return;
    }
    if state.active_call_peer.borrow().is_some() || state.incoming_call_from.borrow().is_some() {
        toast(state, &crate::i18n::t("call_already_active"));
        if let Some(ui) = state.call_ui.borrow().as_ref() {
            ui.window.present();
        }
        return;
    }
    *state.pending_call_peer.borrow_mut() = Some(mid.clone());
    *state.pending_call_kind.borrow_mut() = kind.to_string();
    let name = peer_display_name(state, &mid);
    let ready_msg = crate::i18n::tf(
        if is_video {
            "call_video_ready"
        } else {
            "call_ready"
        },
        &[("name", &name)],
    );
    ensure_call_window(state, &name, kind, CallMode::Ready(&ready_msg));
    if is_video {
        let (video_input, mirror) = {
            let cfg = state.config.borrow();
            (cfg.video_input.clone(), cfg.call_video_mirror)
        };
        if let Some(ui) = state.call_ui.borrow().as_ref() {
            call_window::start_ready_preview(ui, &video_input, mirror);
        }
    }
}

pub(super) fn start_voice_call(state: &AppState) {
    if !calls_experimental_enabled(state) {
        toast(state, &crate::i18n::t("call_experimental_toast"));
        close_active_call_ui(state);
        return;
    }
    if state.active_call_peer.borrow().is_some() || state.incoming_call_from.borrow().is_some() {
        toast(state, &crate::i18n::t("call_already_active"));
        return;
    }
    let Some(mid) = state.pending_call_peer.borrow_mut().take() else {
        return;
    };
    let kind = state.pending_call_kind.borrow().clone();
    let is_video = kind == "VIDEO";
    if !mid.starts_with('u') {
        toast(
            state,
            &crate::i18n::t(if is_video {
                "video_call_unavailable"
            } else {
                "voice_call_unavailable"
            }),
        );
        close_active_call_ui(state);
        return;
    }
    // Release the V4L2 camera synchronously BEFORE Deno opens it for the call.
    if let Some(ui) = state.call_ui.borrow().as_ref() {
        call_window::stop_ready_preview(ui);
    }
    let (audio_in, audio_out, video_in, mirror, mic_vol, spk_vol) = {
        let cfg = state.config.borrow();
        (
            cfg.audio_input.clone(),
            cfg.audio_output.clone(),
            cfg.video_input.clone(),
            cfg.call_video_mirror,
            cfg.call_mic_volume,
            cfg.call_spk_volume,
        )
    };
    match state.sidecar.call_start(
        &mid, &kind, &audio_in, &audio_out, &video_in, mirror, mic_vol, spk_vol,
    ) {
        Ok(_) => {
            *state.active_call_peer.borrow_mut() = Some(mid.clone());
            *state.active_call_kind.borrow_mut() = kind.clone();
            *state.call_camera_enabled.borrow_mut() = true;
            let name = peer_display_name(state, &mid);
            let msg = crate::i18n::tf(
                if is_video {
                    "call_video_calling"
                } else {
                    "call_calling"
                },
                &[("name", &name)],
            );
            ensure_call_window(state, &name, &kind, CallMode::Outgoing(&msg));
        }
        Err(e) => {
            *state.pending_call_peer.borrow_mut() = Some(mid);
            call_error_toast(state, &e.to_string());
        }
    }
}

pub(super) fn cancel_pending_voice_call(state: &AppState) {
    *state.pending_call_peer.borrow_mut() = None;
    *state.pending_call_kind.borrow_mut() = "AUDIO".into();
    close_active_call_ui(state);
}

pub(super) fn answer_voice_call(state: &AppState) {
    if !calls_experimental_enabled(state) {
        toast(state, &crate::i18n::t("call_experimental_toast"));
        decline_voice_call(state);
        return;
    }
    let Some(from) = state.incoming_call_from.borrow().clone() else {
        toast(state, &crate::i18n::t("call_no_incoming"));
        return;
    };
    if state.active_call_peer.borrow().is_some() {
        toast(state, &crate::i18n::t("call_already_active"));
        return;
    }
    if let Some(ui) = state.call_ui.borrow().as_ref() {
        call_window::stop_ready_preview(ui);
    }
    let kind = state.incoming_call_kind.borrow().clone();
    let (audio_in, audio_out, video_in, mirror, mic_vol, spk_vol) = {
        let cfg = state.config.borrow();
        (
            cfg.audio_input.clone(),
            cfg.audio_output.clone(),
            cfg.video_input.clone(),
            cfg.call_video_mirror,
            cfg.call_mic_volume,
            cfg.call_spk_volume,
        )
    };
    match state
        .sidecar
        .call_answer(&audio_in, &audio_out, &video_in, mirror, mic_vol, spk_vol)
    {
        Ok(_) => {
            *state.pending_call_peer.borrow_mut() = None;
            *state.incoming_call_from.borrow_mut() = None;
            *state.active_call_peer.borrow_mut() = Some(from.clone());
            *state.active_call_kind.borrow_mut() = kind.clone();
            *state.call_camera_enabled.borrow_mut() = true;
            let name = peer_display_name(state, &from);
            ensure_call_window(
                state,
                &name,
                &kind,
                CallMode::Outgoing(&crate::i18n::tf("call_answering", &[("name", &name)])),
            );
        }
        Err(e) => call_error_toast(state, &e.to_string()),
    }
}

pub(super) fn decline_voice_call(state: &AppState) {
    let _ = state.sidecar.call_decline();
    *state.pending_call_peer.borrow_mut() = None;
    *state.incoming_call_from.borrow_mut() = None;
    close_active_call_ui(state);
    toast(state, &crate::i18n::t("call_declined"));
}

pub(super) fn end_voice_call(state: &AppState) {
    let has_live_call =
        state.active_call_peer.borrow().is_some() || state.incoming_call_from.borrow().is_some();
    if has_live_call {
        let _ = state.sidecar.call_end();
    }
    *state.pending_call_peer.borrow_mut() = None;
    *state.incoming_call_from.borrow_mut() = None;
    close_active_call_ui(state);
}

pub(super) enum CallMode<'a> {
    Ready(&'a str),
    Outgoing(&'a str),
    Incoming(&'a str),
    Connected(&'a str),
}

pub(super) fn ensure_call_window(
    state: &AppState,
    peer_name: &str,
    kind: &str,
    mode: CallMode<'_>,
) {
    let is_video = kind == "VIDEO";
    if state.call_ui.borrow().is_none() {
        let (mic_v, spk_v) = {
            let cfg = state.config.borrow();
            (cfg.call_mic_volume, cfg.call_spk_volume)
        };
        let ui = call_window::open_call_window(
            &state.app,
            &state.window,
            peer_name,
            mic_v,
            spk_v,
            is_video,
        );
        {
            let s = state.clone();
            ui.start_btn.connect_clicked(move |_| start_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.cancel_btn
                .connect_clicked(move |_| cancel_pending_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.hangup_btn.connect_clicked(move |_| end_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.answer_btn
                .connect_clicked(move |_| answer_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.decline_btn
                .connect_clicked(move |_| decline_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.mute_btn.connect_toggled(move |btn| {
                let muted = btn.is_active();
                *s.call_mic_muted.borrow_mut() = muted;
                let _ = s
                    .sidecar
                    .call_set_audio(muted, *s.call_deafened.borrow(), None, None);
                if let Some(ui) = s.call_ui.borrow().as_ref() {
                    call_window::update_mute_visual(ui, muted);
                }
            });
        }
        {
            let s = state.clone();
            ui.camera_btn.connect_toggled(move |btn| {
                let enabled = !btn.is_active();
                *s.call_camera_enabled.borrow_mut() = enabled;
                let mirror = s.config.borrow().call_video_mirror;
                let _ = s.sidecar.call_set_video(enabled, Some(mirror));
                if let Some(ui) = s.call_ui.borrow().as_ref() {
                    call_window::update_camera_visual(ui, enabled);
                }
            });
        }
        {
            let s = state.clone();
            ui.deafen_btn.connect_toggled(move |btn| {
                let deafened = btn.is_active();
                *s.call_deafened.borrow_mut() = deafened;
                let _ = s
                    .sidecar
                    .call_set_audio(*s.call_mic_muted.borrow(), deafened, None, None);
                if let Some(ui) = s.call_ui.borrow().as_ref() {
                    call_window::update_deafen_visual(ui, deafened);
                }
            });
        }
        {
            let s = state.clone();
            ui.mic_vol.connect_value_changed(move |scale| {
                let g = scale.value();
                s.config.borrow_mut().call_mic_volume = g;
                s.config.borrow().save(&s.data_dir);
                let _ = s.sidecar.call_set_audio(
                    *s.call_mic_muted.borrow(),
                    *s.call_deafened.borrow(),
                    Some(g),
                    None,
                );
            });
        }
        {
            let s = state.clone();
            ui.spk_vol.connect_value_changed(move |scale| {
                let g = scale.value();
                s.config.borrow_mut().call_spk_volume = g;
                s.config.borrow().save(&s.data_dir);
                let _ = s.sidecar.call_set_audio(
                    *s.call_mic_muted.borrow(),
                    *s.call_deafened.borrow(),
                    None,
                    Some(g),
                );
            });
        }
        {
            let s = state.clone();
            call_window::wire_close_hangup(&ui, move || end_voice_call(&s));
        }
        *state.call_mic_muted.borrow_mut() = false;
        *state.call_deafened.borrow_mut() = false;
        *state.call_camera_enabled.borrow_mut() = true;
        *state.call_ui.borrow_mut() = Some(ui);
    }

    if let Some(ui) = state.call_ui.borrow().as_ref() {
        call_window::set_video_mode(ui, is_video);
        call_window::set_peer_display(ui, peer_name);
        if is_video {
            call_window::update_camera_visual(ui, *state.call_camera_enabled.borrow());
        }
        match mode {
            CallMode::Ready(status) => call_window::set_ready_mode(ui, status),
            CallMode::Outgoing(status) => call_window::set_outgoing_mode(ui, status),
            CallMode::Incoming(status) => call_window::set_incoming_mode(ui, status),
            CallMode::Connected(status) => call_window::set_connected(ui, status),
        }
        ui.window.present();
    }
}

pub(super) fn close_active_call_ui(state: &AppState) {
    *state.pending_call_peer.borrow_mut() = None;
    *state.pending_call_kind.borrow_mut() = "AUDIO".into();
    *state.active_call_peer.borrow_mut() = None;
    *state.active_call_kind.borrow_mut() = "AUDIO".into();
    *state.incoming_call_from.borrow_mut() = None;
    *state.incoming_call_kind.borrow_mut() = "AUDIO".into();
    *state.call_mic_muted.borrow_mut() = false;
    *state.call_deafened.borrow_mut() = false;
    *state.call_camera_enabled.borrow_mut() = true;
    if let Some(ui) = state.call_ui.borrow_mut().take() {
        call_window::close_call_window(&ui);
    }
}

pub(super) fn handle_call_state(
    state: &AppState,
    peer: &str,
    kind: &str,
    call_state: &str,
    error: Option<&str>,
) {
    if !calls_experimental_enabled(state)
        && matches!(
            call_state,
            "ringing" | "connecting" | "connected" | "acquiring"
        )
    {
        let _ = state.sidecar.call_end();
        close_active_call_ui(state);
        return;
    }
    let resolved_kind = if kind == "VIDEO" || kind == "AUDIO" {
        kind.to_string()
    } else {
        state.active_call_kind.borrow().clone()
    };
    let is_video = resolved_kind == "VIDEO";
    let name = peer_display_name(state, peer);
    match call_state {
        "acquiring" | "ringing" | "starting" => {
            *state.active_call_peer.borrow_mut() = Some(peer.to_string());
            *state.active_call_kind.borrow_mut() = resolved_kind.clone();
            let msg = crate::i18n::tf(
                if is_video {
                    "call_video_calling"
                } else {
                    "call_calling"
                },
                &[("name", &name)],
            );
            ensure_call_window(state, &name, &resolved_kind, CallMode::Outgoing(&msg));
        }
        "connected" => {
            *state.active_call_peer.borrow_mut() = Some(peer.to_string());
            *state.active_call_kind.borrow_mut() = resolved_kind.clone();
            ensure_call_window(
                state,
                &name,
                &resolved_kind,
                CallMode::Connected(&crate::i18n::tf("call_connected", &[("name", &name)])),
            );
        }
        "ended" => {
            close_active_call_ui(state);
            toast(state, &crate::i18n::t("call_ended"));
        }
        "failed" => {
            close_active_call_ui(state);
            let err = error.unwrap_or("call failed");
            // Only nudge QR re-login for actual desktop-session problems — not Opus/media bugs.
            if err.contains("relogin_android_required") || err.contains("DESKTOPWIN") {
                call_error_toast(state, err);
            } else {
                toast(state, &crate::i18n::tf("call_failed", &[("error", err)]));
            }
        }
        _ => {}
    }
}

pub(super) fn handle_call_video_frame(
    state: &AppState,
    side: &str,
    jpeg_b64: Option<&str>,
    cleared: bool,
) {
    if let Some(ui) = state.call_ui.borrow().as_ref() {
        call_window::update_video_frame(ui, side, jpeg_b64, cleared);
    }
}

pub(super) fn notify_call_incoming(state: &AppState, name: &str, kind: &str) {
    if !notifications_allowed(state) {
        return;
    }
    let is_video = kind == "VIDEO";
    let n = gio::Notification::new(&crate::i18n::t(if is_video {
        "video_call"
    } else {
        "voice_call"
    }));
    n.set_body(Some(&crate::i18n::tf(
        if is_video {
            "call_video_incoming"
        } else {
            "call_incoming"
        },
        &[("name", name)],
    )));
    n.set_priority(gio::NotificationPriority::Urgent);
    state.app.send_notification(Some("line-gtk-call"), &n);
}
