use super::camera_test::CameraPreviewHandle;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use libadwaita::ApplicationWindow;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

const READY_PREVIEW_W: i32 = 480;
const READY_PREVIEW_H: i32 = 270;

pub struct CallUi {
    pub window: ApplicationWindow,
    pub avatar_label: gtk::Label,
    pub stage_avatar_label: gtk::Label,
    pub peer_label: gtk::Label,
    pub status_label: gtk::Label,
    pub timer_label: gtk::Label,
    pub video_stage: gtk::Overlay,
    pub remote_picture: gtk::Picture,
    pub remote_placeholder: gtk::Box,
    pub remote_wait_label: gtk::Label,
    pub local_pip_box: gtk::Box,
    pub local_pip_picture: gtk::Picture,
    pub ready_preview: Rc<CameraPreviewHandle>,
    pub is_video: Rc<Cell<bool>>,
    pub mute_btn: gtk::ToggleButton,
    pub camera_btn: gtk::ToggleButton,
    pub deafen_btn: gtk::ToggleButton,
    pub hangup_btn: gtk::Button,
    pub answer_btn: gtk::Button,
    pub decline_btn: gtk::Button,
    pub start_btn: gtk::Button,
    pub cancel_btn: gtk::Button,
    pub mic_vol: gtk::Scale,
    pub spk_vol: gtk::Scale,
    pub ready_box: gtk::Box,
    pub out_box: gtk::Box,
    pub in_box: gtk::Box,
    pub connected_at: Rc<RefCell<Option<Instant>>>,
    pub tick: Rc<RefCell<Option<glib::SourceId>>>,
    hangup_guard: Rc<RefCell<bool>>,
}

