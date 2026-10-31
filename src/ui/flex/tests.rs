use super::*;
use serde_json::json;

#[test]
fn style_values_cannot_inject_css() {
    assert_eq!(color("#ffffff; color:red"), None);
    assert_eq!(color("#12345678"), Some("#12345678"));
    assert_eq!(length("NaNpx"), None);
    assert_eq!(length("-1px"), None);
    assert_eq!(length("12px"), Some(12));
    let node: Node =
        serde_json::from_value(json!({"type":"span", "text":"<b>&", "color":"#ff0000"})).unwrap();
    assert_eq!(
        span_markup(&node),
        "<span foreground=\"#ff0000\">&lt;b&gt;&amp;</span>"
    );
}

#[test]
#[ignore = "requires a GTK display"]
fn gtk_flex_appointment_snapshot() {
    libadwaita::init().unwrap();
    let row = |icon: &str, label: &str, value: &str| {
        json!({"type":"box", "layout":"horizontal", "spacing":"sm", "contents":[
            {"type":"text", "text":icon,"flex":1,"size":"sm"},
            {"type":"text", "text":label,"flex":2,"size":"sm","color":"#6d7f7b","wrap":true},
            {"type":"text", "text":value,"flex":5,"size":"sm","wrap":true}
        ]})
    };
    let fixture = json!({"type":"bubble","size":"giga","header":{"type":"box","layout":"vertical","backgroundColor":"#0c6b5c","paddingAll":"20px","spacing":"md","contents":[
        {"type":"text","text":"แจ้งเตือนล่วงหน้า 1 วัน","color":"#ffffff","size":"md","wrap":true},
        {"type":"text","text":"พรุ่งนี้มี 1 รายการ","color":"#ffffff","size":"xxl","weight":"bold","wrap":true},
        {"type":"text","text":"ศุกร์ 30 ตุลาคม 2569","color":"#ffffff","size":"md","wrap":true}
    ]},"body":{"type":"box","layout":"vertical","spacing":"xl","contents":[
        {"type":"box","layout":"vertical","backgroundColor":"#e5f4ef","cornerRadius":"12px","paddingAll":"12px","contents":[{"type":"text","text":"กิจกรรม 1","align":"center","color":"#137566","weight":"bold"}]},
        {"type":"text","text":"กิจกรรม 1","color":"#137566","weight":"bold","size":"lg"},
        {"type":"box","layout":"horizontal","backgroundColor":"#edf6ff","borderColor":"#dddddd","borderWidth":"1px","cornerRadius":"12px","contents":[
            {"type":"box","layout":"vertical","width":"5px","flex":0,"backgroundColor":"#3267e9","contents":[]},
            {"type":"box","layout":"vertical","flex":1,"paddingAll":"14px","spacing":"md","contents":[
                {"type":"text","text":"📅 ประชุมรับการประเมินคุณภาพมาตรฐานโรงพยาบาล","size":"lg","weight":"bold","wrap":true},
                {"type":"text","text":"ID EXAMPLE","size":"sm","color":"#6d7f7b"},
                row("📅","วันที่","ศุกร์ 30 ตุลาคม 2569"),row("⏰","เวลา","09:00–11:00 น."),row("📍","สถานที่","โรงพยาบาลร้องกวาง"),row("👥","ผู้เกี่ยวข้อง","ผู้เข้าร่วมประชุม")
            ]}
        ]}
    ]}});
    let fixture = if let Ok(path) = std::env::var("LINE_FLEX_FIXTURE") {
        let roots: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let index = std::env::var("LINE_FLEX_FIXTURE_INDEX")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        roots
            .as_array()
            .map(|rows| rows[index].clone())
            .unwrap_or(roots)
    } else {
        fixture
    };
    let clicked = Rc::new(std::cell::RefCell::new(Vec::new()));
    let actions = clicked.clone();
    let widget = render(
        &fixture,
        Rc::new(move |action| actions.borrow_mut().push(action)),
    )
    .unwrap();
    let window = gtk::Window::builder()
        .default_width(400)
        .child(&widget)
        .build();
    window.present();
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(widget.width() >= 320);
    assert!(widget.height() > 150);
    let paintable = gtk::WidgetPaintable::new(Some(&widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
    let texture = window
        .renderer()
        .unwrap()
        .render_texture(&snapshot.to_node().unwrap(), None);
    texture.save_to_png("/tmp/line-gtk-flex.png").unwrap();
    let action = json!({"type":"bubble","body":{"type":"box","layout":"vertical","contents":[{"type":"button","action":{"type":"message","label":"Send","text":"Exact message"}}]}});
    let actions = clicked.clone();
    let action_widget = render(
        &action,
        Rc::new(move |action| actions.borrow_mut().push(action)),
    )
    .unwrap();
    let button = action_widget
        .first_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    button.emit_clicked();
    assert_eq!(clicked.borrow()[0].data.as_deref(), Some("Exact message"));
    window.close();
}
