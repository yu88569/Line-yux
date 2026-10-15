use super::*;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct QrPayload {
    text: String,
    http_url: Option<String>,
}

impl QrPayload {
    fn new(text: String) -> Self {
        let http_url = reqwest::Url::parse(&text)
            .ok()
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && !text.chars().any(char::is_whitespace)
                    && !text.chars().any(char::is_control)
            })
            .map(|url| url.to_string());
        Self { text, http_url }
    }
}

pub(super) fn decode(path: &Path) -> anyhow::Result<Vec<QrPayload>> {
    let image = image::open(path)?.into_luma8();
    let mut image = rqrr::PreparedImage::prepare(image);
    let grids = image.detect_grids();
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    let had_grids = !grids.is_empty();
    for grid in grids {
        if let Ok((_, text)) = grid.decode()
            && seen.insert(text.clone())
        {
            values.push(QrPayload::new(text));
        }
    }
    if had_grids && values.is_empty() {
        anyhow::bail!("{}", crate::i18n::t("qr_decode_failed"));
    }
    Ok(values)
}

fn still_current(button: &gtk::Button, window: &gtk::Window) -> bool {
    window.is_visible() && button.root().is_some_and(|root| root == *window)
}

pub(super) fn append_button(bar: &gtk::Box, path: &str, win: &gtk::Window) {
    let button = gtk::Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(crate::i18n::t("qr_read"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&button);
    let path = PathBuf::from(path);
    let weak_win = win.downgrade();
    button.connect_clicked(move |button| {
        if !button.is_sensitive() {
            return;
        }
        button.set_sensitive(false);
        button.set_tooltip_text(Some(&crate::i18n::t("qr_reading")));
        let path = path.clone();
        let weak_button = button.downgrade();
        let weak_win = weak_win.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || decode(&path)).await;
            let (Some(button), Some(win)) = (weak_button.upgrade(), weak_win.upgrade()) else {
                return;
            };
            button.set_sensitive(true);
            button.set_tooltip_text(Some(&crate::i18n::t("qr_read")));
            if !still_current(&button, &win) {
                return;
            }
            let values = match result {
                Ok(Ok(values)) => Ok(values),
                _ => Err(()),
            };
            show_results(&button, &win, values);
        });
    });
}

fn show_results(button: &gtk::Button, parent: &gtk::Window, result: Result<Vec<QrPayload>, ()>) {
    let dialog = gtk::Window::builder()
        .title(crate::i18n::t("qr_results"))
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .default_width(480)
        .default_height(220)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    match result {
        Ok(values) if !values.is_empty() => {
            for value in values {
                let label = gtk::Label::builder()
                    .label(&value.text)
                    .selectable(true)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .build();
                content.append(&label);
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let copy = gtk::Button::with_label(&crate::i18n::t("qr_copy"));
                copy.connect_clicked(move |button| button.clipboard().set_text(&value.text));
                actions.append(&copy);
                if let Some(url) = value.http_url {
                    let open = gtk::Button::with_label(&crate::i18n::t("qr_open"));
                    let weak_label = label.downgrade();
                    open.connect_clicked(move |_| {
                        if gio::AppInfo::launch_default_for_uri(
                            &url,
                            None::<&gio::AppLaunchContext>,
                        )
                        .is_err()
                            && let Some(label) = weak_label.upgrade()
                        {
                            label.set_text(&crate::i18n::t("qr_open_failed"));
                        }
                    });
                    actions.append(&open);
                }
                content.append(&actions);
            }
        }
        other => content.append(&gtk::Label::new(Some(&crate::i18n::t(if other.is_err() {
            "qr_decode_failed"
        } else {
            "qr_not_found"
        })))),
    }
    let close = gtk::Button::with_label(&crate::i18n::t("media_close"));
    let weak = dialog.downgrade();
    close.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    content.append(&close);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    dialog.set_child(Some(&scroll));
    let weak = dialog.downgrade();
    button.connect_root_notify(move |button| {
        if button.root().is_none()
            && let Some(dialog) = weak.upgrade()
        {
            dialog.close();
        }
    });
    dialog.present();
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn qr_image(text: &str) -> image::GrayImage {
        qrcode::QrCode::new(text.as_bytes())
            .unwrap()
            .render::<image::Luma<u8>>()
            .min_dimensions(240, 240)
            .build()
    }

    #[test]
    fn decodes_real_images_deduplicates_and_handles_missing_invalid_images() {
        let dir = std::env::temp_dir().join(format!("line-qr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qr.png");
        let url = "https://example.com/path?from=qr";
        qr_image(url).save(&path).unwrap();
        assert_eq!(decode(&path).unwrap(), vec![QrPayload::new(url.into())]);
        let text = "<b>hello</b> & สวัสดี";
        qr_image(text).save(&path).unwrap();
        assert_eq!(decode(&path).unwrap(), vec![QrPayload::new(text.into())]);
        let a = qr_image(url);
        let b = qr_image(text);
        let mut multiple = image::GrayImage::from_pixel(1000, 300, image::Luma([255]));
        image::imageops::overlay(&mut multiple, &a, 0, 0);
        image::imageops::overlay(&mut multiple, &a, 330, 0);
        image::imageops::overlay(&mut multiple, &b, 660, 0);
        multiple.save(&path).unwrap();
        let values = decode(&path).unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&QrPayload::new(url.into())));
        assert!(values.contains(&QrPayload::new(text.into())));
        image::GrayImage::from_pixel(240, 240, image::Luma([255]))
            .save(&path)
            .unwrap();
        assert!(decode(&path).unwrap().is_empty());
        std::fs::write(&path, b"invalid image").unwrap();
        assert!(decode(&path).is_err());
        assert!(decode(&dir.join("missing.png")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_http_and_https_payloads_get_open_actions() {
        for text in ["https://example.com", "http://localhost:8080/path"] {
            assert!(QrPayload::new(text.into()).http_url.is_some());
        }
        for text in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/plain,hello",
            "ftp://example.com",
            "https://",
            "//example.com",
            "example.com",
            "hello",
            " https://example.com",
            "https://example.com\n",
        ] {
            assert!(QrPayload::new(text.into()).http_url.is_none(), "{text:?}");
        }
    }
}
