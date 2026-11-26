//! Bitmap payload shared with the native notification renderer.
use super::{DeviceIcon, pixels_for};
use base64::{Engine, engine::general_purpose::STANDARD};

// 64 px keeps the 28-DIP icon sharp on high-DPI displays and the encoded argument
// small enough for a Windows process command line. Match ui/NotificationIcon.cpp.
const SIDE: i32 = 64;

pub fn encode(device: DeviceIcon) -> Result<String, String> {
    let pixels = pixels_for(
        device.fill,
        device.appearance(true, false),
        SIDE,
        device.kind,
    )?;
    let bgra: Vec<u8> = pixels.into_iter().flat_map(u32::to_le_bytes).collect();
    Ok(STANDARD.encode(bgra))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{Kind, Power, Reading, Value};

    #[test]
    fn payload_matches_the_tray_raster_in_bgra_order() {
        let payload = |kind, percent| {
            let reading = Reading::new(Value::reported(percent), kind, Power::Battery);
            let device = DeviceIcon::from_reading(&reading);
            let decoded = STANDARD.decode(encode(device).unwrap()).unwrap();
            let expected =
                pixels_for(Some(percent), device.appearance(true, false), SIDE, kind).unwrap();
            assert_eq!(decoded.len(), (SIDE * SIDE * 4) as usize);
            assert!(
                decoded
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .eq(expected.into_iter().map(u32::to_le_bytes))
            );
            decoded
        };
        payload(Kind::Headset, 15);
        let mouse = payload(Kind::Mouse, 15);
        assert_ne!(mouse, payload(Kind::Mouse, 20));
    }
}
