use crate::protocol::FlexAction;
use gtk::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

type Activate = Rc<dyn Fn(FlexAction)>;

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Bubble,
    Carousel,
    Box,
    Text,
    Span,
    Button,
    Separator,
    Spacer,
    Filler,
    Image,
    Icon,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Node {
    #[serde(rename = "type")]
    kind: Kind,
    contents: Vec<Node>,
    header: Option<Box<Node>>,
    hero: Option<Box<Node>>,
    body: Option<Box<Node>>,
    footer: Option<Box<Node>>,
    styles: Styles,
    layout: String,
    align_items: String,
    justify_content: String,
    line_spacing: String,
    text: String,
    size: String,
    weight: String,
    color: String,
    align: String,
    gravity: String,
    wrap: bool,
    max_lines: Option<i32>,
    decoration: String,
    flex: Option<i32>,
    spacing: String,
    margin: String,
    padding_all: String,
    padding_top: String,
    padding_bottom: String,
    padding_start: String,
    padding_end: String,
    background_color: String,
    border_color: String,
    border_width: String,
    corner_radius: String,
    width: String,
    height: String,
    style: String,
    action: Option<Action>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Styles {
    header: SectionStyle,
    hero: SectionStyle,
    body: SectionStyle,
    footer: SectionStyle,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SectionStyle {
    background_color: String,
    separator: bool,
    separator_color: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Action {
    #[serde(rename = "type")]
    kind: String,
    label: String,
    text: Option<String>,
    data: Option<String>,
    uri: Option<String>,
}

impl Action {
    fn protocol(&self) -> FlexAction {
        FlexAction {
            kind: self.kind.clone(),
            label: self.label.clone(),
            data: if self.kind == "message" {
                self.text.clone()
            } else {
                self.data.clone()
            },
            uri: self.uri.clone(),
        }
    }
}

pub(super) fn render(contents: &Value, activate: Activate) -> Option<gtk::Widget> {
    let node: Node = serde_json::from_value(contents.clone()).ok()?;
    if !matches!(node.kind, Kind::Bubble | Kind::Carousel | Kind::Box) {
        return None;
    }
    let mut budget = 1500;
    render_node(&node, &activate, 0, &mut budget)
}

fn render_node(
    node: &Node,
    activate: &Activate,
    depth: usize,
    budget: &mut usize,
) -> Option<gtk::Widget> {
    if depth > 32 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let widget: gtk::Widget = match node.kind {
        Kind::Bubble => {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.set_size_request(bubble_width(&node.size), -1);
            card.set_halign(gtk::Align::Start);
            card.set_valign(gtk::Align::Start);
            card.set_overflow(gtk::Overflow::Hidden);
            style(
                &card,
                "background-color: #ffffff; color: #253c3b; border-radius: 16px; font-size: 16px;",
            );
            for (section, section_style) in [
                (&node.header, &node.styles.header),
                (&node.hero, &node.styles.hero),
                (&node.body, &node.styles.body),
                (&node.footer, &node.styles.footer),
            ] {
                if let Some(section) = section {
                    if section_style.separator {
                        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                        if let Some(color) = color(&section_style.separator_color) {
                            style(
                                &separator,
                                &format!("background-color: {color}; min-height: 1px;"),
                            );
                        }
                        card.append(&separator);
                    }
                    if let Some(child) = render_node(section, activate, depth + 1, budget) {
                        let padding = if section.padding_all.is_empty() {
                            "padding: 20px;"
                        } else {
                            ""
                        };
                        let background = color(&section_style.background_color)
                            .map(|c| format!("background-color: {c};"))
                            .unwrap_or_default();
                        style(
                            &child,
                            &format!("{padding}{background}{}", node_css(section)),
                        );
                        card.append(&child);
                    }
                }
            }
            card.upcast()
        }
        Kind::Carousel => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            for child in &node.contents {
                if let Some(widget) = render_node(child, activate, depth + 1, budget) {
                    row.append(&widget);
                }
            }
            gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .max_content_width(420)
                .propagate_natural_width(true)
                .child(&row)
                .build()
                .upcast()
        }
        Kind::Text | Kind::Span => {
            let label = gtk::Label::new(None);
            label.set_xalign(match node.align.as_str() {
                "center" => 0.5,
                "end" => 1.0,
                _ => 0.0,
            });
            label.set_yalign(match node.gravity.as_str() {
                "center" => 0.5,
                "bottom" => 1.0,
                _ => 0.0,
            });
            label.set_wrap(node.wrap);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_hexpand(node.flex != Some(0));
            if node.wrap {
                label.set_width_chars(1);
                label.set_max_width_chars(1);
            } else {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            if let Some(lines) = node.max_lines.filter(|n| *n > 0) {
                label.set_lines(lines);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            if node.contents.is_empty() {
                label.set_text(&node.text);
            } else {
                label.set_markup(&node.contents.iter().map(span_markup).collect::<String>());
            }
            if let Some(spacing) = length(&node.line_spacing) {
                label.layout().set_spacing(spacing * 1024);
            }
            label.upcast()
        }
        Kind::Button => {
            let action = node.action.as_ref()?;
            let button = gtk::Button::with_label(&action.label);
            let handler = activate.clone();
            let action = action.protocol();
            button.connect_clicked(move |_| handler(action.clone()));
            let button_css = match node.style.as_str() {
                "primary" => "background: #06c755; color: white; border: none;",
                "secondary" => "background: #eeeeee; color: #111111; border: none;",
                _ => "background: transparent; color: #42659a; border: none; box-shadow: none;",
            };
            style(&button, button_css);
            button.upcast()
        }
        Kind::Separator => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
        Kind::Spacer | Kind::Filler => {
            let space = gtk::Box::new(gtk::Orientation::Vertical, 0);
            space.set_size_request(-1, length(&node.size).unwrap_or(0));
            space.set_hexpand(true);
            space.upcast()
        }
        Kind::Image | Kind::Icon => {
            let placeholder = gtk::Image::from_icon_name("image-x-generic-symbolic");
            placeholder.set_pixel_size(if node.kind == Kind::Icon { 20 } else { 48 });
            placeholder.set_tooltip_text(Some("Image"));
            placeholder.upcast()
        }
        Kind::Box | Kind::Unknown => {
            let horizontal = matches!(node.layout.as_str(), "horizontal" | "baseline");
            if horizontal {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let mut weights = Vec::new();
                for child in &node.contents {
                    if let Some(widget) = render_node(child, activate, depth + 1, budget) {
                        if let Some(margin) = length(&child.margin) {
                            widget.set_margin_top(0);
                            widget.set_margin_start(margin);
                        }
                        widget.set_valign(match node.align_items.as_str() {
                            "center" => gtk::Align::Center,
                            "flex-end" => gtk::Align::End,
                            "flex-start" => gtk::Align::Start,
                            _ => gtk::Align::Fill,
                        });
                        weights.push(if !child.width.is_empty() {
                            0
                        } else {
                            child.flex.unwrap_or(1).clamp(0, 100)
                        });
                        row.append(&widget);
                    }
                }
                let manager: FlexRowLayout = glib::Object::new();
                use glib::subclass::types::ObjectSubclassIsExt;
                *manager.imp().weights.borrow_mut() = weights;
                manager
                    .imp()
                    .spacing
                    .set(length(&node.spacing).unwrap_or(0));
                row.set_layout_manager(Some(manager));
                row.upcast()
            } else {
                let column = gtk::Box::new(
                    gtk::Orientation::Vertical,
                    length(&node.spacing).unwrap_or(0),
                );
                if !node.text.is_empty() {
                    column.append(&gtk::Label::new(Some(&node.text)));
                }
                for child in &node.contents {
                    if let Some(widget) = render_node(child, activate, depth + 1, budget) {
                        widget.set_halign(match node.align_items.as_str() {
                            "center" => gtk::Align::Center,
                            "flex-end" => gtk::Align::End,
                            "flex-start" => gtk::Align::Start,
                            _ => gtk::Align::Fill,
                        });
                        column.append(&widget);
                    }
                }
                column.upcast()
            }
        }
    };
    let widget = if matches!(node.kind, Kind::Box | Kind::Unknown)
        && matches!(node.justify_content.as_str(), "center" | "flex-end")
    {
        let horizontal = matches!(node.layout.as_str(), "horizontal" | "baseline");
        let container = gtk::Box::new(
            if horizontal {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            },
            0,
        );
        let align = if node.justify_content == "center" {
            gtk::Align::Center
        } else {
            gtk::Align::End
        };
        if horizontal {
            widget.set_hexpand(true);
            widget.set_halign(align);
        } else {
            widget.set_vexpand(true);
            widget.set_valign(align);
        }
        container.append(&widget);
        container.upcast::<gtk::Widget>()
    } else {
        widget
    };
    if !node.corner_radius.is_empty() {
        widget.set_overflow(gtk::Overflow::Hidden);
    }
    style(&widget, &node_css(node));
    if let Some(margin) = length(&node.margin) {
        widget.set_margin_top(margin);
    }
    if let Some(width) = length(&node.width) {
        widget.set_size_request(width, widget.height_request());
    }
    if let Some(height) = length(&node.height) {
        widget.set_size_request(widget.width_request(), height);
    }
    if node.kind != Kind::Button
        && let Some(action) = &node.action
    {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let action = action.protocol();
        let handler = activate.clone();
        click.connect_released(move |_, _, _, _| handler(action.clone()));
        widget.add_controller(click);
        widget.set_cursor_from_name(Some("pointer"));
    }
    Some(widget)
}

fn bubble_width(size: &str) -> i32 {
    match size {
        "nano" => 180,
        "micro" => 220,
        "kilo" => 260,
        "giga" => 360,
        _ => 320,
    }
}

fn color(value: &str) -> Option<&str> {
    let hex = value.strip_prefix('#')?;
    (matches!(hex.len(), 6 | 8) && hex.bytes().all(|c| c.is_ascii_hexdigit())).then_some(value)
}

fn length(value: &str) -> Option<i32> {
    match value {
        "none" => Some(0),
        "xs" => Some(2),
        "sm" => Some(4),
        "md" => Some(8),
        "lg" => Some(12),
        "xl" => Some(16),
        "xxl" => Some(20),
        _ => value
            .strip_suffix("px")?
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1000.0)
            .map(|n| n.round() as i32),
    }
}

fn font_size(size: &str) -> Option<i32> {
    match size {
        "xxs" => Some(10),
        "xs" => Some(12),
        "sm" => Some(13),
        "md" => Some(14),
        "lg" => Some(16),
        "xl" => Some(20),
        "xxl" => Some(24),
        "3xl" => Some(30),
        "4xl" => Some(36),
        "5xl" => Some(48),
        _ => length(size),
    }
}

fn node_css(node: &Node) -> String {
    let mut css = String::new();
    for (property, value) in [
        ("background-color", &node.background_color),
        ("color", &node.color),
        ("border-color", &node.border_color),
    ] {
        if let Some(value) = color(value) {
            css.push_str(&format!("{property}: {value};"));
        }
    }
    for (property, value) in [
        ("padding", &node.padding_all),
        ("padding-top", &node.padding_top),
        ("padding-bottom", &node.padding_bottom),
        ("padding-left", &node.padding_start),
        ("padding-right", &node.padding_end),
        ("border-radius", &node.corner_radius),
        ("border-width", &node.border_width),
    ] {
        if let Some(value) = length(value) {
            css.push_str(&format!("{property}: {value}px;"));
        }
    }
    if !node.border_width.is_empty() {
        css.push_str("border-style: solid;");
    }
    if matches!(node.kind, Kind::Text | Kind::Span) {
        if let Some(size) = Some(font_size(&node.size).unwrap_or(16)) {
            css.push_str(&format!("font-size: {size}px;"));
        }
        if node.weight == "bold" {
            css.push_str("font-weight: 700;");
        }
        if matches!(node.decoration.as_str(), "underline" | "line-through") {
            css.push_str(&format!("text-decoration-line: {};", node.decoration));
        }
    }
    css
}

fn span_markup(node: &Node) -> String {
    let mut attrs = String::new();
    if let Some(color) = color(&node.color) {
        attrs.push_str(&format!(" foreground=\"{}\"", &color[..7]));
    }
    if let Some(size) = font_size(&node.size) {
        attrs.push_str(&format!(" size=\"{}\"", size * 1024));
    }
    if node.weight == "bold" {
        attrs.push_str(" weight=\"bold\"");
    }
    match node.decoration.as_str() {
        "underline" => attrs.push_str(" underline=\"single\""),
        "line-through" => attrs.push_str(" strikethrough=\"true\""),
        _ => {}
    }
    format!(
        "<span{attrs}>{}</span>",
        glib::markup_escape_text(&node.text)
    )
}

fn style(widget: &impl IsA<gtk::Widget>, css: &str) {
    if css.is_empty() {
        return;
    }
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let name = format!("flex-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let previous_name = widget.widget_name();
    if !previous_name.starts_with("flex-") {
        widget.set_widget_name(&name);
    }
    let name = widget.widget_name();
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("#{name} {{ {css} }}"));
    #[allow(deprecated)]
    widget
        .style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 30);
}

mod row_layout {
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct FlexRowLayout {
        pub weights: RefCell<Vec<i32>>,
        pub spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FlexRowLayout {
        const NAME: &'static str = "LineFlexRowLayout";
        type Type = super::FlexRowLayout;
        type ParentType = gtk::LayoutManager;
    }
    impl ObjectImpl for FlexRowLayout {}

    impl FlexRowLayout {
        fn children(&self, widget: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut result = Vec::new();
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                if current.should_layout() {
                    result.push(current);
                }
            }
            result
        }

        fn widths(&self, children: &[gtk::Widget], width: i32) -> Vec<i32> {
            let weights = self.weights.borrow();
            let fixed = children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    if weights.get(i) == Some(&0) {
                        child.measure(gtk::Orientation::Horizontal, -1).1
                    } else {
                        0
                    }
                })
                .collect::<Vec<_>>();
            let remaining = (width
                - fixed.iter().sum::<i32>()
                - self.spacing.get() * (children.len() as i32 - 1).max(0))
            .max(0);
            let total = weights.iter().sum::<i32>().max(1);
            let mut used = 0;
            let mut previous = 0;
            fixed
                .into_iter()
                .enumerate()
                .map(|(i, fixed)| {
                    let weight = weights.get(i).copied().unwrap_or(1);
                    if weight == 0 {
                        return fixed;
                    }
                    used += weight;
                    let edge = remaining * used / total;
                    let width = edge - previous;
                    previous = edge;
                    width
                })
                .collect()
        }
    }

    impl LayoutManagerImpl for FlexRowLayout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let children = self.children(widget);
            if orientation == gtk::Orientation::Horizontal {
                let spacing = self.spacing.get() * (children.len() as i32 - 1).max(0);
                let (minimum, natural) =
                    children
                        .iter()
                        .fold((spacing, spacing), |(minimum, natural), child| {
                            let (min, nat, _, _) = child.measure(orientation, -1);
                            (minimum + min, natural + nat.max(min))
                        });
                return (minimum, natural, -1, -1);
            }
            let widths = self.widths(&children, if for_size < 0 { 320 } else { for_size });
            let (minimum, natural) =
                children
                    .iter()
                    .zip(widths)
                    .fold((0, 0), |(minimum, natural), (child, width)| {
                        let (min, nat, _, _) = child.measure(orientation, width);
                        (minimum.max(min), natural.max(nat.max(min)))
                    });
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _: i32) {
            let children = self.children(widget);
            let widths = self.widths(&children, width);
            let mut x = 0;
            for (child, width) in children.iter().zip(widths) {
                let transform =
                    gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0));
                child.allocate(width, height, -1, Some(transform));
                x += width + self.spacing.get();
            }
        }
    }
}

glib::wrapper! {
    pub struct FlexRowLayout(ObjectSubclass<row_layout::FlexRowLayout>) @extends gtk::LayoutManager;
}

#[cfg(test)]
mod tests;