fn peer_initial(peer_name: &str) -> String {
    peer_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

pub fn open_call_window(
    app: &impl IsA<gtk::Application>,
    parent: &ApplicationWindow,
    peer_name: &str,
    mic_vol: f64,
    spk_vol: f64,
    is_video: bool,
) -> CallUi {
    let window = ApplicationWindow::builder()
        .application(app)
        .title(if is_video {
            crate::i18n::t("video_call")
        } else {
            crate::i18n::t("voice_call")
        })
        .default_width(if is_video { 720 } else { 380 })
        .default_height(if is_video { 620 } else { 560 })
        .resizable(is_video)
        .build();
    window.set_transient_for(Some(parent));
    window.set_modal(false);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(20)
        .margin_end(20)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-call-window"])
        .build();

    let video_stage = gtk::Overlay::builder()
        .hexpand(true)
        .vexpand(true)
        .visible(is_video)
        .css_classes(["line-call-video-stage"])
        .build();
    video_stage.set_size_request(520, 320);

    let remote_picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .hexpand(true)
        .vexpand(true)
        .build();
    video_stage.set_child(Some(&remote_picture));

    let remote_placeholder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let stage_avatar_label = gtk::Label::builder()
        .label(peer_initial(peer_name))
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-call-avatar"])
        .build();
    stage_avatar_label.set_size_request(96, 96);
    let remote_wait_label = gtk::Label::builder()
        .label(crate::i18n::tf(
            "call_remote_video_waiting",
            &[("name", peer_name)],
        ))
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "caption"])
        .build();
    remote_placeholder.append(&stage_avatar_label);
    remote_placeholder.append(&remote_wait_label);
    video_stage.add_overlay(&remote_placeholder);

    let local_pip_box = gtk::Box::builder()
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(12)
        .margin_bottom(12)
        .visible(false)
        .css_classes(["line-call-local-pip"])
        .build();
    local_pip_box.set_size_request(192, 108);
    let local_pip_picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .build();
    local_pip_picture.set_size_request(192, 108);
    local_pip_box.append(&local_pip_picture);
    video_stage.add_overlay(&local_pip_box);

    let placeholder_cb = remote_placeholder.clone();
    let wait_cb = remote_wait_label.clone();
    let ready_preview = CameraPreviewHandle::new(
        remote_picture.clone(),
        READY_PREVIEW_W,
        READY_PREVIEW_H,
        move |_fps, err, active| {
            if active {
                placeholder_cb.set_visible(false);
            } else {
                placeholder_cb.set_visible(true);
                if let Some(e) = err {
                    wait_cb.set_text(&crate::i18n::tf("camera_test_error", &[("error", &e)]));
                }
            }
        },
    );

    let avatar = gtk::Label::builder()
        .label(peer_initial(peer_name))
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .visible(!is_video)
        .css_classes(["line-call-avatar"])
        .build();
    avatar.set_size_request(96, 96);

    let peer_label = gtk::Label::builder()
        .label(peer_name)
        .halign(gtk::Align::Center)
        .css_classes(["title-2"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();

    let status_label = gtk::Label::builder()
        .label(crate::i18n::t("call_calling_short"))
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "title-4"])
        .build();

    let timer_label = gtk::Label::builder()
        .label("00:00")
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();

    let vol_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(4)
        .halign(gtk::Align::Center)
        .width_request(280)
        .build();

    let mic_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .homogeneous(false)
        .css_classes(["line-call-vol-row"])
        .build();
    let mic_lab = gtk::Label::builder()
        .label(crate::i18n::t("call_mic_vol"))
        .xalign(0.0)
        .width_request(64)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption"])
        .build();
    let mic_vol_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 4.0, 0.05);
    mic_vol_scale.set_value(mic_vol.clamp(0.0, 4.0));
    mic_vol_scale.add_mark(1.0, gtk::PositionType::Bottom, None);
    mic_vol_scale.set_hexpand(true);
    mic_vol_scale.set_draw_value(true);
    mic_vol_scale.set_value_pos(gtk::PositionType::Right);
    mic_vol_scale.set_digits(2);
    mic_row.append(&mic_lab);
    mic_row.append(&mic_vol_scale);

    let spk_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .homogeneous(false)
        .css_classes(["line-call-vol-row"])
        .build();
    let spk_lab = gtk::Label::builder()
        .label(crate::i18n::t("call_spk_vol"))
        .xalign(0.0)
        .width_request(64)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption"])
        .build();
    let spk_vol_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 4.0, 0.05);
    spk_vol_scale.set_value(spk_vol.clamp(0.0, 4.0));
    spk_vol_scale.add_mark(1.0, gtk::PositionType::Bottom, None);
    spk_vol_scale.set_hexpand(true);
    spk_vol_scale.set_draw_value(true);
    spk_vol_scale.set_value_pos(gtk::PositionType::Right);
    spk_vol_scale.set_digits(2);
    spk_row.append(&spk_lab);
    spk_row.append(&spk_vol_scale);

    vol_box.append(&mic_row);
    vol_box.append(&spk_row);

    let out_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .build();

    let mute_btn = gtk::ToggleButton::builder()
        .icon_name("microphone-sensitivity-high-symbolic")
        .tooltip_text(crate::i18n::t("call_mute_mic"))
        .css_classes(["circular", "line-call-ctl"])
        .build();
    mute_btn.set_size_request(52, 52);

    let camera_btn = gtk::ToggleButton::builder()
        .icon_name("camera-video-symbolic")
        .tooltip_text(crate::i18n::t("call_camera_off"))
        .visible(is_video)
        .css_classes(["circular", "line-call-ctl"])
        .build();
    camera_btn.set_size_request(52, 52);

    let deafen_btn = gtk::ToggleButton::builder()
        .icon_name("audio-volume-high-symbolic")
        .tooltip_text(crate::i18n::t("call_deafen"))
        .css_classes(["circular", "line-call-ctl"])
        .build();
    deafen_btn.set_size_request(52, 52);

    let hangup_btn = gtk::Button::builder()
        .icon_name("call-stop-symbolic")
        .tooltip_text(crate::i18n::t("call_hangup"))
        .css_classes(["circular", "destructive-action", "line-call-hangup"])
        .build();
    hangup_btn.set_size_request(64, 64);

    out_box.append(&mute_btn);
    out_box.append(&camera_btn);
    out_box.append(&hangup_btn);
    out_box.append(&deafen_btn);

    let in_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .visible(false)
        .build();

    let decline_btn = gtk::Button::builder()
        .icon_name("call-stop-symbolic")
        .tooltip_text(crate::i18n::t("call_decline"))
        .css_classes(["circular", "destructive-action", "line-call-hangup"])
        .build();
    decline_btn.set_size_request(64, 64);

    let answer_btn = gtk::Button::builder()
        .icon_name(if is_video {
            "camera-video-symbolic"
        } else {
            "call-start-symbolic"
        })
        .tooltip_text(crate::i18n::t("call_answer"))
        .css_classes(["circular", "suggested-action", "line-call-answer"])
        .build();
    answer_btn.set_size_request(64, 64);

    in_box.append(&decline_btn);
    in_box.append(&answer_btn);

    let ready_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .visible(false)
        .build();

    let cancel_btn = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text(crate::i18n::t("call_cancel"))
        .css_classes(["circular", "line-call-ctl"])
        .build();
    cancel_btn.set_size_request(64, 64);

    let start_btn = gtk::Button::builder()
        .icon_name(if is_video {
            "camera-video-symbolic"
        } else {
            "call-start-symbolic"
        })
        .tooltip_text(crate::i18n::t("call_start"))
        .css_classes(["circular", "suggested-action", "line-call-answer"])
        .build();
    start_btn.set_size_request(64, 64);

    ready_box.append(&cancel_btn);
    ready_box.append(&start_btn);

    root.append(&video_stage);
    root.append(&avatar);
    root.append(&peer_label);
    root.append(&status_label);
    root.append(&timer_label);
    root.append(&vol_box);
    root.append(&ready_box);
    root.append(&out_box);
    root.append(&in_box);

    window.set_content(Some(&root));
    window.present();

    CallUi {
        window,
        avatar_label: avatar,
        stage_avatar_label,
        peer_label,
        status_label,
        timer_label,
        video_stage,
        remote_picture,
        remote_placeholder,
        remote_wait_label,
        local_pip_box,
        local_pip_picture,
        ready_preview,
        is_video: Rc::new(Cell::new(is_video)),
        mute_btn,
        camera_btn,
        deafen_btn,
        hangup_btn,
        answer_btn,
        decline_btn,
        start_btn,
        cancel_btn,
        mic_vol: mic_vol_scale,
        spk_vol: spk_vol_scale,
        ready_box,
        out_box,
        in_box,
        connected_at: Rc::new(RefCell::new(None)),
        tick: Rc::new(RefCell::new(None)),
        hangup_guard: Rc::new(RefCell::new(false)),
    }
}

