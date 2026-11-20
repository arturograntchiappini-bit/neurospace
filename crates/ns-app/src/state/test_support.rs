//! Helpers shared by the tests of the state modules.

use ns_core::{IndexDelta, LoadCache, Vault};

/// Everything on disk, as the first reload of a reloader sees it.
pub fn full_delta(vault: &Vault) -> IndexDelta {
    IndexDelta::parse_complete(vault.load_changes(&mut LoadCache::default()).unwrap())
}
