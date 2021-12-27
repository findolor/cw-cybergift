use crate::msg::ClaimMsg;
use crate::state::{Config, CONFIG, MERKLE_ROOT};
use crate::ContractError;
use anyhow::Result;
use cosmwasm_std::{
    from_binary, Binary, Coin, Deps, DepsMut, MessageInfo, Record, StdError, StdResult, Uint128,
    Uint64, VerificationError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use sha2::Digest;
use sha3::Keccak256;
use std::convert::TryInto;

pub fn update_coefficient(deps: DepsMut, amount: Uint128, config: &mut Config) -> StdResult<()> {
    let coefficient_up = config.coefficient_up;
    let coefficient_down = config.coefficient_down;
    let initial_balance = config.initial_balance;
    let current_balance = config.current_balance;

    let new_coefficient =
        coefficient_up + (coefficient_down - coefficient_up) * initial_balance / current_balance;

    config.coefficient = new_coefficient;
    config.current_balance = current_balance - amount;
    CONFIG.save(deps.storage, &config)
}

pub fn verify_merkle_proof(
    deps: &DepsMut,
    info: &MessageInfo,
    amount: Uint128,
    proof: Vec<String>,
) -> Result<bool, ContractError> {
    let merkle_root = MERKLE_ROOT.load(deps.storage)?;

    let user_input = format!("{}{}", info.sender, amount);
    let hash = sha2::Sha256::digest(user_input.as_bytes())
        .as_slice()
        .try_into()
        .map_err(|_| ContractError::WrongLength {})?;

    let hash = proof.into_iter().try_fold(hash, |hash, p| {
        let mut proof_buf = [0; 32];
        hex::decode_to_slice(p, &mut proof_buf)?;
        let mut hashes = [hash, proof_buf];
        hashes.sort_unstable();
        sha2::Sha256::digest(&hashes.concat())
            .as_slice()
            .try_into()
            .map_err(|_| ContractError::WrongLength {})
    })?;

    let mut root_buf: [u8; 32] = [0; 32];
    hex::decode_to_slice(merkle_root, &mut root_buf)?;
    if root_buf != hash {
        return Err(StdError::verification_err(VerificationError::GenericErr {}).into());
    }
    Ok(true)
}

pub fn verify_eth(
    deps: Deps,
    claim_msg: &ClaimMsg,
    signature: Binary,
) -> Result<bool, ContractError> {
    let mut hasher = Keccak256::new();
    let msg = to_string(&claim_msg).map_err(|err| ContractError::InvalidInput {})?;
    hasher.update(format!("\x19Ethereum Signed Message:\n{}", msg.len()));
    hasher.update(msg);
    let hash = hasher.finalize();
    // Decompose signature
    let (v, rs) = match signature.split_last() {
        Some(pair) => pair,
        None => {
            return Err(ContractError::IsNotEligible {
                msg: "Signature must not be empty".to_string(),
            })
        }
    };
    let recovery = get_recovery_param(*v)?;

    // Verification
    let calculated_pubkey = deps.api.secp256k1_recover_pubkey(&hash, rs, recovery)?;
    let calculated_address = ethereum_address_raw(&calculated_pubkey)?;
    if claim_msg.gift_claiming_address.as_bytes() != calculated_address {
        return Err(ContractError::IsNotEligible {
            msg: "signer address is not calculated addr".to_string(),
        });
    }
    deps.api
        .secp256k1_verify(&hash, rs, &calculated_pubkey)
        .map_err(|err| ContractError::IsNotEligible {
            msg: err.to_string(),
        })
}

fn get_recovery_param(v: u8) -> StdResult<u8> {
    match v {
        27 => Ok(0),
        28 => Ok(1),
        _ => Err(StdError::generic_err("Values of v other than 27 and 28 not supported. Replay protection (EIP-155) cannot be used here."))
    }
}

/// Returns a raw 20 byte Ethereum address
fn ethereum_address_raw(pubkey: &[u8]) -> StdResult<[u8; 20]> {
    let (tag, data) = match pubkey.split_first() {
        Some(pair) => pair,
        None => return Err(StdError::generic_err("Public key must not be empty")),
    };
    if *tag != 0x04 {
        return Err(StdError::generic_err("Public key must start with 0x04"));
    }
    if data.len() != 64 {
        return Err(StdError::generic_err("Public key must be 65 bytes long"));
    }

    let hash = Keccak256::digest(data);
    Ok(hash[hash.len() - 20..].try_into().unwrap())
}

pub fn verify_cosmos(
    deps: Deps,
    claim_msg: &ClaimMsg,
    signature: Binary,
) -> Result<bool, ContractError> {
    Ok(true)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Tx {
    pub chain_id: String,
    pub account_number: Uint64,
    pub sequence: Uint64,
    pub fee: Fee,
    pub msgs: Vec<Msg>,
    pub memo: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Fee {
    pub gas: Uint128,
    pub amount: Coin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Msg {
    pub signer: String,
    pub data: Binary,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Signature {
    pub pub_key: String,
    pub signature: Binary,
}