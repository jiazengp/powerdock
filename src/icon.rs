//! Full-size Fluent system Mouse glyph with a proportional battery badge.
//! Rasterize only when data, theme or physical tray size changes.
use crate::{battery::Kind, settings::wide};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
#[derive(Clone, Copy, Debug)]
pub struct Appearance {
    pub foreground: u32,
    pub fill: u32,
}
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
pub fn appearance(dark: bool, hc: bool, percent: Option<u8>, charging: bool) -> Appearance {
    let foreground = unsafe {
        if hc {
            GetSysColor(COLOR_WINDOWTEXT)
        } else if percent.is_none() {
            if dark {
                rgb(167, 174, 185)
            } else {
                rgb(92, 99, 112)
            }
        } else if dark {
            rgb(250, 250, 250)
        } else {
            rgb(24, 28, 34)
        }
    };
    let fill = if hc {
        foreground
    } else if charging {
        if dark {
            rgb(108, 203, 95)
        } else {
            rgb(15, 123, 15)
        }
    } else if percent.is_some_and(|value| value <= 20) {
        if dark {
            rgb(255, 153, 164)
        } else {
            rgb(196, 43, 28)
        }
    } else if percent.is_some() {
        if dark {
            rgb(117, 194, 255)
        } else {
            rgb(0, 95, 184)
        }
    } else {
        foreground
    };
    Appearance { foreground, fill }
}
fn channels(c: u32) -> [f32; 3] {
    [
        (c & 255) as f32,
        ((c >> 8) & 255) as f32,
        ((c >> 16) & 255) as f32,
    ]
}
fn rounded(x: f32, y: f32, b: [f32; 4], radius: f32) -> bool {
    let [l, t, r, bottom] = b;
    if x < l || x > r || y < t || y > bottom {
        return false;
    }
    let cx = x.clamp(l + radius, r - radius);
    let cy = y.clamp(t + radius, bottom - radius);
    (x - cx) * (x - cx) + (y - cy) * (y - cy) <= radius * radius
}
/// Full-size system Mouse glyph with a lower-right battery modifier.
/// Percentage is deliberately in the Shell tooltip and panel, not tiny tray numerals.
pub fn pixels(percent: Option<u8>, style: Appearance, size: i32) -> Result<Vec<u32>, String> {
    pixels_for(percent, style, size, Kind::Mouse)
}
pub fn pixels_for(
    percent: Option<u8>,
    style: Appearance,
    size: i32,
    kind: Kind,
) -> Result<Vec<u32>, String> {
    unsafe {
        if !(16..=128).contains(&size) {
            return Err("无效的托盘图标尺寸".into());
        }
        let factor = 4;
        let side = size * factor;
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return Err("无法创建图标绘图上下文".into());
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = side;
        info.bmiHeader.biHeight = -side;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            return Err("无法创建鼠标字形位图".into());
        }
        std::ptr::write_bytes(bits.cast::<u8>(), 0, (side * side * 4) as usize);
        let previous = SelectObject(dc, bitmap);
        let font = CreateFontW(
            -side,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe Fluent Icons").as_ptr(),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0xFFFFFF);
        let glyph = wide(kind.glyph().to_string());
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: side,
            bottom: side,
        };
        DrawTextW(
            dc,
            glyph.as_ptr(),
            1,
            &mut bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        GdiFlush();
        let mask = std::slice::from_raw_parts(bits.cast::<u32>(), (side * side) as usize);
        let mut left = side;
        let mut top = side;
        let mut right = 0;
        let mut bottom = 0;
        for y in 0..side {
            for x in 0..side {
                if mask[(y * side + x) as usize] & 255 > 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        let glyph_width = (right - left).max(1) as f32;
        let glyph_height = (bottom - top).max(1) as f32;
        let scale = (14.9 / glyph_height).min(13.6 / glyph_width);
        let height = glyph_height * scale;
        let width = glyph_width * scale;
        let mouse_left = (14.3 - width) / 2.0;
        let mouse_top = (15.6 - height) / 2.0;
        let ink = channels(style.foreground);
        let fill = channels(style.fill);
        let unit = 16.0 / side as f32;
        let mut result = vec![0u32; (size * size) as usize];
        for py in 0..size {
            for px in 0..size {
                let mut sum = [0f32; 4];
                for sy in 0..factor {
                    for sx in 0..factor {
                        let x = (px * factor + sx) as f32 * unit + unit / 2.0;
                        let y = (py * factor + sy) as f32 * unit + unit / 2.0;
                        let mut p = [0.0; 4];
                        if x >= mouse_left
                            && x < mouse_left + width
                            && y >= mouse_top
                            && y < mouse_top + height
                        {
                            let gx = left + ((x - mouse_left) / width * glyph_width) as i32;
                            let gy = top + ((y - mouse_top) / height * glyph_height) as i32;
                            if gx >= 0 && gx < side && gy >= 0 && gy < side {
                                let a = (mask[(gy * side + gx) as usize] & 255) as f32 / 255.0;
                                p = [ink[0] * a, ink[1] * a, ink[2] * a, 255.0 * a];
                            }
                        }
                        // A transparent one-pixel separation keeps the modifier recognizable.
                        if x > 6.25 && y > 9.7 {
                            p = [0.0; 4];
                        }
                        let outer = rounded(x, y, [7.0, 10.6, 14.9, 15.35], 0.65);
                        let inner = rounded(x, y, [8.0, 11.55, 13.95, 14.4], 0.15);
                        let terminal = (14.9..15.9).contains(&x) && (12.0..14.0).contains(&y);
                        let frame = (outer && !inner) || terminal;
                        let level = inner
                            && percent.is_some()
                            && x <= 8.0 + 5.95 * f32::from(percent.unwrap_or(0).min(100)) / 100.0;
                        let unknown = inner
                            && percent.is_none()
                            && ((x - 8.0) / 5.95 + (y - 11.55) / 2.85 - 1.0).abs() < 0.17;
                        if frame || level || unknown {
                            p = [fill[0], fill[1], fill[2], 255.0];
                        }
                        for c in 0..4 {
                            sum[c] += p[c] / (factor * factor) as f32;
                        }
                    }
                }
                result[(py * size + px) as usize] = ((sum[3].round() as u32) << 24)
                    | ((sum[0].round() as u32) << 16)
                    | ((sum[1].round() as u32) << 8)
                    | (sum[2].round() as u32);
            }
        }
        SelectObject(dc, old_font);
        DeleteObject(font);
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        Ok(result)
    }
}
pub fn create(
    percent: Option<u8>,
    style: Appearance,
    size: i32,
    kind: Kind,
) -> Result<HICON, String> {
    unsafe {
        let pixels = pixels_for(percent, style, size, kind)?;
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let image = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if image.is_null() {
            return Err("无法创建鼠标图标位图".into());
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
        let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: image,
        });
        DeleteObject(mask);
        DeleteObject(image);
        if icon.is_null() {
            Err("无法创建鼠标电量图标".into())
        } else {
            Ok(icon)
        }
    }
}
/// Contact sheet uses this exact rasterizer: 16 px native, and 24 px at 2x zoom.
pub fn export_preview(path: &std::path::Path) -> Result<(), String> {
    let width = 640usize;
    let height = 240usize;
    let mut canvas = vec![0u8; width * height * 4];
    let cases = [
        Some(100),
        Some(75),
        Some(37),
        Some(20),
        Some(5),
        Some(0),
        Some(60),
        None,
    ];
    for (row, dark) in [false, true].into_iter().enumerate() {
        let bg = if dark { [32u8; 3] } else { [250u8; 3] };
        for y in row * 120..(row + 1) * 120 {
            for x in 0..width {
                let at = (y * width + x) * 4;
                canvas[at..at + 3].copy_from_slice(&bg);
                canvas[at + 3] = 255;
            }
        }
        for (column, percent) in cases.iter().copied().enumerate() {
            let style = appearance(dark, false, percent, column == 6);
            for (dy, size, zoom) in [(16, 16, 1), (51, 24, 2)] {
                let image = pixels(percent, style, size)?;
                for y in 0..size as usize {
                    for x in 0..size as usize {
                        let p = image[y * size as usize + x];
                        let a = p >> 24;
                        let c = [p & 255, (p >> 8) & 255, (p >> 16) & 255];
                        for sy in 0..zoom {
                            for sx in 0..zoom {
                                let at = (((row * 120 + dy + y * zoom + sy) * width)
                                    + (column * 80 + 15 + x * zoom + sx))
                                    * 4;
                                for i in 0..3 {
                                    canvas[at + i] =
                                        (c[i] + u32::from(bg[i]) * (255 - a) / 255).min(255) as u8;
                                }
                                canvas[at + 3] = 255;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut bmp = Vec::with_capacity(54 + canvas.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54u32 + canvas.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    bmp.extend_from_slice(&canvas);
    std::fs::write(path, bmp).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_reflect_charge_and_level() {
        assert_ne!(
            appearance(true, false, Some(5), false).fill,
            appearance(true, false, Some(75), false).fill
        );
        assert_ne!(
            appearance(true, false, Some(75), true).fill,
            appearance(true, false, Some(75), false).fill
        );
    }
    #[test]
    fn battery_badge_fill_grows_with_percentage() {
        let style = appearance(true, false, Some(50), false);
        let mut previous = 0u64;
        for percent in [0, 20, 50, 100] {
            let image = pixels(Some(percent), style, 16).unwrap();
            let alpha: u64 = (11..15)
                .flat_map(|y| (8..14).map(move |x| y * 16 + x))
                .map(|i| u64::from(image[i] >> 24))
                .sum();
            assert!(alpha > previous, "fill must grow at {percent}%");
            previous = alpha;
        }
    }
    #[test]
    fn all_peripheral_kinds_render_at_native_tray_sizes() {
        for kind in [
            Kind::Mouse,
            Kind::Keyboard,
            Kind::Headset,
            Kind::Gamepad,
            Kind::Phone,
            Kind::Other,
        ] {
            for size in [16, 24, 32] {
                let image = pixels_for(
                    Some(18),
                    appearance(true, false, Some(18), false),
                    size,
                    kind,
                )
                .unwrap();
                assert!(image.iter().any(|p| p >> 24 != 0));
                assert!(image.iter().all(|p| {
                    let a = p >> 24;
                    (p & 255) <= a && ((p >> 8) & 255) <= a && ((p >> 16) & 255) <= a
                }));
            }
        }
    }
    #[test]
    fn raster_is_premultiplied_at_all_supported_tray_sizes() {
        for size in [16, 20, 24, 32] {
            for p in [None, Some(0), Some(5), Some(37), Some(100)] {
                let image = pixels(p, appearance(true, false, p, false), size).unwrap();
                assert_eq!(image.len(), (size * size) as usize);
                assert!(image.iter().any(|p| p >> 24 != 0));
                assert!(image.iter().all(|p| {
                    let a = p >> 24;
                    (p & 255) <= a && ((p >> 8) & 255) <= a && ((p >> 16) & 255) <= a
                }));
            }
        }
    }
}
