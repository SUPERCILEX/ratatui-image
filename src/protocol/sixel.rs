//! Sixel backend implementations.
//! Uses [`sixel-bytes`] to draw image pixels, if the terminal [supports] the [Sixel] protocol.
//! Needs the `sixel` feature.
//!
//! [`sixel-bytes`]: https://github.com/benjajaja/sixel-bytes
//! [supports]: https://arewesixelyet.com
//! [Sixel]: https://en.wikipedia.org/wiki/Sixel
use image::{DynamicImage, Rgb};
use ratatui::{buffer::Buffer, layout::Rect};
use sixel_bytes::{sixel_string, DiffusionMethod, PixelFormat, SixelError};
use std::cmp::min;
use std::io;

use super::{Protocol, ResizeProtocol};
use crate::{ImageSource, Resize, Result};

// Fixed sixel backend
#[derive(Clone, Default)]
pub struct FixedSixel {
    pub data: String,
    pub rect: Rect,
}

impl FixedSixel {
    pub fn from_source(
        source: &ImageSource,
        resize: Resize,
        background_color: Option<Rgb<u8>>,
        area: Rect,
    ) -> Result<Self> {
        let (img, rect) = resize
            .resize(source, Rect::default(), area, background_color, false)
            .unwrap_or_else(|| (source.image.clone(), source.desired));

        let data = encode(img)?;
        Ok(Self { data, rect })
    }
}

// TODO: change E to sixel_rs::status::Error and map when calling
pub fn encode(img: DynamicImage) -> Result<String> {
    let (w, h) = (img.width(), img.height());
    let img_rgba8 = img.to_rgba8();
    let bytes = img_rgba8.as_raw();

    let data = sixel_string(
        bytes,
        w as _,
        h as _,
        PixelFormat::RGBA8888,
        DiffusionMethod::Stucki,
    )
    .map_err(sixel_err)?;
    Ok(data)
}

fn sixel_err(err: SixelError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("{err:?}"))
}

impl Protocol for FixedSixel {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        render(self.rect, &self.data, area, buf, false)
    }
}

fn render(rect: Rect, data: &str, area: Rect, buf: &mut Buffer, overdraw: bool) {
    let render_area = match render_area(rect, area, overdraw) {
        None => {
            // If we render out of area, then the buffer will attempt to write regular text (or
            // possibly other sixels) over the image.
            //
            // On some implementations (e.g. Xterm), this actually works but the image is
            // forever overwritten since we won't write out the same sixel data for the same
            // (col,row) position again (see buffer diffing).
            // Thus, when the area grows, the newly available cells will skip rendering and
            // leave artifacts instead of the image data.
            //
            // On some implementations (e.g. ???), only text with its foreground color is
            // overlayed on the image, also forever overwritten.
            //
            // On some implementations (e.g. patched Alactritty), image graphics are never
            // overwritten and simply draw over other UI elements.
            //
            // Note that [ResizeBackend] forces to ignore this early return, since it will
            // always resize itself to the area.
            return;
        }
        Some(r) => r,
    };

    buf.get_mut(render_area.left(), render_area.top())
        .set_symbol(data);
    let mut skip_first = false;

    // Skip entire area
    for y in render_area.top()..render_area.bottom() {
        for x in render_area.left()..render_area.right() {
            if !skip_first {
                skip_first = true;
                continue;
            }
            buf.get_mut(x, y).set_skip(true);
        }
    }
}

fn render_area(rect: Rect, area: Rect, overdraw: bool) -> Option<Rect> {
    if overdraw {
        return Some(Rect::new(
            area.x,
            area.y,
            min(rect.width, area.width),
            min(rect.height, area.height),
        ));
    }

    if rect.width > area.width || rect.height > area.height {
        return None;
    }
    Some(Rect::new(area.x, area.y, rect.width, rect.height))
}

#[derive(Clone)]
pub struct SixelState {
    source: ImageSource,
    current: FixedSixel,
    hash: u64,
}

impl SixelState {
    pub fn new(source: ImageSource) -> SixelState {
        SixelState {
            source,
            current: FixedSixel::default(),
            hash: u64::default(),
        }
    }
}

impl ResizeProtocol for SixelState {
    fn rect(&self) -> Rect {
        self.current.rect
    }
    fn render(
        &mut self,
        resize: &Resize,
        background_color: Option<Rgb<u8>>,
        area: Rect,
        buf: &mut Buffer,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let force = self.source.hash != self.hash;
        if let Some((img, rect)) = resize.resize(
            &self.source,
            self.current.rect,
            area,
            background_color,
            force,
        ) {
            match encode(img) {
                Ok(data) => {
                    let current = FixedSixel { data, rect };
                    self.current = current;
                    self.hash = self.source.hash;
                }
                Err(_err) => {
                    // TODO: save err in struct and expose in trait?
                }
            }
        }

        render(self.current.rect, &self.current.data, area, buf, true);
    }
}
