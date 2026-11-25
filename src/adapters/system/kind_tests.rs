use super::*;
#[test]
fn categories_use_typed_phone_metadata() {
    let bytes: Vec<u8> = "Other.Category\0Communication.Phone\0\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(category_kind(DEVPROP_TYPE_STRING_LIST, &bytes), Kind::Phone);
    assert_eq!(category_kind(DEVPROP_TYPE_STRING, &bytes), Kind::Other);
    assert_eq!(category_kind(DEVPROP_TYPE_STRING_LIST, &[1]), Kind::Other);
}
#[test]
fn types_do_not_guess_from_names_or_confuse_cod_bitfields() {
    assert_eq!(classic_kind(0x7a020c), Kind::Phone); // Generic Bluetooth phone class.
    assert_eq!(classic_kind(0x200), Kind::Phone);
    assert_eq!(ble_kind(0x40), Kind::Phone);
    assert_eq!(classic_kind(0x580), Kind::Mouse);
    assert_eq!(classic_kind(0x540), Kind::Keyboard);
    assert_eq!(classic_kind(0x508), Kind::Gamepad);
    assert_eq!(classic_kind(0x418), Kind::Headset);
    assert_eq!(ble_kind(0x3c2), Kind::Mouse);
    assert_eq!(ble_kind(0x943), Kind::Headset);
    assert_eq!(ble_kind(0), Kind::Other);
}
