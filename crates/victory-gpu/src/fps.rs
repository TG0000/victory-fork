//! The frame rate drawn into the presented picture (`VICTORY_FPS=1`): a
//! small bitmap copied over the back buffer's top left corner. It is redrawn
//! only when the text changes and needs no pipeline, so measuring with it on
//! costs one small copy per frame.

use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::host::Host;

/// Characters the box has room for.
const COLUMNS: u32 = 48;
/// A glyph's cell, in dots.
const CELL: (u32, u32) = (6, 9);

static TEXT: Mutex<String> = Mutex::new(String::new());

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_FPS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// What the next frames show.
pub(crate) fn set(text: String) {
    if enabled() {
        *TEXT.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }
}

/// The seven rows of a glyph, five dots each, the leftmost in bit 4.
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        '.' => [0, 0, 0, 0, 0, 0x0c, 0x0c],
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'D' => [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1b, 0x11],
        _ => [0; 7],
    }
}

/// Whether the dot at (`x`, `y`) of the box showing `text` is lit.
fn lit(text: &[char], x: u32, y: u32) -> bool {
    let (column, dx) = (x / CELL.0, x % CELL.0);
    if dx == 0 || y == 0 || y > 7 {
        return false;
    }
    let c = text.get(column as usize).copied().unwrap_or(' ');
    glyph(c)[y as usize - 1] & (1 << (5 - dx)) != 0
}

/// Lit and unlit pixels of `format`, opaque, and their size in bytes.
fn pixels(format: DXGI_FORMAT) -> Option<([u64; 2], usize)> {
    Some(match format {
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM => ([0xffff_ffff, 0xff00_0000], 4),
        DXGI_FORMAT_R10G10B10A2_UNORM => ([0xffff_ffff, 0xc000_0000], 4),
        DXGI_FORMAT_R16G16B16A16_FLOAT => ([0x3c00_3c00_3c00_3c00, 0x3c00_0000_0000_0000], 8),
        _ => return None,
    })
}

#[derive(Default)]
pub(crate) struct Overlay {
    /// The mapped bitmap, what it shows, and the picture it was made for.
    made: Option<Made>,
}

struct Made {
    buffer: ID3D12Resource,
    data: *mut u8,
    footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    scale: u32,
    picture: (u32, u32, DXGI_FORMAT),
    text: Option<String>,
}

impl Overlay {
    /// Records the copy into `back`, which is in the copy destination state.
    /// The last submission must be complete (the bitmap may be redrawn).
    pub(crate) fn record(
        &mut self,
        host: &Host,
        list: &ID3D12GraphicsCommandList,
        back: &ID3D12Resource,
        picture: (u32, u32, DXGI_FORMAT),
    ) {
        if !enabled() {
            return;
        }
        if self.made.as_ref().is_none_or(|m| m.picture != picture) {
            self.made = make(host, picture);
        }
        let Some(made) = &mut self.made else { return };
        let text = TEXT.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if text.is_empty() {
            return;
        }
        if made.text.as_ref() != Some(&text) {
            made.draw(&text);
            made.text = Some(text);
        }
        // SAFETY: live resources; the copies of their pointers are not
        // released (`ManuallyDrop`), and the box lies inside both.
        unsafe {
            let to = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(back),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            let from = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&made.buffer),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: made.footprint,
                },
            };
            list.CopyTextureRegion(&to, 0, 0, 0, &from, None);
        }
    }
}

fn make(host: &Host, picture: (u32, u32, DXGI_FORMAT)) -> Option<Made> {
    let (width, height, format) = picture;
    let (_, bytes) = pixels(format)?;
    let scale = (height / 360).max(1);
    let width = (COLUMNS * CELL.0 * scale).min(width);
    let height = (CELL.1 * scale).min(height);
    let pitch = (width * bytes as u32).next_multiple_of(D3D12_TEXTURE_DATA_PITCH_ALIGNMENT);
    let (buffer, data) = host.upload_buffer(u64::from(pitch) * u64::from(height))?;
    Some(Made {
        buffer,
        data,
        footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
            Offset: 0,
            Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                Format: format,
                Width: width,
                Height: height,
                Depth: 1,
                RowPitch: pitch,
            },
        },
        scale,
        picture,
        text: None,
    })
}

impl Made {
    fn draw(&mut self, text: &str) {
        let Some((colors, bytes)) = pixels(self.picture.2) else {
            return;
        };
        let text: Vec<char> = text.chars().collect();
        let f = self.footprint.Footprint;
        let mut row = vec![0u8; f.Width as usize * bytes];
        for y in 0..f.Height {
            for (x, pixel) in row.chunks_exact_mut(bytes).enumerate() {
                let on = lit(&text, x as u32 / self.scale, y / self.scale);
                let color = colors[usize::from(!on)].to_le_bytes();
                pixel.copy_from_slice(&color[..bytes]);
            }
            // SAFETY: the mapped buffer holds `Height` rows of `RowPitch`.
            unsafe {
                let at = self.data.add(y as usize * f.RowPitch as usize);
                std::ptr::copy_nonoverlapping(row.as_ptr(), at, row.len());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_follow_the_glyphs() {
        let text: Vec<char> = "1.".chars().collect();
        // The top dot of "1", its base, and the gap before the next cell.
        assert!(lit(&text, 3, 1));
        assert!(lit(&text, 2, 7) && lit(&text, 4, 7));
        assert!(!lit(&text, 6, 7));
        assert!(!lit(&text, 3, 0) && !lit(&text, 3, 8));
        assert!(lit(&text, CELL.0 + 2, 7));
        // Past the text.
        assert!(!lit(&text, 5 * CELL.0 + 3, 4));
    }
}
