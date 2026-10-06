use anyhow::{Result, anyhow};
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use qrcode::EcLevel;
use qrcode::QrCode;
use std::cell::Cell;
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMethod {
    Qr,
    Email,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginPhase {
    Idle,
    Busy,
    Restarting,
    Recovering,
}

pub struct LoginWidgets {
    /// Full-bleed root (owns the page gradient).
    pub page: gtk::Box,
    pub method: Cell<LoginMethod>,
    pub phase: Cell<LoginPhase>,
    pub email: gtk::Entry,
    pub password: gtk::PasswordEntry,
    pub submit_btn: gtk::Button,
    pub method_btn: gtk::Button,
    pub email_help: gtk::Label,
    pub stage: gtk::Stack,
    pub qr_picture: gtk::Picture,
    pub pin_label: gtk::Label,
    pub hint: gtk::Label,
    pub retry_btn: gtk::Button,
    pub subtitle: gtk::Label,
    pub pin_caption: gtk::Label,
    pub note: gtk::Label,
}

pub fn build_login_page() -> LoginWidgets {
    // Root fills the window so the gradient covers the whole page.
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-login-page"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_top(40)
        .margin_bottom(40)
        .margin_start(40)
        .margin_end(40)
        .css_classes(["line-login-content"])
        .build();

    let title = gtk::Label::builder()
        .label("LINE GTK")
        .css_classes(["title-1", "line-login-brand"])
        .build();

    let subtitle = gtk::Label::builder()
        .label(crate::i18n::t("login_subtitle"))
        .css_classes(["dim-label", "line-login-subtitle"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .visible(false)
        .build();

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-login-card"])
        .build();

    let stage = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vhomogeneous(true)
        .hhomogeneous(true)
        .build();

    let qr_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();

    let qr_picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .width_request(280)
        .height_request(280)
        .css_classes(["line-login-qr"])
        .build();
    qr_box.append(&qr_picture);
    stage.add_named(&qr_box, Some("qr"));

    let email_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .width_request(280)
        .build();
    let email = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("login_email"))
        .input_purpose(gtk::InputPurpose::Email)
        .build();
    let password = gtk::PasswordEntry::builder()
        .placeholder_text(crate::i18n::t("login_password"))
        .show_peek_icon(true)
        .build();
    let submit_btn = gtk::Button::builder()
        .label(crate::i18n::t("login_submit"))
        .css_classes(["suggested-action"])
        .build();
    let email_help = gtk::Label::builder()
        .label(crate::i18n::t("login_email_help"))
        .wrap(true)
        .max_width_chars(36)
        .build();
    email_box.append(&email);
    email_box.append(&password);
    email_box.append(&submit_btn);
    email_box.append(&email_help);
    stage.add_named(&email_box, Some("email"));
    let method_btn = gtk::Button::with_label(&crate::i18n::t("login_use_email"));

    let pin_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(280)
        .height_request(280)
        .build();

    let pin_caption = gtk::Label::builder()
        .label(crate::i18n::t("login_pin_caption"))
        .css_classes(["title-3"])
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();

    let pin_label = gtk::Label::builder()
        .label("----")
        .justify(gtk::Justification::Center)
        .selectable(true)
        .build();
    pin_label.set_markup("<span size='92000' weight='heavy' letter_spacing='8000'>----</span>");

    pin_box.append(&pin_caption);
    pin_box.append(&pin_label);
    stage.add_named(&pin_box, Some("pin"));
    stage.set_visible_child_name("qr");

    card.append(&stage);

    let hint = gtk::Label::builder()
        .label(crate::i18n::t("login_qr_hint"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(40)
        .css_classes(["line-login-hint"])
        .build();

    let retry_btn = gtk::Button::builder()
        .label(crate::i18n::t("login_retry"))
        .css_classes(["pill", "suggested-action"])
        .halign(gtk::Align::Center)
        .visible(false)
        .build();

    let note = gtk::Label::builder()
        .label(crate::i18n::t("login_note"))
        .css_classes(["caption", "dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(48)
        .build();

    content.append(&title);
    content.append(&subtitle);
    content.append(&card);
    content.append(&hint);
    content.append(&retry_btn);
    content.append(&method_btn);
    content.append(&note);
    page.append(&content);

    LoginWidgets {
        page,
        method: Cell::new(LoginMethod::Qr),
        phase: Cell::new(LoginPhase::Idle),
        email,
        password,
        submit_btn,
        method_btn,
        email_help,
        stage,
        qr_picture,
        pin_label,
        hint,
        retry_btn,
        subtitle,
        pin_caption,
        note,
    }
}

pub fn apply_login_language(w: &LoginWidgets) {
    w.subtitle.set_text(&crate::i18n::t("login_subtitle"));
    w.email
        .set_placeholder_text(Some(&crate::i18n::t("login_email")));
    w.password
        .set_placeholder_text(Some(&crate::i18n::t("login_password")));
    w.submit_btn.set_label(&crate::i18n::t("login_submit"));
    w.email_help.set_text(&crate::i18n::t("login_email_help"));
    w.method_btn
        .set_label(&crate::i18n::t(if w.method.get() == LoginMethod::Qr {
            "login_use_email"
        } else {
            "login_use_qr"
        }));
    w.pin_caption.set_text(&crate::i18n::t("login_pin_caption"));
    w.retry_btn.set_label(&crate::i18n::t("login_retry"));
    w.note.set_text(&crate::i18n::t("login_note"));
}

pub fn show_method_stage(w: &LoginWidgets) {
    set_phase(w, LoginPhase::Idle);
    w.password.set_text("");
    if w.method.get() == LoginMethod::Qr {
        show_qr_stage(w);
    } else {
        w.stage.set_visible_child_name("email");
        w.retry_btn.set_visible(false);
        w.hint.set_text(&crate::i18n::t("login_email_hint"));
    }
    apply_login_language(w);
}

pub fn set_phase(w: &LoginWidgets, phase: LoginPhase) {
    w.phase.set(phase);
    let editable = phase == LoginPhase::Idle;
    w.email.set_sensitive(editable);
    w.password.set_sensitive(editable);
    w.submit_btn.set_sensitive(editable);
    w.method_btn.set_sensitive(!matches!(
        phase,
        LoginPhase::Restarting | LoginPhase::Recovering
    ));
}

pub fn valid_credentials(email: &str, password: &str) -> bool {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.contains('.')
        && !domain.contains('@')
        && !email.chars().any(char::is_whitespace)
        && !password.is_empty()
}

pub fn show_qr_stage(w: &LoginWidgets) {
    w.stage.set_visible_child_name("qr");
    w.retry_btn.set_visible(false);
    w.hint.set_text(&crate::i18n::t("login_qr_hint"));
    w.pin_label
        .set_markup("<span size='92000' weight='heavy' letter_spacing='8000'>----</span>");
}

pub fn show_pin_stage(w: &LoginWidgets, pin: &str) {
    set_phase(w, LoginPhase::Busy);
    let safe = glib::markup_escape_text(pin);
    w.pin_label.set_markup(&format!(
        "<span size='92000' weight='heavy' letter_spacing='8000'>{safe}</span>"
    ));
    w.stage.set_visible_child_name("pin");
    w.retry_btn.set_visible(true);
    w.hint.set_text(&crate::i18n::t("login_pin_waiting"));
}

pub fn set_qr(picture: &gtk::Picture, url: &str) -> Result<()> {
    let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)
        .map_err(|e| anyhow!("qr encode: {e}"))?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(280, 280)
        .build();

    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| anyhow!("png encode: {e}"))?;

    let bytes = gtk::glib::Bytes::from(&png[..]);
    let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf = Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE)
        .map_err(|e| anyhow!("pixbuf: {e}"))?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);
    picture.set_paintable(Some(&texture));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_reject_invalid_email_and_preserve_password_whitespace() {
        assert!(valid_credentials(" user@example.com ", " password "));
        assert!(valid_credentials("user@example.com", " "));
        for email in [
            "user",
            "@example.com",
            "user@@example.com",
            "user@.example.com",
            "user@example.com.",
            "bad user@example.com",
        ] {
            assert!(!valid_credentials(email, "password"));
        }
        assert!(!valid_credentials("user@example.com", ""));
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn email_widgets_follow_login_lifecycle() {
        gtk::init().unwrap();
        crate::i18n::set_lang(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), "eng");
        let w = build_login_page();
        w.method.set(LoginMethod::Email);
        show_method_stage(&w);
        assert_eq!(w.stage.visible_child_name().as_deref(), Some("email"));
        assert!(w.submit_btn.is_sensitive());
        assert!(w.password.shows_peek_icon());
        w.password.set_text(" secret ");
        assert_eq!(w.password.text(), " secret ");
        set_phase(&w, LoginPhase::Busy);
        assert!(!w.submit_btn.is_sensitive());
        assert!(w.method_btn.is_sensitive());
        show_pin_stage(&w, "1234");
        assert_eq!(w.stage.visible_child_name().as_deref(), Some("pin"));
        show_method_stage(&w);
        assert_eq!(w.stage.visible_child_name().as_deref(), Some("email"));
        assert_eq!(w.password.text(), "");
        w.hint.set_text(&crate::i18n::t("email_login_failed"));
        assert!(w.hint.text().contains("Login failed"));
        crate::i18n::set_lang(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), "th");
        apply_login_language(&w);
        assert_eq!(w.submit_btn.label().as_deref(), Some("เข้าสู่ระบบ"));
        assert!(w.email_help.text().contains("โทรศัพท์"));
        w.method.set(LoginMethod::Qr);
        show_method_stage(&w);
        assert_eq!(w.stage.visible_child_name().as_deref(), Some("qr"));
    }
}
