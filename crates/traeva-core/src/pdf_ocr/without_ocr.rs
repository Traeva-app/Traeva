//! Unsupported-platform stand-ins. The runtime returns `unsupported`.

pub(crate) mod render {
    use super::super::Settings;

    pub(crate) struct Gray {
        pub width: usize,
        pub height: usize,
    }

    pub(crate) struct Rendered {
        pub gray: Gray,
        pub millis: u128,
    }

    pub(crate) fn for_each_rendered_page(
        _bytes: &[u8],
        pages: &[usize],
        _page_count: usize,
        settings: &Settings,
        mut visit: impl FnMut(usize, Result<Rendered, String>),
    ) {
        for &number in pages.iter().take(settings.max_ocr_pages) {
            visit(
                number,
                Err("local PDF OCR is supported only on macOS".to_string()),
            );
        }
    }
}

pub(crate) mod vision {
    use serde::Serialize;

    use super::super::Settings;
    use super::render::Gray;

    #[derive(Debug, Clone, Serialize)]
    pub struct Line {
        pub text: String,
        pub confidence: f32,
        pub bbox: [f64; 4],
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct VisionInfo;

    pub fn prepare_ocr(_settings: &Settings) -> Result<super::super::Preparation, String> {
        Err("local PDF OCR is supported only on macOS".to_string())
    }

    pub fn vision_info() -> Result<VisionInfo, String> {
        Err("local PDF OCR is supported only on macOS".to_string())
    }

    pub fn system_supported() -> bool {
        false
    }

    pub(crate) fn recognize(_gray: &Gray, _settings: &Settings) -> Result<Vec<Line>, String> {
        Err("local PDF OCR is supported only on macOS".to_string())
    }
}
