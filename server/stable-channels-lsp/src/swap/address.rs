//! Refund-address validation per rail, ported from OpenReceive's `core/src/swap/address.ts`.
//! These are checksum checks, not shape guards: a refund address is typed or pasted by the user,
//! and a false accept sends the deposit somewhere unrecoverable.
//! - Tron: Base58Check, 25 bytes, mainnet `0x41` prefix, double-SHA-256 checksum.
//! - Ethereum: `0x` + 40 hex; EIP-55 verified whenever the address carries mixed case.
//! - Solana: base58, 32–44 chars, decoding to exactly a 32-byte ed25519 public key.

use bitcoin_hashes::{sha256d, Hash};
use sha3::{Digest, Keccak256};

use super::PayInAsset;

const BASE58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const TRON_ADDRESS_PREFIX: u8 = 0x41;
const MAX_ADDRESS_LEN: usize = 200;

/// User-facing reason `address` cannot receive a refund on this rail, or None when it can.
pub fn refund_address_error(asset: PayInAsset, address: &str) -> Option<String> {
    let label = asset.network_label();
    if address.is_empty()
        || address.len() > MAX_ADDRESS_LEN
        || address.contains(char::is_whitespace)
    {
        return Some(format!("That doesn't look like a {label} address."));
    }
    let valid = match asset.network() {
        "TRX" => is_valid_tron(address),
        "ETH" => is_valid_ethereum(address),
        _ => is_valid_solana(address),
    };
    if valid {
        return None;
    }
    Some(match asset.network() {
        "ETH" if eth_shape(address) => {
            format!("That {label} address failed its checksum. Copy it again from your wallet.")
        }
        "ETH" => format!("That doesn't look like an {label} address. Use a 0x address."),
        "TRX" if tron_shape(address) => {
            format!("That {label} address failed its checksum. Copy it again from your wallet.")
        }
        "TRX" => {
            format!("That doesn't look like a {label} address. Use an address starting with T.")
        }
        _ => {
            format!("That doesn't look like a {label} address. Check you pasted the full address.")
        }
    })
}

/// Bitcoin-alphabet base58 decode; leading '1's are leading zero bytes. None on a bad character.
fn decode_base58(value: &str) -> Option<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new(); // little-endian
    for c in value.bytes() {
        let mut carry = BASE58_ALPHABET.iter().position(|&a| a == c)? as u32;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    bytes.extend(value.bytes().take_while(|&c| c == b'1').map(|_| 0));
    bytes.reverse();
    Some(bytes)
}

fn is_base58(s: &str) -> bool {
    s.bytes().all(|c| BASE58_ALPHABET.contains(&c))
}

fn tron_shape(s: &str) -> bool {
    s.len() == 34 && s.starts_with('T') && is_base58(s)
}

fn is_valid_tron(s: &str) -> bool {
    if !tron_shape(s) {
        return false;
    }
    let Some(decoded) = decode_base58(s) else {
        return false;
    };
    if decoded.len() != 25 || decoded[0] != TRON_ADDRESS_PREFIX {
        return false;
    }
    sha256d::Hash::hash(&decoded[..21]).to_byte_array()[..4] == decoded[21..]
}

fn is_valid_solana(s: &str) -> bool {
    (32..=44).contains(&s.len()) && is_base58(s) && decode_base58(s).is_some_and(|d| d.len() == 32)
}

fn eth_shape(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_ethereum(s: &str) -> bool {
    if !eth_shape(s) {
        return false;
    }
    let body = &s[2..];
    let lower = body.to_ascii_lowercase();
    // No mixed case means no EIP-55 bits to verify: accept on shape, as Ethereum wallets do.
    if body == lower || body == body.to_ascii_uppercase() {
        return true;
    }
    let digest = Keccak256::digest(lower.as_bytes());
    body.bytes()
        .zip(lower.bytes())
        .enumerate()
        .all(|(i, (actual, l))| {
            if !l.is_ascii_alphabetic() {
                return true;
            }
            let nibble = if i % 2 == 0 {
                digest[i / 2] >> 4
            } else {
                digest[i / 2] & 0x0f
            };
            (nibble >= 8) == actual.is_ascii_uppercase()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRON: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
    const SOL: &str = "So11111111111111111111111111111111111111112";
    const ETH_EIP55: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";

    #[test]
    fn valid_addresses_pass() {
        assert_eq!(refund_address_error(PayInAsset::UsdtTron, TRON), None);
        assert_eq!(refund_address_error(PayInAsset::UsdtSol, SOL), None);
        assert_eq!(
            refund_address_error(PayInAsset::UsdcSol, "11111111111111111111111111111111"),
            None
        );
        assert_eq!(refund_address_error(PayInAsset::UsdtEth, ETH_EIP55), None);
        assert_eq!(
            refund_address_error(PayInAsset::UsdcEth, &ETH_EIP55.to_lowercase()),
            None
        );
    }

    #[test]
    fn wrong_chain_addresses_are_rejected() {
        // Each rail refuses the other chains' valid addresses.
        assert!(refund_address_error(PayInAsset::UsdtTron, ETH_EIP55)
            .unwrap()
            .contains("starting with T"));
        assert!(refund_address_error(PayInAsset::UsdtTron, SOL).is_some());
        assert!(refund_address_error(PayInAsset::UsdtEth, TRON)
            .unwrap()
            .contains("0x address"));
        assert!(refund_address_error(PayInAsset::UsdtEth, SOL).is_some());
        // A Tron address is base58 of 25 bytes, not a 32-byte Solana key.
        assert!(refund_address_error(PayInAsset::UsdtSol, TRON).is_some());
        assert!(refund_address_error(PayInAsset::UsdcSol, ETH_EIP55).is_some());
    }

    #[test]
    fn checksum_and_shape_failures_are_rejected() {
        // One character changed: shape still Tron, Base58Check fails.
        let typo = TRON.replace("q8ZY", "q8ZZ");
        assert!(refund_address_error(PayInAsset::UsdtTron, &typo)
            .unwrap()
            .contains("checksum"));
        // EIP-55 case flipped on one letter.
        let bad_case = ETH_EIP55.replacen("aAeb", "aaeb", 1);
        assert!(refund_address_error(PayInAsset::UsdtEth, &bad_case)
            .unwrap()
            .contains("checksum"));
        // Truncated Solana paste, non-base58 character, whitespace, empty.
        assert!(refund_address_error(PayInAsset::UsdtSol, &SOL[..30]).is_some());
        assert!(refund_address_error(PayInAsset::UsdtSol, &SOL.replace('S', "0")).is_some());
        assert!(refund_address_error(PayInAsset::UsdtTron, &format!(" {TRON}")).is_some());
        assert!(refund_address_error(PayInAsset::UsdtTron, "").is_some());
    }
}
