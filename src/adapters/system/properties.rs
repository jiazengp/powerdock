//! Typed native PnP property decoding.
use super::*;

pub(super) fn property(node: u32, key: &DEVPROPKEY) -> Option<(u32, Vec<u8>)> {
    let mut data = vec![0; 4096];
    let mut size = data.len() as u32;
    let mut kind = 0;
    let result =
        unsafe { CM_Get_DevNode_PropertyW(node, key, &mut kind, data.as_mut_ptr(), &mut size, 0) };
    if result != CR_SUCCESS || size as usize > data.len() {
        return None;
    }
    data.truncate(size as usize);
    Some((kind, data))
}
pub(super) fn integer(kind: u32, data: &[u8]) -> Option<u32> {
    match (kind, data.len()) {
        (DEVPROP_TYPE_BYTE, 1) => Some(u32::from(data[0])),
        (DEVPROP_TYPE_UINT32, 4) => Some(u32::from_le_bytes(data.try_into().ok()?)),
        _ => None,
    }
}
pub(super) fn number(node: u32, key: &DEVPROPKEY) -> Option<u32> {
    let (kind, data) = property(node, key)?;
    integer(kind, &data)
}
pub(super) fn text(node: u32, key: &DEVPROPKEY) -> Option<String> {
    let (kind, data) = property(node, key)?;
    if kind != DEVPROP_TYPE_STRING || !data.len().is_multiple_of(2) {
        return None;
    }
    let utf16: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|u| *u != 0)
        .collect();
    Some(String::from_utf16_lossy(&utf16).chars().take(64).collect())
}
pub fn container(node: u32) -> Option<String> {
    let (kind, data) = property(node, &DEVPKEY_Device_ContainerId)?;
    if kind != DEVPROP_TYPE_GUID || data.len() != 16 || data.iter().all(|b| *b == 0) {
        return None;
    }
    Some(data.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn instance(node: u32) -> Option<String> {
    let mut data = [0u16; 512];
    if unsafe { CM_Get_Device_IDW(node, data.as_mut_ptr(), data.len() as u32, 0) } != CR_SUCCESS {
        return None;
    }
    let length = data.iter().position(|v| *v == 0)?;
    Some(String::from_utf16_lossy(&data[..length]))
}
