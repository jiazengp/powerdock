//! Qualified Windows collection acquisition and bounded input registration.
use super::route::declares;
use super::*;
use windows::{
    Devices::HumanInterfaceDevice::{HidInputReportReceivedEventArgs, HidReportType},
    Foundation::TypedEventHandler,
    Storage::{FileAccessMode, Streams::DataReader},
    core::HSTRING,
};
const OPEN_FOR: Duration = Duration::from_secs(2);

pub fn open(
    endpoints: &[device::Device],
    generation: Arc<AtomicU64>,
    expected: u64,
) -> Result<Transport, String> {
    if endpoints.is_empty() || endpoints.len() > 2 {
        return Err(
            "unsupported: HID++ requires one unambiguous short/long collection pair".into(),
        );
    }
    let first = &endpoints[0];
    let mut routes = Vec::new();
    for endpoint in endpoints {
        let route = Route::from_metadata(endpoint)
            .ok_or("unsupported: unverified HID++ vendor collection")?;
        if routes.contains(&route)
            || endpoint.vendor_id != first.vendor_id
            || endpoint.product_id != first.product_id
            || (endpoints.len() > 1
                && (first.container.is_none() || endpoint.container != first.container))
        {
            return Err("unsupported: ambiguous HID++ collection association".into());
        }
        routes.push(route);
    }
    let state = State::new(generation, expected);
    let deadline = Instant::now() + OPEN_FOR;
    let mut transport = Transport {
        endpoints: Vec::new(),
        state,
        vendor_id: first.vendor_id,
        product_id: first.product_id,
    };
    for (metadata, route) in endpoints.iter().zip(routes) {
        if !transport.state.active() || Instant::now() >= deadline {
            return Err("cancelled: HID++ open lease expired".into());
        }
        let end = metadata
            .path
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(metadata.path.len());
        let path = HSTRING::from_wide(&metadata.path[..end]);
        let operation =
            HidDevice::FromIdAsync(&path, FileAccessMode::ReadWrite).map_err(win_error)?;
        let device = crate::winrt::wait(
            Ok(operation.clone()),
            deadline,
            &transport.state.generation,
            expected,
        )
        .map_err(|_| {
            if !transport.state.active() {
                "cancelled: HID++ open lease expired".into()
            } else if Instant::now() >= deadline {
                "timeout: HID++ open deadline".into()
            } else {
                operation
                    .ErrorCode()
                    .ok()
                    .filter(|code| code.is_err())
                    .map_or_else(
                        || "disconnected: Windows HID++ open did not complete".into(),
                        |code| win_error(windows::core::Error::from_hresult(code)),
                    )
            }
        })?;
        let mut endpoint = Endpoint {
            device,
            token: None,
            route,
        };
        if endpoint.device.VendorId().map_err(win_error)? != metadata.vendor_id
            || endpoint.device.ProductId().map_err(win_error)? != metadata.product_id
            || endpoint.device.UsagePage().map_err(win_error)? != metadata.usage_page
            || endpoint.device.UsageId().map_err(win_error)? != metadata.usage
            || !declares(&endpoint.device, HidReportType::Input, route)?
            || !declares(&endpoint.device, HidReportType::Output, route)?
        {
            return Err("unsupported: HID++ descriptor identity or report ID mismatch".into());
        }
        let report = endpoint
            .device
            .CreateOutputReportById(u16::from(route.id()))
            .map_err(win_error)?;
        if report.Id().map_err(win_error)? != u16::from(route.id())
            || report
                .Data()
                .map_err(win_error)?
                .Length()
                .map_err(win_error)? as usize
                != route.len()
        {
            return Err("unsupported: HID++ output report length mismatch".into());
        }
        let callback_state = transport.state.clone();
        endpoint.token = Some(
            endpoint
                .device
                .InputReportReceived(&TypedEventHandler::<
                    HidDevice,
                    HidInputReportReceivedEventArgs,
                >::new(move |_, args| {
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    if !callback_state.active() {
                        return Ok(());
                    }
                    let parsed = (|| -> windows::core::Result<Vec<u8>> {
                        let report = args.Report()?;
                        if report.Id()? != u16::from(route.id()) {
                            return Ok(Vec::new());
                        }
                        let buffer = report.Data()?;
                        if buffer.Length()? as usize != route.len() {
                            return Ok(Vec::new());
                        }
                        let reader = DataReader::FromBuffer(&buffer)?;
                        let mut data = vec![0; route.len()];
                        let result = reader.ReadBytes(&mut data);
                        let _ = reader.Close();
                        result?;
                        Ok(data)
                    })();
                    match parsed {
                        Ok(data) => {
                            callback_state.accept(route, data);
                        }
                        Err(_) => callback_state.retire(),
                    }
                    Ok(())
                }))
                .map_err(win_error)?,
        );
        transport.endpoints.push(endpoint);
    }
    if !transport.state.active() || Instant::now() >= deadline {
        return Err("cancelled: HID++ late open".into());
    }
    Ok(transport)
}
