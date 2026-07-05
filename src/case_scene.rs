//! Case compositing: bakes an album cover onto the transparent cassette case
//! sprite (case_transparent.png, 480×735) for the fan browser and the reveal.
//! The cover fills the whole J-card (full-fill, per the approved design).

use cosmic::widget::image;

// ── Case compositing for the fan browser ────────────────────────────────────

use ::image as imagecrate;

/// The transparent case sprite (background removed) and its J-card fill zone,
/// baked from case_transparent.png (480×735).
const CASE_T_W: u32 = 480;
const CASE_T_H: u32 = 735;
const FILL_T: (u32, u32, u32, u32) = (68, 73, 413, 664);

/// Compose an album cover onto the transparent case, returning an RGBA handle
/// ready to draw in the fan. If `cover_bytes` is None (or fails to decode),
/// returns the blank case. `case_png` is the raw bytes of case_transparent.png.
pub fn compose(case_png: &[u8], cover_bytes: Option<&[u8]>) -> image::Handle {
    // Decode the transparent case (PNG) to RGBA.
    let case = match decode_rgba(case_png) {
        Some(img) => img,
        None => {
            return image::Handle::from_rgba(
                CASE_T_W,
                CASE_T_H,
                vec![0; (CASE_T_W * CASE_T_H * 4) as usize],
            )
        }
    };
    let (cw, ch) = (case.width(), case.height());
    let mut buf = case.into_raw(); // RGBA, len cw*ch*4

    if let Some(bytes) = cover_bytes {
        if let Some(cover) = decode_rgba(bytes) {
            let (fx0, fy0, fx1, fy1) = FILL_T;
            let fw = fx1 - fx0;
            let fh = fy1 - fy0;
            let cover = imagecrate::imageops::resize(
                &cover,
                fw,
                fh,
                imagecrate::imageops::FilterType::Lanczos3,
            );
            // Paint the cover into the fill zone (opaque).
            for y in 0..fh {
                for x in 0..fw {
                    let px = cover.get_pixel(x, y);
                    let dx = fx0 + x;
                    let dy = fy0 + y;
                    if dx < cw && dy < ch {
                        let i = ((dy * cw + dx) * 4) as usize;
                        buf[i] = px[0];
                        buf[i + 1] = px[1];
                        buf[i + 2] = px[2];
                        buf[i + 3] = 255;
                    }
                }
            }
        }
    }

    image::Handle::from_rgba(cw, ch, buf)
}

/// Decode image bytes (format auto-detected) into an RGBA buffer.
fn decode_rgba(bytes: &[u8]) -> Option<imagecrate::RgbaImage> {
    let reader = imagecrate::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    Some(img.to_rgba8())
}
