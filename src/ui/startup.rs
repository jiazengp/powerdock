//! Login-startup registration, independent of device and presentation state.
use super::*;

pub(super) fn startup_enabled() -> bool {
    unsafe {
        ["PowerDock", "MchoseBattery"].iter().any(|name| {
            let mut size = 0;
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run").as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut size,
            ) == ERROR_SUCCESS
        })
    }
}
pub(super) fn set_startup(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut key = null_mut();
        let result = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run").as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if result != ERROR_SUCCESS {
            return Err(format!("无法设置登录启动：{result}"));
        }
        let result = if enabled {
            let text = wide(format!("\"{}\"", updates::launcher_path().display()));
            RegSetValueExW(
                key,
                wide("PowerDock").as_ptr(),
                0,
                REG_SZ,
                text.as_ptr().cast(),
                (text.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, wide("PowerDock").as_ptr())
        };
        let success = result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND);
        // Remove only our legacy Run value after the new preference was saved.
        let legacy = if success {
            RegDeleteValueW(key, wide("MchoseBattery").as_ptr())
        } else {
            ERROR_SUCCESS
        };
        RegCloseKey(key);
        if success && (legacy == ERROR_SUCCESS || legacy == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(format!(
                "登录启动设置失败：{}",
                if success { legacy } else { result }
            ))
        }
    }
}
