mod app;
mod audio;
mod cassette;
mod label;
mod loading;
mod library;
mod mpris;
mod rack_scene;
mod case_scene;
mod case_open;
mod artscrape;

fn main() -> cosmic::iced::Result {
    // Portrait window to match the skin's aspect (843×1264 + controls below).
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(560.0, 1020.0))
        .size_limits(cosmic::iced::Limits::NONE.min_width(420.0).min_height(760.0));

    cosmic::app::run::<app::App>(settings, ())
}
