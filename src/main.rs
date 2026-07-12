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
    let mut settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(560.0, 1020.0))
        .size_limits(cosmic::iced::Limits::NONE.min_width(420.0).min_height(760.0));

    // libcosmic draws the header-bar window controls (minimize / maximize / close)
    // as XDG icon lookups against icon theme "Cosmic". Off COSMIC that theme isn't
    // installed, so the buttons exist and stay clickable but render blank -- an
    // empty header strip.
    //
    // This MUST go through Settings, not cosmic::icon_theme::set_default(): run()
    // internally calls set_default(config::icon_theme()) and overwrites any earlier
    // call. Settings::default_icon_theme also sets core.icon_theme_override, so a
    // later ToolkitConfig event can't reset it either.
    //
    // Lookup order is [chosen theme, "Cosmic"], so "Cosmic" remains the fallback.
    // Skipped under COSMIC so its own icon theme keeps priority there.
    if !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_uppercase()
        .contains("COSMIC")
    {
        settings = settings.default_icon_theme("Adwaita");
    }

    cosmic::app::run::<app::App>(settings, ())
}
