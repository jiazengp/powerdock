//! Native window dispatch, reentrancy fencing and deferred events.
use super::*;

pub(super) unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Context;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let context = &*pointer;
        // HID arrival/removal can be synchronously reentrant. Queue only interface
        // identities here, before borrowing App, and coalesce the PnP burst.
        if message == WM_DEVICECHANGE
            && (wparam == DBT_DEVICEARRIVAL as usize || wparam == DBT_DEVICEREMOVECOMPLETE as usize)
            && lparam != 0
        {
            let header = &*(lparam as *const DEV_BROADCAST_HDR);
            let offset = std::mem::offset_of!(DEV_BROADCAST_DEVICEINTERFACE_W, dbcc_name);
            if header.dbch_devicetype == DBT_DEVTYP_DEVICEINTERFACE
                && (offset + 2..=8192).contains(&(header.dbch_size as usize))
            {
                let units = std::slice::from_raw_parts(
                    (lparam as *const u8).add(offset).cast::<u16>(),
                    (header.dbch_size as usize - offset) / 2,
                );
                let end = units
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(units.len());
                let path = String::from_utf16_lossy(&units[..end]);
                let mut pending = context.pending_interfaces.borrow_mut();
                if !path.is_empty() && !pending.iter().any(|old| old.eq_ignore_ascii_case(&path)) {
                    pending.push(path);
                }
                SetTimer(hwnd, TIMER_TOPOLOGY, 300, None);
            }
            return 0;
        }
        // Calls into the Shell or another UI process may synchronously dispatch messages.
        if context.active.replace(true) {
            let flag = match message {
                WM_RESULT => 1,
                WM_POWERBROADCAST => {
                    context.generation.fetch_add(1, Ordering::AcqRel);
                    2
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE => 4,
                _ => 0,
            };
            context.deferred.set(context.deferred.get() | flag);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let _guard = DispatchGuard(context);
        let app = &mut *context.app.get();
        match message {
            WM_RESULT => {
                app.consume_result();
                0
            }
            WM_OPEN => {
                app.open_ui(0);
                0
            }
            WM_UI_REGISTER => {
                let ui = wparam as HWND;
                if app.valid_ui(ui) {
                    let first_registration = app.ui != ui;
                    app.ui = ui;
                    if first_registration {
                        PostMessageW(ui, WM_UI_PAGE, usize::from(app.page), 0);
                    }
                    app.publish_state();
                }
                0
            }
            WM_UI_CLOSED => {
                if wparam as HWND == app.ui {
                    app.ui = null_mut();
                }
                0
            }
            WM_COPYDATA => {
                if lparam != 0 {
                    let data = &*(lparam as *const COPYDATASTRUCT);
                    let sender = wparam as HWND;
                    if agent::is_request_tag(data.dwData) {
                        if data.cbData == 0
                            || data.cbData as usize > agent::MAX_BYTES
                            || data.lpData.is_null()
                            || !agent::same_executable(sender)
                        {
                            return 0;
                        }
                        let bytes = std::slice::from_raw_parts(
                            data.lpData.cast::<u8>(),
                            data.cbData as usize,
                        );
                        let (response, limit) =
                            match serde_json::from_slice::<agent::Request>(bytes) {
                                Ok(request) if request.version == 1 => {
                                    let limit = agent::reply_limit(data.dwData, &request.command);
                                    (app.agent_command(request.command), limit)
                                }
                                _ => (
                                    agent::failure(
                                        "invalid_request",
                                        "Unsupported schema or malformed request",
                                    ),
                                    agent::MAX_BYTES,
                                ),
                            };
                        return isize::from(agent::send_reply(sender, hwnd, response, limit));
                    }
                }
                if wparam as HWND != app.ui || !app.valid_ui(app.ui) || lparam == 0 {
                    return 0;
                }
                let data = &*(lparam as *const COPYDATASTRUCT);
                if data.dwData == 4
                    && data.cbData > 0
                    && data.cbData <= 512
                    && !data.lpData.is_null()
                {
                    let bytes =
                        std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize);
                    if let Ok(operation @ agent::Operation::GeneralSet { .. }) =
                        serde_json::from_slice::<agent::Operation>(bytes)
                    {
                        return isize::from(app.agent_command(operation)["ok"] == true);
                    }
                    return 0;
                }
                if data.dwData == 3
                    && data.cbData > 0
                    && data.cbData <= 256
                    && !data.lpData.is_null()
                {
                    let bytes =
                        std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize);
                    if let Ok(operation @ agent::Operation::DeviceReminderSet { .. }) =
                        serde_json::from_slice::<agent::Operation>(bytes)
                    {
                        let response = app.agent_command(operation);
                        if response["ok"] != true {
                            app.notice_error = Some("设备提醒设置未保存，请重试".into());
                            app.publish_state();
                            return 0;
                        }
                        return 1;
                    }
                    return 0;
                }
                if data.dwData != 2 || data.cbData == 0 || data.cbData > 64 || data.lpData.is_null()
                {
                    return 0;
                }
                let bytes =
                    std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize);
                if let Ok(id) = std::str::from_utf8(bytes) {
                    return isize::from(app.refresh_device(id));
                }
                0
            }
            WM_COMMAND => {
                if lparam as HWND == app.ui && app.valid_ui(app.ui) {
                    app.command((wparam & 0xFFFF) as u16);
                }
                0
            }
            WM_TRAY => {
                let event = (lparam as u32) & 0xFFFF;
                if event == WM_LBUTTONUP || event == NIN_SELECT || event == (NIN_SELECT | 1) {
                    app.open_ui(0);
                } else if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                    app.open_ui(2);
                }
                0
            }
            WM_RESCAN => {
                app.last = None;
                app.error = Some("设备连接已变更，正在重新查询".into());
                let _ = app.update_tray(false);
                app.publish_state();
                app.refresh(true);
                0
            }
            WM_TIMER => {
                if wparam == TIMER_EXIT {
                    PostQuitMessage(0);
                } else if wparam == TIMER_TOPOLOGY {
                    KillTimer(hwnd, TIMER_TOPOLOGY);
                    let interfaces = std::mem::take(&mut *context.pending_interfaces.borrow_mut());
                    if !app.demo
                        && let Some(tx) = &app.tx
                        && let Err(mpsc::TrySendError::Full(Command::TopologyChanged {
                            interfaces,
                            ..
                        })) = tx.try_send(Command::TopologyChanged { interfaces })
                    {
                        context.pending_interfaces.borrow_mut().extend(interfaces);
                        SetTimer(hwnd, TIMER_TOPOLOGY, 300, None);
                    }
                } else if wparam == TIMER_NOTICE {
                    app.on_notice_timer();
                } else if wparam == TIMER_UPDATE {
                    if let Some(at) = app.updates.tick(&app.config.updates) {
                        app.config.updates.last_checked_at = at;
                        app.save();
                    }
                    SetTimer(
                        hwnd,
                        TIMER_UPDATE,
                        if app.updates.busy() { 1000 } else { 60_000 },
                        None,
                    );
                    app.publish_state();
                } else if wparam == TIMER_STALE {
                    app.flush_deferred_notices();
                    if !app.demo {
                        app.select_current();
                    }
                    let _ = app.update_tray(false);
                    app.publish_state();
                    app.reap_ui();
                }
                0
            }
            WM_POWERBROADCAST => {
                if wparam == 0x12 || wparam == 7 {
                    app.generation.fetch_add(1, Ordering::AcqRel);
                    app.error = Some("系统已恢复，正在重新查询".into());
                    let _ = app.update_tray(false);
                    app.publish_state();
                    app.refresh(true);
                }
                1
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED => {
                i18n::set_language(app.config.language);
                app.publish_state();
                app.dark = theme_dark();
                app.icon_key.clear();
                let _ = app.update_tray(false);
                0
            }
            WM_CLOSE | WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ if message == app.taskbar_message => {
                app.icon_key.clear();
                if let Err(e) = app.update_tray(true) {
                    settings::log_error(&e);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
