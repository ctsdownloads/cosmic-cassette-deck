mod app;
mod audio;
mod cassette;
mod jcard;
mod label;
mod library;
mod mpris;
mod rack_scene;
mod case_scene;
mod case_open;
mod artscrape;

/// Smallest window the layout stays readable at.
const MIN_W: f32 = 420.0;
const MIN_H: f32 = 760.0;

fn main() -> cosmic::iced::Result {
    // Render with Vulkan unless the user picked a backend. The OpenGL fallback
    // crashes on hybrid Intel/NVIDIA systems (khronos-egl panics when NVIDIA's
    // EGL returns no display without an error code), and every package ships
    // Vulkan. If Vulkan fails anyway, iced falls back to software rendering.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "vulkan");
    }

    // Portrait window to match the skin's aspect (843×1264 + controls below),
    // or the size it had when it was last closed.
    let size = app::read_state()
        .get("window")
        .and_then(|v| v.split_once('x'))
        .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
        .filter(|&(w, h)| w >= MIN_W && h >= MIN_H)
        .unwrap_or((560.0, 1020.0));
    let mut settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(size.0, size.1))
        .size_limits(cosmic::iced::Limits::NONE.min_width(MIN_W).min_height(MIN_H));

    // Always the dark theme: every page sits on the charcoal background, so the
    // widgets and text must not follow a light desktop theme.
    settings = settings.theme(cosmic::Theme::dark());

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
