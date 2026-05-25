// This file is Copyright its original authors, visible in version control
// history.
//
// This file is licensed under the Apache License, Version 2.0 <LICENSE-APACHE
// or http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// You may not use this file except in accordance with one or both of these
// licenses.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;

/// Minimal TlsConfig (subset of LDK Server's struct, only the fields we use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsConfig {
	pub cert_path: Option<String>,
	pub key_path: Option<String>,
	pub hosts: Vec<String>,
}

/// Write a file with the given content and mode, failing if the file already exists.
/// Mirrors LDK Server's `util::write_new` helper.
fn write_new(path: &Path, content: &[u8], mode: u32) -> std::io::Result<()> {
	let mut file =
		fs::OpenOptions::new().create_new(true).write(true).mode(mode).open(path)?;
	file.write_all(content)?;
	fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
	file.sync_all()?;
	Ok(())
}

// Issuer and Subject common name
const ISSUER_NAME: &str = "localhost";

// PEM markers
const PEM_CERT_BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const PEM_CERT_END: &str = "-----END CERTIFICATE-----";
const PEM_KEY_BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_KEY_END: &str = "-----END PRIVATE KEY-----";

// OIDs
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01]; // 1.2.840.10045.2.1
const OID_PRIME256V1: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07]; // 1.2.840.10045.3.1.7
const OID_ECDSA_WITH_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02]; // 1.2.840.10045.4.3.2
const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03]; // 2.5.4.3
const OID_SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1D, 0x11]; // 2.5.29.17

// DER tag constants (universal class, bits 7-6 = 00)
const TAG_INTEGER: u8 = 0x02;
const TAG_BIT_STRING: u8 = 0x03;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_OID: u8 = 0x06;
const TAG_UTF8_STRING: u8 = 0x0C;
const TAG_UTC_TIME: u8 = 0x17;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_SET: u8 = 0x31;

/// Gets or generates TLS configuration. If custom paths are provided, uses those.
/// Otherwise, generates a self-signed certificate in the storage directory.
pub fn get_or_generate_tls_config(
	tls_config: Option<TlsConfig>, storage_dir: &str,
) -> Result<ServerConfig, String> {
	if let Some(config) = tls_config {
		let cert_path = config.cert_path.unwrap_or(format!("{storage_dir}/tls.crt"));
		let key_path = config.key_path.unwrap_or(format!("{storage_dir}/tls.key"));
		if !fs::exists(&cert_path).unwrap_or(false) || !fs::exists(&key_path).unwrap_or(false) {
			generate_self_signed_cert(&cert_path, &key_path, &config.hosts)?;
		}
		load_tls_config(&cert_path, &key_path)
	} else {
		// Check if we already have generated certs, if we don't, generate new ones
		let cert_path = format!("{storage_dir}/tls.crt");
		let key_path = format!("{storage_dir}/tls.key");
		if !fs::exists(&cert_path).unwrap_or(false) || !fs::exists(&key_path).unwrap_or(false) {
			generate_self_signed_cert(&cert_path, &key_path, &[])?;
		}

		load_tls_config(&cert_path, &key_path)
	}
}

/// Parses a PEM-encoded certificate file and returns the DER-encoded certificates.
fn parse_pem_certs(pem_data: &str) -> Result<Vec<CertificateDer<'static>>, String> {
	let mut certs = Vec::new();

	for block in pem_data.split(PEM_CERT_END) {
		if let Some(start) = block.find(PEM_CERT_BEGIN) {
			let base64_content: String = block[start + PEM_CERT_BEGIN.len()..]
				.lines()
				.filter(|line| !line.starts_with("-----") && !line.is_empty())
				.collect();

			let der = base64::engine::general_purpose::STANDARD
				.decode(&base64_content)
				.map_err(|e| format!("Failed to decode certificate base64: {e}"))?;

			certs.push(CertificateDer::from(der));
		}
	}

	Ok(certs)
}

