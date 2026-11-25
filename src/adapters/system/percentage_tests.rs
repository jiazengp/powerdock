use super::*;
#[test]
fn windows_unknown_levels_are_not_zero_or_truncated() {
    assert_eq!(percentage(DEVPROP_TYPE_BYTE, &[0]), Some(0));
    assert_eq!(percentage(DEVPROP_TYPE_BYTE, &[100]), Some(100));
    assert_eq!(percentage(DEVPROP_TYPE_BYTE, &[101]), None);
    assert_eq!(
        percentage(DEVPROP_TYPE_UINT32, &65536u32.to_le_bytes()),
        None
    );
    assert_eq!(percentage(DEVPROP_TYPE_BOOLEAN, &[1]), None);
}
