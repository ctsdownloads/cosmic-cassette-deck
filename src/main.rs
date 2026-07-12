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
    // libcosmic draws the header-bar window controls as XDG icon lookups
    // (window-minimize-symbolic / window-maximize-symbolic / window-close-symbolic)
    // against icon theme "Cosmic". Off COSMIC that theme isn't installed, so the
    // buttons still exist and are clickable but render blank.
    //
    // Adwaita ships all four (incl. window-restore-symbolic) and is present on
    // GNOME/Fedora/Ubuntu. libcosmic searches [chosen theme, "Cosmic"] in order,
    // so "Cosmic" remains the fallback for anything Adwaita lacks. Skipped under
    // COSMIC so its own icon theme keeps priority there.
    if !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_uppercase()
        .contains("COSMIC")
    {
        cosmic::icon_theme::set_default("Adwaita");
    }

    // Portrait window to match the skin's aspect (843×1264 + controls below).
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(560.0, 1020.0))
        .size_limits(cosmic::iced::Limits::NONE.min_width(420.0).min_height(760.0));

    cosmic::app::run::<app::App>(settings, ())
}