/// Parses a PEM-encoded PKCS#8 private key file and returns the DER-encoded key.
fn parse_pem_private_key(pem_data: &str) -> Result<PrivateKeyDer<'static>, String> {
	let start = pem_data.find(PEM_KEY_BEGIN).ok_or("Missing BEGIN PRIVATE KEY marker")?;
	let end = pem_data.find(PEM_KEY_END).ok_or("Missing END PRIVATE KEY marker")?;

	let base64_content: String = pem_data[start + PEM_KEY_BEGIN.len()..end]
		.lines()
		.filter(|line| !line.starts_with("-----") && !line.is_empty())
		.collect();

	let der = base64::engine::general_purpose::STANDARD
		.decode(&base64_content)
		.map_err(|e| format!("Failed to decode private key base64: {e}"))?;

	Ok(PrivateKeyDer::Pkcs8(der.into()))
}

/// Generates a self-signed TLS certificate and saves it to the storage directory.
/// Returns the paths to the generated cert and key files.
fn generate_self_signed_cert(
	cert_path: &str, key_path: &str, configure_hosts: &[String],
) -> Result<(), String> {
	let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string()];
	hosts.extend_from_slice(configure_hosts);

	let rng = SystemRandom::new();

	// Generate ECDSA P-256 key pair
	let pkcs8_doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
		.map_err(|e| format!("Failed to generate key pair: {e}"))?;

	let key_pair =
		EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8_doc.as_ref(), &rng)
			.map_err(|e| format!("Failed to parse generated key pair: {e}"))?;

	// Build the certificate
	let cert_der = build_self_signed_cert(&key_pair, &hosts, &rng)?;

	// Convert to PEM format
	let cert_pem = der_to_pem(&cert_der, PEM_CERT_BEGIN, PEM_CERT_END);
	let key_pem = der_to_pem(pkcs8_doc.as_ref(), PEM_KEY_BEGIN, PEM_KEY_END);

	write_new(Path::new(key_path), key_pem.as_bytes(), 0o400)
		.map_err(|e| format!("Failed to write TLS key to '{key_path}': {e}"))?;
	fs::write(cert_path, &cert_pem)
		.map_err(|e| format!("Failed to write TLS certificate to '{cert_path}': {e}"))?;

	Ok(())
}

fn der_to_pem(der: &[u8], begin: &str, end: &str) -> String {
	let b64 = base64::engine::general_purpose::STANDARD.encode(der);
	let lines: Vec<&str> =
		b64.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
	format!("{begin}\n{}\n{end}\n", lines.join("\n"))
}

/// Build a self-signed X.509 certificate
fn build_self_signed_cert(
	key_pair: &EcdsaKeyPair, hosts: &[String], rng: &SystemRandom,
) -> Result<Vec<u8>, String> {
	// Build TBSCertificate
	let tbs_cert = build_tbs_certificate(key_pair, hosts)?;

	// Sign the TBSCertificate
	let signature = key_pair.sign(rng, &tbs_cert).map_err(|_| "Failed to sign certificate")?;

	// Build final Certificate structure: the signed certificate data, the algorithm used
	// to sign it, and the signature itself.
	// Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
	let sig_alg_oid = der_oid(OID_ECDSA_WITH_SHA256);
	let sig_alg = der_sequence(&sig_alg_oid);
	let sig_value = der_bit_string(signature.as_ref());

	let cert_content = [tbs_cert, sig_alg, sig_value].concat();
	let cert = der_sequence(&cert_content);

	Ok(cert)
}

