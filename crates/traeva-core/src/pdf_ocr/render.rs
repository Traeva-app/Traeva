//! Page-at-a-time rendering with hayro, as 8-bit grey for Vision.
//! The output pixel budget does not cap decoded source images or Vision.

use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

use super::Settings;

/// One rendered page: 8-bit grey, one byte per pixel, rows top to bottom.
pub(crate) struct Gray {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub(crate) struct Rendered {
    pub gray: Gray,
    pub millis: u128,
}

const NOT_A_PASSWORD: &str = "\u{0}traeva-pdf-ocr: not a password\u{0}";
const MAX_SIDE: usize = 16_000;

/// The callback completes recognition and drops this page before the next
/// rendering starts. The PDF still retains its page tree and decoded page
/// content caches for the request; the process monitor covers those allocations.
pub(crate) fn for_each_rendered_page(
    bytes: &[u8],
    pages: &[usize],
    page_count: usize,
    settings: &Settings,
    visit: impl FnMut(usize, Result<Rendered, String>),
) {
    let plan = |width, height| plan_dimensions(width, height, settings.dpi, settings.max_pixels);
    for_each_page_planned(bytes, pages, page_count, settings, plan, visit);
}

/// `for_each_rendered_page` with each page's scale and size from `plan`
/// (given the page's width and height in points), still within
/// `settings.max_pixels`: a page for the model is drawn so that its longest
/// side fits (`model_image`).
pub(crate) fn for_each_page_planned(
    bytes: &[u8],
    pages: &[usize],
    page_count: usize,
    settings: &Settings,
    plan: impl Fn(f32, f32) -> Result<(f32, usize, usize), String>,
    mut visit: impl FnMut(usize, Result<Rendered, String>),
) {
    let pages = &pages[..pages.len().min(settings.max_ocr_pages)];
    if pages.is_empty() {
        return;
    }
    let opened = panic::catch_unwind(AssertUnwindSafe(|| {
        Pdf::new_with_password(bytes.to_vec(), NOT_A_PASSWORD)
    }));
    let pdf = match opened {
        Ok(Ok(pdf)) => pdf,
        _ => {
            for &number in pages {
                visit(
                    number,
                    Err("the PDF could not be opened for rendering".to_string()),
                );
            }
            return;
        }
    };
    let all = pdf.pages();
    if all.len() != page_count {
        for &number in pages {
            visit(
                number,
                Err("the PDF's page tree reads differently each time".to_string()),
            );
        }
        return;
    }
    let interpreter = InterpreterSettings {
        render_annotations: false,
        ..InterpreterSettings::default()
    };
    for &number in pages {
        let started = Instant::now();
        let result = match number.checked_sub(1).and_then(|index| all.get(index)) {
            None => Err(format!("page {number} is not in the PDF")),
            Some(page) => panic::catch_unwind(AssertUnwindSafe(|| {
                let (w, h) = page.render_dimensions();
                let (scale, planned_width, planned_height) = plan(w, h)?;
                // Decoded image/font caches must not accumulate across pages.
                let cache = RenderCache::new();
                let pixmap = render(
                    page,
                    &cache,
                    &interpreter,
                    &RenderSettings::default(),
                    &PixmapSettings {
                        x_scale: scale,
                        y_scale: scale,
                        bg_color: WHITE,
                    },
                );
                let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
                if (width, height) != (planned_width, planned_height)
                    || width
                        .checked_mul(height)
                        .is_none_or(|area| area > settings.max_pixels)
                {
                    return Err("the renderer exceeded the planned pixel dimensions".to_string());
                }
                let pixels = pixmap
                    .data_as_u8_slice()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| luma(p[0], p[1], p[2]))
                    .collect();
                Ok(Gray {
                    width,
                    height,
                    pixels,
                })
            }))
            .map_err(|_| format!("rendering page {number} stopped (internal error)"))
            .and_then(|result| result),
        };
        // The RGBA pixmap and renderer cache have already been dropped here.
        let millis = started.elapsed().as_millis();
        visit(number, result.map(|gray| Rendered { gray, millis }));
    }
}

/// Match hayro 0.8's f32 multiplication and u16 truncation before it allocates
/// the output buffer. Finite input checks also cover callers outside the CLI.
pub(crate) fn plan_dimensions(
    width_pt: f32,
    height_pt: f32,
    dpi: f32,
    max_pixels: usize,
) -> Result<(f32, usize, usize), String> {
    if !width_pt.is_finite() || !height_pt.is_finite() || width_pt <= 0.0 || height_pt <= 0.0 {
        return Err("page dimensions must be finite and positive".to_string());
    }
    if !dpi.is_finite() || dpi <= 0.0 || max_pixels == 0 {
        return Err("OCR dpi and pixel budget must be finite and positive".to_string());
    }
    let mut scale = dpi / 72.0;
    let area = f64::from(width_pt) * f64::from(height_pt) * f64::from(scale).powi(2);
    if area > max_pixels as f64 {
        scale = (max_pixels as f64 / (f64::from(width_pt) * f64::from(height_pt))).sqrt() as f32;
    }
    let longest = width_pt.max(height_pt);
    if f64::from(longest) * f64::from(scale) > MAX_SIDE as f64 {
        scale = MAX_SIDE as f32 / longest;
    }
    for _ in 0..4 {
        let (w, h) = (width_pt * scale, height_pt * scale);
        if !scale.is_finite()
            || scale <= 0.0
            || !w.is_finite()
            || !h.is_finite()
            || w < 1.0
            || h < 1.0
        {
            return Err("the page cannot fit the pixel budget at a nonzero size".to_string());
        }
        let (width, height) = (w as usize, h as usize);
        if width <= MAX_SIDE
            && height <= MAX_SIDE
            && width
                .checked_mul(height)
                .is_some_and(|area| area <= max_pixels)
        {
            return Ok((scale, width, height));
        }
        // Rounding the scale to f32 can put one edge just above the budget.
        scale = scale.next_down();
    }
    Err("the page cannot fit the pixel budget".to_string())
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as u8
}

#[cfg(test)]
mod tests {
    use super::plan_dimensions;

    #[test]
    fn rejects_invalid_dimensions_and_budgets() {
        for (width, height, dpi, pixels) in [
            (f32::NAN, 792.0, 200.0, 8_000_000),
            (612.0, f32::INFINITY, 200.0, 8_000_000),
            (-612.0, 792.0, 200.0, 8_000_000),
            (612.0, 0.0, 200.0, 8_000_000),
            (612.0, 792.0, f32::NAN, 8_000_000),
            (612.0, 792.0, f32::INFINITY, 8_000_000),
            (612.0, 792.0, 0.0, 8_000_000),
            (612.0, 792.0, 200.0, 0),
            (1.0, 1_000_000.0, 200.0, 10),
        ] {
            assert!(plan_dimensions(width, height, dpi, pixels).is_err());
        }
    }

    #[test]
    fn planned_dimensions_fit_exact_integer_pixel_budget() {
        for (width, height) in [(612.0, 792.0), (123.45, 678.9), (1e20, 2e20)] {
            for pixels in [1_024, 200_003, 1_000_003, 8_000_000, usize::MAX] {
                for dpi in [72.0, 200.0, 300.0, f32::MAX] {
                    let (scale, w, h) = plan_dimensions(width, height, dpi, pixels).unwrap();
                    assert!((1..=16_000).contains(&w));
                    assert!((1..=16_000).contains(&h));
                    assert!(w * h <= pixels);
                    assert_eq!(w, (width * scale) as u16 as usize);
                    assert_eq!(h, (height * scale) as u16 as usize);
                }
            }
        }
    }
}
