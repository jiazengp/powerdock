#[test]
fn changed_containers_coalesce_drain_and_reject_late_updates() {
    let mut catalog = super::Catalog::new(1);
    catalog.add(1, false, "one".into(), ());
    catalog.update(1, false, "one".into(), |_| Ok(()));
    assert_eq!(catalog.take_changed(), Some(vec!["one".into()]));
    assert_eq!(catalog.take_changed(), Some(Vec::new()));
    catalog.remove(1, false, "one".into());
    assert_eq!(catalog.take_changed(), Some(vec!["one".into()]));
    catalog.retire();
    catalog.add(1, false, "late".into(), ());
    assert_eq!(catalog.take_changed(), Some(Vec::new()));
    let mut catalog = super::Catalog::<()>::new(2);
    for i in 0..=super::MAX_ENDPOINTS {
        catalog.mark_changed(i.to_string());
    }
    assert!(catalog.changed_ids.is_empty());
    assert_eq!(catalog.take_changed(), None);
    assert_eq!(catalog.take_changed(), Some(Vec::new()));
}

use super::*;
use std::cell::RefCell;

#[test]
fn patches_preserve_unspecified_fields_and_removal_is_transport_scoped() {
    let mut state = Catalog::new(7);
    state.add(7, true, "same".into(), RefCell::new((Some(42), Some(true))));
    state.add(
        7,
        false,
        "same".into(),
        RefCell::new((Some(81), Some(false))),
    );
    state.update(7, true, "same".into(), |item| {
        item.borrow_mut().1 = Some(false);
        Ok(())
    });
    assert_eq!(
        *state.entries[&(true, "same".into())].borrow(),
        (Some(42), Some(false))
    );
    state.remove(7, true, "same".into());
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries.contains_key(&(false, "same".into())));
    state.update(7, true, "same".into(), |_| {
        panic!("Removed cannot be resurrected by Updated")
    });
}

#[test]
fn stop_rejects_late_events_and_restart_uses_a_new_generation() {
    let mut state = Catalog::new(1);
    state.add(1, true, "a".into(), 1);
    state.retire();
    state.add(1, true, "b".into(), 2);
    state.remove(1, true, "a".into());
    state.update(1, true, "a".into(), |_| panic!("late patch"));
    state.finish(1, true);
    state.stopped(1, true);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.complete, [false; 2]);
    assert!(state.stopped[1]);
    let mut restarted = Catalog::new(2);
    restarted.add(1, true, "late".into(), 1);
    restarted.finish(1, true);
    restarted.stopped(1, true);
    assert!(restarted.entries.is_empty());
    assert_eq!(restarted.complete, [false; 2]);
    assert_eq!(restarted.stopped, [false; 2]);
    restarted.add(2, true, "new".into(), 2);
    restarted.finish(2, false);
    restarted.finish(2, true);
    assert_eq!(restarted.entries.len(), 1);
    assert_eq!(restarted.complete, [true; 2]);
}

#[test]
fn overflow_and_unexpected_stop_are_explicit_errors() {
    let mut state = Catalog::new(3);
    for id in 0..=MAX_ENDPOINTS {
        state.add(3, false, id.to_string(), id);
    }
    assert_eq!(state.entries.len(), MAX_ENDPOINTS);
    assert!(state.error.as_deref().unwrap().contains("truncated"));
    state.add(3, false, "0".into(), 99);
    assert_eq!(state.entries[&(false, "0".into())], 99);
    let mut stopped: Catalog<()> = Catalog::new(4);
    stopped.stopped(4, false);
    assert!(stopped.error.is_some());
}

#[test]
fn pnp_property_patches_keep_known_values_and_cannot_complete_bluetooth_enumeration() {
    let paired: Catalog<RefCell<(Option<u8>, Option<u8>)>> = Catalog::new(9);
    let mut containers = Catalog::new(9);
    containers.add(9, false, "container".into(), RefCell::new((Some(42), None)));
    // An Updated event containing only ChargingState must preserve BatteryLife.
    containers.update(9, false, "container".into(), |item| {
        item.borrow_mut().1 = Some(1);
        Ok(())
    });
    assert_eq!(
        *containers.entries[&(false, "container".into())].borrow(),
        (Some(42), Some(1))
    );
    containers.finish(9, false);
    assert_eq!(paired.complete, [false; 2]);
    assert!(
        paired.entries.is_empty(),
        "PnP metadata must not become a paired endpoint"
    );
    containers.retire();
    containers.update(9, false, "container".into(), |_| panic!("late PnP patch"));
    containers.remove(9, false, "container".into());
    assert_eq!(containers.entries.len(), 1);
}
