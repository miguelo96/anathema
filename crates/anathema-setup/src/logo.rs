use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};

pub const EMBER: Color32 = Color32::from_rgb(255, 90, 31);
pub const EMBER_DIM: Color32 = Color32::from_rgb(120, 44, 18);

fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = 64;
    (0..=steps)
        .map(|i| {
            let t = from + (to - from) * i as f32 / steps as f32;
            center + Vec2::new(t.cos(), t.sin()) * radius
        })
        .collect()
}

pub fn draw(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = size * 0.36;
    let width = size * 0.075;
    let deg = std::f32::consts::PI / 180.0;
    painter.add(egui::Shape::line(
        arc(center, radius, -52.0 * deg, 232.0 * deg),
        Stroke::new(width, EMBER),
    ));
    painter.add(egui::Shape::line(
        arc(center, radius, 246.0 * deg, 262.0 * deg),
        Stroke::new(width, EMBER_DIM),
    ));
    let spark = center + Vec2::new((-30.0 * deg).cos(), (-30.0 * deg).sin()) * (radius + width * 2.2);
    painter.circle_filled(spark, width * 0.55, EMBER);
    painter.circle_filled(center, width * 0.5, EMBER_DIM);
}