pub fn set_peer_display(ui: &CallUi, peer_name: &str) {
    let init = peer_initial(peer_name);
    ui.avatar_label.set_text(&init);
    ui.stage_avatar_label.set_text(&init);
    ui.peer_label.set_text(peer_name);
    ui.remote_wait_label.set_text(&crate::i18n::tf(
        "call_remote_video_waiting",
        &[("name", peer_name)],
    ));
}

pub fn set_video_mode(ui: &CallUi, is_video: bool) {
    if ui.is_video.get() == is_video {
        return;
    }
    ui.is_video.set(is_video);
    ui.window.set_title(Some(&if is_video {
        crate::i18n::t("video_call")
    } else {
        crate::i18n::t("voice_call")
    }));
    ui.video_stage.set_visible(is_video);
    ui.avatar_label.set_visible(!is_video);
    ui.camera_btn.set_visible(is_video);
    ui.start_btn.set_icon_name(if is_video {
        "camera-video-symbolic"
    } else {
        "call-start-symbolic"
    });
    ui.answer_btn.set_icon_name(if is_video {
        "camera-video-symbolic"
    } else {
        "call-start-symbolic"
    });
    ui.window.set_resizable(is_video);
    ui.window
        .set_default_size(if is_video { 720 } else { 380 }, if is_video { 620 } else { 560 });
    if !is_video {
        stop_ready_preview(ui);
        ui.local_pip_picture
            .set_paintable(None::<&gdk::Paintable>);
        ui.local_pip_box.set_visible(false);
    }
}

pub fn start_ready_preview(ui: &CallUi, device: &str, mirror: bool) {
    ui.local_pip_box.set_visible(false);
    ui.remote_placeholder.set_visible(true);
    ui.ready_preview.start(device, mirror);
}

pub fn stop_ready_preview(ui: &CallUi) {
    if ui.ready_preview.is_running() {
        ui.ready_preview.stop();
        ui.remote_picture.set_paintable(None::<&gdk::Paintable>);
        ui.remote_placeholder.set_visible(true);
    }
}

pub fn update_video_frame(ui: &CallUi, side: &str, jpeg_b64: Option<&str>, cleared: bool) {
    if cleared || jpeg_b64.is_none_or(str::is_empty) {
        match side {
            "local" => {
                ui.local_pip_picture
                    .set_paintable(None::<&gdk::Paintable>);
                ui.local_pip_box.set_visible(false);
            }
            "remote" => {
                ui.remote_picture.set_paintable(None::<&gdk::Paintable>);
                ui.remote_placeholder.set_visible(true);
            }
            _ => {}
        }
        return;
    }
    let Some(b64) = jpeg_b64 else {
        return;
    };
    let raw = glib::base64_decode(b64);
    if raw.is_empty() {
        return;
    }
    let bytes = glib::Bytes::from_owned(raw);
    let Ok(tex) = gdk::Texture::from_bytes(&bytes) else {
        return;
    };
    match side {
        "local" => {
            ui.local_pip_picture.set_paintable(Some(&tex));
            ui.local_pip_box.set_visible(true);
        }
        "remote" => {
            ui.remote_picture.set_paintable(Some(&tex));
            ui.remote_placeholder.set_visible(false);
        }
        _ => {}
    }
}

