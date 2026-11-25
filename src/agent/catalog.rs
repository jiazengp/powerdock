//! Byte-bounded pages for discovery metadata and active device snapshots.
use super::{MAX_BYTES, failure, success};
use serde_json::{Value, json};

pub fn catalog_page(
    candidates: &[crate::monitor::Candidate],
    cursor: Option<&str>,
    page_size: u16,
) -> Value {
    page(candidates, cursor, page_size, &"catalog", |items| {
        json!({
            "candidates": items, "active_limit": crate::battery::MAX_DEVICES,
            "max_parallel": crate::monitor::MAX_PARALLEL_QUERIES, "connection_verified": false,
        })
    })
}

pub fn devices_page(
    devices: &[Value],
    cursor: Option<&str>,
    page_size: u16,
    all: bool,
    selected: Option<&str>,
    refresh_pending: bool,
) -> Value {
    let scope = ("devices", all, selected, refresh_pending);
    page(devices, cursor, page_size, &scope, |items| {
        json!({
            "devices": items, "selected_id": selected, "refresh_pending": refresh_pending,
        })
    })
}

fn page<T: serde::Serialize>(
    entries: &[T],
    cursor: Option<&str>,
    page_size: u16,
    scope: &impl serde::Serialize,
    data: impl Fn(&[T]) -> Value,
) -> Value {
    use sha2::{Digest, Sha256};
    if !(1..=32).contains(&page_size) {
        return failure("invalid_arguments", "Page size must be between 1 and 32");
    }
    let revision: String = Sha256::digest(serde_json::to_vec(&(scope, entries)).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let start = if let Some(cursor) = cursor {
        let Some((rev, offset)) = cursor.split_once('.') else {
            return failure(
                "invalid_cursor",
                "Use next_cursor returned by the same command",
            );
        };
        if rev.len() != 64
            || !rev.bytes().all(|b| b.is_ascii_hexdigit())
            || offset.is_empty()
            || !offset.bytes().all(|b| b.is_ascii_digit())
        {
            return failure(
                "invalid_cursor",
                "Use next_cursor returned by the same command",
            );
        }
        if rev != revision {
            return failure(
                "stale_revision",
                "Snapshot changed; restart without a cursor",
            );
        }
        match offset.parse::<usize>() {
            Ok(n) if n < entries.len() => n,
            _ => return failure("invalid_cursor", "Cursor offset is outside the snapshot"),
        }
    } else {
        0
    };
    let response = |end: usize| {
        let mut snapshot = data(&entries[start..end]);
        snapshot["revision"] = json!(revision);
        snapshot["offset"] = json!(start);
        snapshot["next_cursor"] = json!((end < entries.len()).then(|| format!("{revision}.{end}")));
        snapshot["total"] = json!(entries.len());
        success(snapshot)
    };
    let mut end = start;
    while end < entries.len() && end - start < usize::from(page_size) {
        if serde_json::to_vec(&response(end + 1)).unwrap().len() > MAX_BYTES {
            break;
        }
        end += 1;
    }
    if end == start && start < entries.len() {
        return failure("response_too_large", "One entry exceeds the IPC byte limit");
    }
    response(end)
}