/// Builds the TBSCertificate (To Be Signed Certificate) structure per RFC 5280.
///
/// This is the core certificate data that gets signed. The structure contains:
/// - Version (v3)
/// - Serial number (fixed to 1)
/// - Signature algorithm identifier
/// - Issuer and subject distinguished names (both set to CN=localhost)
/// - Validity period (2026-01-01 to 2049-12-31)
/// - Subject public key info
/// - Extensions (Subject Alternative Names for the provided hosts)
fn build_tbs_certificate(key_pair: &EcdsaKeyPair, hosts: &[String]) -> Result<Vec<u8>, String> {
	// version [0] EXPLICIT INTEGER { v3(2) }
	let version = der_context_explicit(0, &der_integer(&[2]));

	// serialNumber INTEGER (fixed to 1)
	let serial_number = der_integer(&[1]);

	// signature AlgorithmIdentifier (ECDSA with SHA-256)
	let sig_alg_oid = der_oid(OID_ECDSA_WITH_SHA256);
	let signature_alg = der_sequence(&sig_alg_oid);

	// issuer Name (CN=localhost)
	let issuer = build_name(ISSUER_NAME);

	// validity (2026-01-01 00:00:00Z to 2049-12-31 23:59:59Z)
	// UTCTime format: YYMMDDHHMMSSZ (years 00-49 = 2000-2049, years 50-99 = 1950-1999)
	let validity =
		der_sequence(&[der_utc_time("260101000000Z"), der_utc_time("491231235959Z")].concat());

	// subject Name (same as issuer for self-signed)
	let subject = build_name(ISSUER_NAME);

	// subjectPublicKeyInfo
	let spki = build_subject_public_key_info(key_pair);

	// extensions [3] EXPLICIT Extensions
	let extensions = build_extensions(hosts)?;
	let extensions_explicit = der_context_explicit(3, &extensions);

	let tbs_content = [
		version,
		serial_number,
		signature_alg,
		issuer,
		validity,
		subject,
		spki,
		extensions_explicit,
	]
	.concat();

	Ok(der_sequence(&tbs_content))
}

fn build_name(cn: &str) -> Vec<u8> {
	// A Name (like "CN=ldk-server") is a list of relative distinguished names (RDNs).
	// Each RDN is a set of attribute-value pairs (e.g., commonName = "ldk-server").
	// Name ::= SEQUENCE OF RelativeDistinguishedName
	// RDN ::= SET OF AttributeTypeAndValue
	// AttributeTypeAndValue ::= SEQUENCE { type OID, value ANY }
	let cn_attr = der_sequence(&[der_oid(OID_COMMON_NAME), der_utf8_string(cn)].concat());
	let rdn = der_set(&cn_attr);
	der_sequence(&rdn)
}

fn build_subject_public_key_info(key_pair: &EcdsaKeyPair) -> Vec<u8> {
	// Contains the public key and identifies what type of key it is (EC using P-256 curve).
	// SubjectPublicKeyInfo ::= SEQUENCE {
	//   algorithm AlgorithmIdentifier,
	//   subjectPublicKey BIT STRING
	// }
	let algorithm = der_sequence(&[der_oid(OID_EC_PUBLIC_KEY), der_oid(OID_PRIME256V1)].concat());

	let public_key = key_pair.public_key().as_ref();
	let public_key_bits = der_bit_string(public_key);

	der_sequence(&[algorithm, public_key_bits].concat())
}

fn build_extensions(hosts: &[String]) -> Result<Vec<u8>, String> {
	// Extensions add optional features to the certificate. Each extension has an OID
	// identifying what it is, an optional critical flag, and the extension data.
	// Extensions ::= SEQUENCE OF Extension
	// Extension ::= SEQUENCE { extnID OID, critical BOOLEAN DEFAULT FALSE, extnValue OCTET STRING }

	// Build Subject Alternative Name extension
	let san_ext = build_san_extension(hosts)?;

	Ok(der_sequence(&san_ext))
}

