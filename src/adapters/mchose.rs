pub(crate) mod a7;
pub(crate) mod keyboard;
pub(crate) mod profiles;
mod reader;
mod yjx;

pub(crate) use reader::{Session, query, query_diagnostic, query_keyboard};

pub(crate) fn id(device: &super::device::Device) -> String {
    crate::battery::key(
        "mchose",
        device
            .container
            .as_deref()
            .unwrap_or(&String::from_utf16_lossy(&device.path)),
    )
}
