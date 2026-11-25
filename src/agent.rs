//! Local automation: CLI syntax, wire schema and native IPC have separate owners.
mod catalog;
mod cli;
mod compatibility;
mod ipc;
mod schema;
#[cfg(test)]
mod tests;

pub use catalog::{catalog_page, devices_page};
pub use cli::run;
pub use ipc::{is_request_tag, reply_limit, same_executable, send_reply};
pub use schema::{GeneralPatch, Operation, Request, failure, success};

pub const MAX_BYTES: usize = 32 * 1024;
pub(super) const MAX_SNAPSHOT_BYTES: usize = 128 * 1024;

#[cfg(test)]
pub(crate) use ipc::{call as call_for_test, legacy_call as legacy_call_for_test};

#[cfg(test)]
pub(crate) fn inventory_with_preview_host_for_test() -> serde_json::Value {
    let devices = crate::adapters::device::enumerate().expect("HID metadata enumeration");
    compatibility::inventory(&devices, ipc::call(Operation::ProvidersGet {}, true))
}