pub fn set_ready_mode(ui: &CallUi, status: &str) {
    stop_timer(ui);
    *ui.connected_at.borrow_mut() = None;
    ui.timer_label.set_visible(false);
    ui.status_label.set_text(status);
    ui.ready_box.set_visible(true);
    ui.out_box.set_visible(false);
    ui.in_box.set_visible(false);
}

pub fn set_outgoing_mode(ui: &CallUi, status: &str) {
    stop_ready_preview(ui);
    ui.status_label.set_text(status);
    ui.ready_box.set_visible(false);
    ui.out_box.set_visible(true);
    ui.in_box.set_visible(false);
}

pub fn set_incoming_mode(ui: &CallUi, status: &str) {
    stop_ready_preview(ui);
    ui.status_label.set_text(status);
    ui.ready_box.set_visible(false);
    ui.out_box.set_visible(false);
    ui.in_box.set_visible(true);
}

pub fn set_connected(ui: &CallUi, status: &str) {
    stop_ready_preview(ui);
    ui.status_label.set_text(status);
    ui.ready_box.set_visible(false);
    ui.out_box.set_visible(true);
    ui.in_box.set_visible(false);
    *ui.connected_at.borrow_mut() = Some(Instant::now());
    ui.timer_label.set_visible(true);
    start_timer(ui);
}

fn start_timer(ui: &CallUi) {
    stop_timer(ui);
    let connected_at = ui.connected_at.clone();
    let timer_label = ui.timer_label.clone();
    let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let Some(started) = *connected_at.borrow() else {
            return glib::ControlFlow::Break;
        };
        let secs = started.elapsed().as_secs();
        timer_label.set_text(&format!("{:02}:{:02}", secs / 60, secs % 60));
        glib::ControlFlow::Continue
    });
    *ui.tick.borrow_mut() = Some(id);
}

pub fn stop_timer(ui: &CallUi) {
    if let Some(id) = ui.tick.borrow_mut().take() {
        id.remove();
    }
}

pub fn close_call_window(ui: &CallUi) {
    *ui.hangup_guard.borrow_mut() = true;
    stop_ready_preview(ui);
    stop_timer(ui);
    ui.window.close();
}

pub fn update_mute_visual(ui: &CallUi, muted: bool) {
    if ui.mute_btn.is_active() != muted {
        ui.mute_btn.set_active(muted);
    }
    if muted {
        ui.mute_btn
            .set_icon_name("microphone-sensitivity-muted-symbolic");
        ui.mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_unmute_mic")));
    } else {
        ui.mute_btn
            .set_icon_name("microphone-sensitivity-high-symbolic");
        ui.mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_mute_mic")));
    }
}

pub fn update_camera_visual(ui: &CallUi, enabled: bool) {
    let off = !enabled;
    if ui.camera_btn.is_active() != off {
        ui.camera_btn.set_active(off);
    }
    if enabled {
        ui.camera_btn.set_icon_name("camera-video-symbolic");
        ui.camera_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_camera_off")));
    } else {
        ui.camera_btn.set_icon_name("camera-disabled-symbolic");
        ui.camera_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_camera_on")));
        ui.local_pip_picture
            .set_paintable(None::<&gdk::Paintable>);
        ui.local_pip_box.set_visible(false);
    }
}

pub fn update_deafen_visual(ui: &CallUi, deafened: bool) {
    if ui.deafen_btn.is_active() != deafened {
        ui.deafen_btn.set_active(deafened);
    }
    if deafened {
        ui.deafen_btn.set_icon_name("audio-volume-muted-symbolic");
        ui.deafen_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_undeafen")));
    } else {
        ui.deafen_btn.set_icon_name("audio-volume-high-symbolic");
        ui.deafen_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_deafen")));
    }
}

pub fn wire_close_hangup(ui: &CallUi, hangup: impl Fn() + 'static) {
    let guard = ui.hangup_guard.clone();
    ui.window.connect_close_request(move |_| {
        if !*guard.borrow() {
            hangup();
        }
        glib::Propagation::Proceed
    });
}
