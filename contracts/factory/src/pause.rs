//! Emergency-pause flag storage and gating for `DripFactory`.
//!
//! Supports granular pause levels: `CreationOnly` and `FullEmergency` (Issue #767).

use soroban_sdk::{contracttype, Env};

use crate::errors::Error;
use crate::storage::DataKey;

/// Granular pause levels for creation vs withdrawal and emergency operations (Issue #767).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PauseType {
    Unpaused = 0,
    CreationOnly = 1,
    FullEmergency = 2,
}

/// Reads the current granular pause level.
pub fn pause_level(env: &Env) -> PauseType {
    if let Some(level) = env.storage().instance().get(&DataKey::PauseLevel) {
        level
    } else if drip_common::pause::is_paused(env, &DataKey::Paused) {
        PauseType::FullEmergency
    } else {
        PauseType::Unpaused
    }
}

/// Sets the granular pause level and updates the boolean flag in instance storage.
pub fn set_pause_level(env: &Env, level: PauseType) {
    env.storage().instance().set(&DataKey::PauseLevel, &level);
    let is_paused = level != PauseType::Unpaused;
    let _ = drip_common::pause::set_paused(env, &DataKey::Paused, is_paused);
}

/// Reads the emergency-pause flag.
///
/// Defaults to `false` (unpaused) when the key has never been set.
pub fn is_paused(env: &Env) -> bool {
    pause_level(env) != PauseType::Unpaused
}

/// Writes the emergency-pause flag directly to instance storage.
pub fn set_paused(env: &Env, paused: bool) {
    let level = if paused {
        PauseType::FullEmergency
    } else {
        PauseType::Unpaused
    };
    set_pause_level(env, level);
}

/// Rejects if stream creation is paused (either CreationOnly or FullEmergency).
pub fn require_creation_allowed(env: &Env) -> Result<(), Error> {
    match pause_level(env) {
        PauseType::Unpaused => Ok(()),
        PauseType::CreationOnly | PauseType::FullEmergency => Err(Error::ContractPaused),
    }
}

/// The pause gate: `Err(ContractPaused)` while the factory is halted in any pause level.
pub fn require_not_paused(env: &Env) -> Result<(), Error> {
    match pause_level(env) {
        PauseType::Unpaused => Ok(()),
        _ => Err(Error::ContractPaused),
    }
}

/// Rejects if full emergency pause is active, allowing operations during CreationOnly.
#[allow(dead_code)]
pub fn require_withdrawals_allowed(env: &Env) -> Result<(), Error> {
    match pause_level(env) {
        PauseType::FullEmergency => Err(Error::ContractPaused),
        _ => Ok(()),
    }
}
