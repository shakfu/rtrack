mod app;
mod dialogs;
mod grid;
mod history;
mod input;
mod instrument_editor;
mod menu;
mod pattern_matrix;
mod sidebar;
mod state;
mod transport;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1220.0, 700.0])
            .with_title("rtrack"),
        ..Default::default()
    };

    eframe::run_native(
        "rtrack",
        options,
        Box::new(|cc| Ok(Box::new(app::RtrackApp::new(cc)))),
    )
}
