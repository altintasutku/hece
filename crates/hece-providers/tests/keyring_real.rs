//! Round trip through the real OS keyring. Ignored by default (CI has no keyring); run with
//! `cargo test -p hece-providers --test keyring_real -- --ignored`.
#![allow(clippy::unwrap_used, reason = "tests may unwrap")]

use hece_providers::{ApiKey, KeyringStore, SecretStore};

#[test]
#[ignore = "needs an unlocked OS keyring"]
fn set_get_delete_round_trip() {
    let account = "hece-selftest";
    let store = KeyringStore;
    store.set(account, &ApiKey::new("not-a-real-key")).unwrap();
    assert_eq!(
        store.get(account).unwrap().unwrap().expose(),
        "not-a-real-key"
    );
    store.delete(account).unwrap();
    assert!(store.get(account).unwrap().is_none());
    // Deleting a missing entry still succeeds.
    store.delete(account).unwrap();
}