fn build_san_extension(hosts: &[String]) -> Result<Vec<u8>, String> {
	// Subject Alternative Name (SAN) lists the hostnames/IPs the certificate is valid for.
	// Each entry is either a DNS name (like "localhost") or an IP address.
	// GeneralNames ::= SEQUENCE OF GeneralName
	// GeneralName ::= CHOICE {
	//   dNSName      [2] IA5String,
	//   iPAddress    [7] OCTET STRING
	// }
	let mut general_names = Vec::new();

	for host in hosts {
		if let Ok(ip) = host.parse::<IpAddr>() {
			// IP address - tag [7]
			let ip_bytes = match ip {
				IpAddr::V4(v4) => v4.octets().to_vec(),
				IpAddr::V6(v6) => v6.octets().to_vec(),
			};
			general_names.extend(der_context_implicit(7, &ip_bytes));
		} else {
			// DNS name - tag [2]
			general_names.extend(der_context_implicit(2, host.as_bytes()));
		}
	}

	let san_value = der_sequence(&general_names);
	let san_octet = der_octet_string(&san_value);

	// Extension ::= SEQUENCE { extnID, extnValue }
	// (critical is omitted since it defaults to FALSE)
	Ok(der_sequence(&[der_oid(OID_SUBJECT_ALT_NAME), san_octet].concat()))
}

// DER encoding helpers

fn der_length_size(len: usize) -> usize {
	if len < 128 {
		1
	} else if len < 256 {
		2
	} else if len < 65536 {
		3
	} else {
		4
	}
}

fn der_tag_length_value(tag: u8, value: &[u8]) -> Vec<u8> {
	let len = value.len();
	let len_size = der_length_size(len);
	let mut result = Vec::with_capacity(1 + len_size + len);

	result.push(tag);

	// Encode length using DER rules:
	// - Short form (len < 128): single byte with the length value
	// - Long form (len >= 128): first byte is 0x80 | number_of_length_bytes,
	//   followed by the length in big-endian
	if len < 128 {
		result.push(len as u8);
	} else if len < 256 {
		result.push(0x81); // 1 length byte follows
		result.push(len as u8);
	} else if len < 65536 {
		result.push(0x82); // 2 length bytes follow
		result.push((len >> 8) as u8);
		result.push(len as u8);
	} else {
		result.push(0x83); // 3 length bytes follow
		result.push((len >> 16) as u8);
		result.push((len >> 8) as u8);
		result.push(len as u8);
	}

	result.extend_from_slice(value);
	result
}

fn der_sequence(content: &[u8]) -> Vec<u8> {
	der_tag_length_value(TAG_SEQUENCE, content)
}

fn der_set(content: &[u8]) -> Vec<u8> {
	der_tag_length_value(TAG_SET, content)
}

fn der_integer(value: &[u8]) -> Vec<u8> {
	// If high bit is set, prepend 0x00 to indicate positive
	if !value.is_empty() && value[0] & 0x80 != 0 {
		let mut padded = Vec::with_capacity(1 + value.len());
		padded.push(0x00);
		padded.extend_from_slice(value);
		der_tag_length_value(TAG_INTEGER, &padded)
	} else {
		der_tag_length_value(TAG_INTEGER, value)
	}
}

fn der_bit_string(value: &[u8]) -> Vec<u8> {
	// BIT STRING: first byte is number of unused bits (0 for us)
	let mut content = Vec::with_capacity(1 + value.len());
	content.push(0x00);
	content.extend_from_slice(value);
	der_tag_length_value(TAG_BIT_STRING, &content)
}

fn der_octet_string(value: &[u8]) -> Vec<u8> {
	der_tag_length_value(TAG_OCTET_STRING, value)
}

fn der_oid(oid: &[u8]) -> Vec<u8> {
	der_tag_length_value(TAG_OID, oid)
}

fn der_utf8_string(s: &str) -> Vec<u8> {
	der_tag_length_value(TAG_UTF8_STRING, s.as_bytes())
}

fn der_utc_time(s: &str) -> Vec<u8> {
	der_tag_length_value(TAG_UTC_TIME, s.as_bytes())
}

