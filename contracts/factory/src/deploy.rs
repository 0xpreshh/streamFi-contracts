use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, BytesN, Env, Symbol, Val, Vec};

use crate::errors::Error;
use crate::storage::DataKey;
use crate::{events, pause, ttl};

/// Derives a deterministic salt from the stream ID (Issue #684).
pub fn compute_salt(env: &Env, stream_id: u64) -> BytesN<32> {
    env.crypto()
        .sha256(&soroban_sdk::Bytes::from_array(
            env,
            &stream_id.to_be_bytes(),
        ))
        .into()
}

/// Derives a deterministic deployment salt from sender, recipient, nonce, and token (Issue #764).
pub fn derive_salt(
    env: &Env,
    sender: &Address,
    recipient: &Address,
    nonce: u64,
    token: &Address,
) -> BytesN<32> {
    let mut payload = soroban_sdk::Bytes::new(env);
    payload.append(&sender.to_xdr(env));
    payload.append(&recipient.to_xdr(env));
    payload.append(&soroban_sdk::Bytes::from_array(env, &nonce.to_be_bytes()));
    payload.append(&token.to_xdr(env));
    env.crypto().sha256(&payload).into()
}

pub fn deploy_stream_with_salt(
    env: &Env,
    wasm_hash: &BytesN<32>,
    salt: BytesN<32>,
    init_args: Vec<Val>,
) -> Result<Address, Error> {
    let salt_key = DataKey::SaltUsed(salt.clone());
    if env.storage().persistent().has(&salt_key) {
        return Err(Error::SaltAlreadyUsed);
    }

    // Step 1: deploy the WASM — no constructor called yet.
    let addr = env
        .deployer()
        .with_current_contract(salt)
        .deploy(wasm_hash.clone());

    env.storage().persistent().set(&salt_key, &true);

    // Step 2: call `initialize` on the freshly deployed contract.
    let _: () = env.invoke_contract(&addr, &Symbol::new(env, "initialize"), init_args);

    Ok(addr)
}

pub fn deploy_stream(
    env: &Env,
    wasm_hash: &BytesN<32>,
    stream_id: u64,
    init_args: Vec<Val>,
) -> Result<Address, Error> {
    // Derive a deterministic salt from the stream ID so each stream gets a
    // unique, reproducible contract address.
    let salt = compute_salt(env, stream_id);
    deploy_stream_with_salt(env, wasm_hash, salt, init_args)
}

/// Governance-guarded update of the WASM hash used for newly deployed streams.
///
/// Only the governor may call this. Streams created after the call deploy the
/// new code; streams that already exist are independent contracts and are
/// unaffected. Re-setting the current hash is a no-op and emits no event.
///
/// Checked in order: governor auth, non-zero hash, factory not paused.
/// Also bumps the instance and persistent-bucket TTLs, so the registry stays
/// alive during protocol-upgrade activity.
pub fn set_stream_wasm_hash(env: &Env, new_wasm_hash: &BytesN<32>) -> Result<(), Error> {
    let governor: Address = env
        .storage()
        .instance()
        .get(&DataKey::GovernorAddress)
        .ok_or(Error::NotInitialized)?;
    governor.require_auth();

    // A zero hash would deploy a no-op contract, burning funds sent to future
    // `create_stream` calls with no way to recover them.
    if *new_wasm_hash == BytesN::from_array(env, &[0u8; 32]) {
        return Err(Error::InvalidWasmHash);
    }

    // A paused factory accepts no state changes, even from the governor.
    pause::require_not_paused(env)?;

    let old_wasm_hash: BytesN<32> = env
        .storage()
        .instance()
        .get(&DataKey::StreamWasmHash)
        .ok_or(Error::NotInitialized)?;
    ttl::bump_instance(env);
    ttl::bump_persistent_bucket(env);
    if old_wasm_hash != *new_wasm_hash {
        env.storage()
            .instance()
            .set(&DataKey::StreamWasmHash, new_wasm_hash);
        events::stream_contract_upgraded(env, &old_wasm_hash, new_wasm_hash);
    }
    Ok(())
}
