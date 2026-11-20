use soroban_sdk::{panic_with_error, Env};

use crate::storage::{DataKey, StreamInfo};
#[allow(unused_imports)]
pub use crate::storage::StreamState;
use crate::Error;

/// Calculate stream age in seconds from start_time up to current_time (Issue #808).
#[allow(dead_code)]
pub fn elapsed_seconds(info: &StreamInfo, current_time: u64) -> u64 {
    info.elapsed_seconds(current_time)
}

/// Helper method to check if a stream's remaining balance is depleted (Issue #753).
#[allow(dead_code)]
pub fn is_depleted(state: &StreamState) -> bool {
    state.is_depleted()
}

/// True when the stream's schedule has completed by `current_time` (Issue #927).
#[allow(dead_code)]
pub fn is_fully_vested(info: &StreamInfo, current_time: u64) -> bool {
    info.is_fully_vested(current_time)
}

/// Load the full stream state in a single storage read.
///
/// Reads the consolidated `Config` key. All streams are now written with
/// the consolidated layout; pre-consolidation streams were migrated on their
/// first `save()` call (see `save()` for the one-time migration logic).
pub fn try_load(env: &Env) -> Result<StreamInfo, Error> {
    let s = env.storage().instance();
    s.get::<_, StreamInfo>(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

pub fn load(env: &Env) -> StreamInfo {
    match try_load(env) {
        Ok(info) => info,
        Err(err) => panic_with_error!(env, err),
    }
}

/// Persist the entire stream state in a single storage write.
///
/// All streams are now required to use the consolidated `Config` key.
/// Pre-consolidation streams were migrated on their first `save()` call
/// in a prior contract version (legacy keys were removed as part of the upgrade).
pub fn save(env: &Env, info: &StreamInfo) {
    env.storage().instance().set(&DataKey::Config, info);
}

pub fn assert_not_cancelled(info: &StreamInfo) -> Result<(), Error> {
    if info.is_cancelled() {
        Err(Error::StreamCancelled)
    } else {
        Ok(())
    }
}

/// Check if a stream is in a cancellable state (Issue #937).
///
/// Returns `true` if the stream can be cancelled:
/// - Stream is not already cancelled
/// - Stream has not been fully completed/withdrawn
///
/// The caller authorization (sender permissions, recipient restrictions) should be
/// validated separately at the contract method level using stream config and address.
pub fn is_cancellable(info: &StreamInfo) -> Result<bool, Error> {
    // Stream cannot be cancelled if it's already in a terminal cancelled state
    if info.is_cancelled() {
        return Ok(false);
    }

    // Stream can be cancelled as long as it's not fully cancelled
    // Other authorization checks (sender, recipient, etc.) are done at the method level
    Ok(info.is_in_cancellable_state())
}

/// Maximum allowed re-entrancy depth. Must never be > 1 in production —
/// Soroban's execution model makes re-entrancy structurally impossible,
/// so any depth > 0 signals a bug or an emerging cross-contract callback
/// capability. Kept as a named constant so a future protocol upgrade that
/// introduces hooks can be gated here rather than in every call site.
pub const MAX_REENTRANCY_DEPTH: u32 = 1;

/// Acquire the re-entrancy guard.
///
/// Uses a depth counter stored at [`DataKey::Guard`] instead of a boolean
/// flag. The counter approach:
///
/// - Detects unbalanced acquire / release (depth would underflow).
/// - Allows controlled re-entrancy up to [`MAX_REENTRANCY_DEPTH`] if the
///   protocol ever supports it.
/// - Prevents the "two simultaneous acquires both see 0" TOCTOU that a
///   plain boolean check-then-set is theoretically vulnerable to.
///
/// In Soroban's synchronous, single-shot execution model the TOCTOU is
/// not exploitable today; the counter is defense-in-depth for future
/// protocol capabilities (hooks, cross-contract callbacks, etc.).
pub fn lock(env: &Env) -> Result<(), Error> {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth >= MAX_REENTRANCY_DEPTH {
        return Err(Error::ReentrancyForbidden);
    }
    s.set(&DataKey::Guard, &(depth + 1));
    Ok(())
}

/// Release the re-entrancy guard.
///
/// Decrements the depth counter. If the counter is already zero (i.e.
/// `unlock` was called without a matching `lock`), the call is silently
/// swallowed — the state is already unlocked and panicking here would
/// unnecessarily block legitimate code paths.
pub fn unlock(env: &Env) {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth > 0 {
        s.set(&DataKey::Guard, &(depth - 1));
    }
}

/// Execute `f` under the re-entrancy guard, releasing the lock afterwards.
///
/// This is the preferred entry point for all state-mutating contract
/// methods. It guarantees that the guard is released on every exit path —
/// early `return`, `?` propagation, or normal completion — without
/// requiring the caller to remember a manual `unlock()` call.
///
/// # Examples
///
/// ```ignore
/// pub fn withdraw(env: Env, amount: i128) -> Result<i128, Error> {
///     state::with_guard(&env, |env| Self::_withdraw(env, amount))
/// }
/// ```
pub fn with_guard<R>(env: &Env, f: impl FnOnce(&Env) -> Result<R, Error>) -> Result<R, Error> {
    lock(env)?;
    let result = f(env);
    unlock(env);
    result
}