fn der_context_explicit(tag_num: u8, content: &[u8]) -> Vec<u8> {
	// 0xA0 = context-specific class, constructed
	der_tag_length_value(0xA0 | tag_num, content)
}

fn der_context_implicit(tag_num: u8, content: &[u8]) -> Vec<u8> {
	// 0x80 = context-specific class, primitive
	der_tag_length_value(0x80 | tag_num, content)
}

/// Loads TLS configuration from provided paths.
fn load_tls_config(cert_path: &str, key_path: &str) -> Result<ServerConfig, String> {
	let cert_pem = fs::read_to_string(cert_path)
		.map_err(|e| format!("Failed to read TLS certificate file '{cert_path}': {e}"))?;
	let key_pem = fs::read_to_string(key_path)
		.map_err(|e| format!("Failed to read TLS key file '{key_path}': {e}"))?;

	let certs = parse_pem_certs(&cert_pem)?;

	if certs.is_empty() {
		return Err("No certificates found in certificate file".to_string());
	}

	let key = parse_pem_private_key(&key_pem)?;

	let mut config = ServerConfig::builder()
		.with_no_client_auth()
		.with_single_cert(certs, key)
		.map_err(|e| format!("Failed to build TLS server config: {e}"))?;
	config.alpn_protocols = vec![b"h2".to_vec()];
	Ok(config)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_pem_certs() {
		let pem = "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJAKHBfpegPjMCMA0GCSqGSIb3DQEBCwUAMBExDzANBgNVBAMMBnVu\ndXNlZDAeFw0yMzAxMDEwMDAwMDBaFw0yNDAxMDEwMDAwMDBaMBExDzANBgNVBAMM\nBnVudXNlZDBcMA0GCSqGSIb3DQEBAQUAA0sAMEgCQQC7o96FCEcJsggt0c0dSfEB\nmm6vv1LdCoxXnhOSCutoJgJgmCPBjU1doFFKwAtXjfOv0eSLZ3NHLu0LRKmVvOsP\nAgMBAAGjUzBRMB0GA1UdDgQWBBQK3fc0myO0psd71FJd8v7VCmDJOzAfBgNVHSME\nGDAWgBQK3fc0myO0psd71FJd8v7VCmDJOzAPBgNVHRMBAf8EBTADAQH/MA0GCSqG\nSIb3DQEBCwUAA0EAhJg0cx2pFfVfGBfbJQNFa+A4ynJBMqKYlbUnJBfWPwg13RhC\nivLjYyhKzEbnOug0TuFfVaUBGfBYbPgaJQ4BAg==\n-----END CERTIFICATE-----\n";

		let certs = parse_pem_certs(pem).unwrap();
		assert_eq!(certs.len(), 1);
		assert!(!certs[0].is_empty());
	}

	#[test]
	fn test_parse_pem_certs_multiple() {
		let pem = "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJAKHBfpegPjMCMA0GCSqGSIb3DQEBCwUAMBExDzANBgNVBAMMBnVu\ndXNlZDAeFw0yMzAxMDEwMDAwMDBaFw0yNDAxMDEwMDAwMDBaMBExDzANBgNVBAMM\nBnVudXNlZDBcMA0GCSqGSIb3DQEBAQUAA0sAMEgCQQC7o96FCEcJsggt0c0dSfEB\nmm6vv1LdCoxXnhOSCutoJgJgmCPBjU1doFFKwAtXjfOv0eSLZ3NHLu0LRKmVvOsP\nAgMBAAGjUzBRMB0GA1UdDgQWBBQK3fc0myO0psd71FJd8v7VCmDJOzAfBgNVHSME\nGDAWgBQK3fc0myO0psd71FJd8v7VCmDJOzAPBgNVHRMBAf8EBTADAQH/MA0GCSqG\nSIb3DQEBCwUAA0EAhJg0cx2pFfVfGBfbJQNFa+A4ynJBMqKYlbUnJBfWPwg13RhC\nivLjYyhKzEbnOug0TuFfVaUBGfBYbPgaJQ4BAg==\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJAKHBfpegPjMCMA0GCSqGSIb3DQEBCwUAMBExDzANBgNVBAMMBnVu\ndXNlZDAeFw0yMzAxMDEwMDAwMDBaFw0yNDAxMDEwMDAwMDBaMBExDzANBgNVBAMM\nBnVudXNlZDBcMA0GCSqGSIb3DQEBAQUAA0sAMEgCQQC7o96FCEcJsggt0c0dSfEB\nmm6vv1LdCoxXnhOSCutoJgJgmCPBjU1doFFKwAtXjfOv0eSLZ3NHLu0LRKmVvOsP\nAgMBAAGjUzBRMB0GA1UdDgQWBBQK3fc0myO0psd71FJd8v7VCmDJOzAfBgNVHSME\nGDAWgBQK3fc0myO0psd71FJd8v7VCmDJOzAPBgNVHRMBAf8EBTADAQH/MA0GCSqG\nSIb3DQEBCwUAA0EAhJg0cx2pFfVfGBfbJQNFa+A4ynJBMqKYlbUnJBfWPwg13RhC\nivLjYyhKzEbnOug0TuFfVaUBGfBYbPgaJQ4BAg==\n-----END CERTIFICATE-----\n";

		let certs = parse_pem_certs(pem).unwrap();
		assert_eq!(certs.len(), 2);
	}

	#[test]
	fn test_parse_pem_certs_empty() {
		let certs = parse_pem_certs("").unwrap();
		assert!(certs.is_empty());

		let certs = parse_pem_certs("not a cert").unwrap();
		assert!(certs.is_empty());
	}

	#[test]
	fn test_parse_pem_private_key_pkcs8() {
		let pem = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg2a2rwplBQLzHPDvn\nsaw8HKDP6WYBSF684gcz+D7zeVShRANCAAQq8R/E45tTNWMEpK8abYM7VzuJxpPS\nhJCi6bzjOPGHawEO8safLOWFaV7GqLJM0OdM3eu/qcz8HwgI3T8EVHQK\n-----END PRIVATE KEY-----\n";

		let key = parse_pem_private_key(pem).unwrap();
		assert!(matches!(key, PrivateKeyDer::Pkcs8(_)));
	}

	#[test]
	fn test_parse_pem_private_key_invalid() {
		let result = parse_pem_private_key("");
		assert!(result.is_err());

		let result = parse_pem_private_key("not a key");
		assert!(result.is_err());
	}

	#[test]
	fn test_generate_and_load_roundtrip() {
		// Both ring and aws-lc-rs may be present as transitive deps, so we must
		// explicitly install the ring provider before calling load_tls_config
		// (which internally constructs a rustls ServerConfig).
		let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();

		let temp_dir = std::env::temp_dir();
		let mut suffix_bytes = [0u8; 8];
		getrandom::getrandom(&mut suffix_bytes).unwrap();
		let suffix = u64::from_ne_bytes(suffix_bytes);
		let cert_path = temp_dir.join(format!("test_tls_cert_{suffix}.pem"));
		let key_path = temp_dir.join(format!("test_tls_key_{suffix}.pem"));

		// Clean up any existing files to be safe
		let _ = fs::remove_file(&cert_path);
		let _ = fs::remove_file(&key_path);

		// Generate cert
		generate_self_signed_cert(cert_path.to_str().unwrap(), key_path.to_str().unwrap(), &[])
			.unwrap();

		// Verify files exist
		assert!(cert_path.exists());
		assert!(key_path.exists());

		// Load config
		let res = load_tls_config(cert_path.to_str().unwrap(), key_path.to_str().unwrap());
		assert!(res.is_ok());

		// Clean up
		let _ = fs::remove_file(&cert_path);
		let _ = fs::remove_file(&key_path);
	}
}
