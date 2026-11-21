//! PKCS#12 certificate files (`.p12`/`.pfx`) for digital signing (design D9).
//!
//! Read with RustCrypto's `pkcs12` structures: modern PBES2/AES and legacy 3DES/RC2 bags,
//! HMAC integrity check, RSA 2048–8192 and ECDSA P-256/P-384 keys. Every buffer that holds
//! decrypted key material is `Zeroizing`, and the key types clear themselves on drop.
//! Errors never contain the password or key bytes.

use std::time::SystemTime;

use der::asn1::{ObjectIdentifier, OctetString};
use der::{Decode, Encode};
use pkcs12::kdf::{Pkcs12KeyType, derive_key_utf8};
use sha2::Digest as _;
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::ext::pkix::KeyUsage;
use zeroize::Zeroizing;

use crate::secret::Secret;
use crate::{PdfError, guard};

/// Largest certificate file read.
pub const MAX_P12_BYTES: usize = 1 << 20;
/// Certificates kept in a chain.
pub const MAX_CHAIN: usize = 16;
/// Key derivation rounds accepted (higher is refused as hostile).
const MAX_ITERATIONS: i32 = 10_000_000;

const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_ENCRYPTED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.6");
const OID_PBES2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.5.13");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_EC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const OID_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const OID_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const OID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_LOCAL_KEY_ID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.21");
const OID_CN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");

/// The one message for every way a certificate file can fail to open: no hint whether
/// the password or the file was wrong, and nothing from either.
pub const BAD_FILE: &str = "incorrect password or unreadable certificate";

fn bad() -> PdfError {
    PdfError::Certificate(BAD_FILE.into())
}

/// Facts about a certificate shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    /// Common name, or the whole subject when it has none.
    pub name: String,
    pub subject: String,
    pub issuer: String,
    /// Serial number in hex.
    pub serial: String,
    /// SHA-256 of the DER certificate, hex with colons.
    pub fingerprint: String,
    pub not_before: SystemTime,
    pub not_after: SystemTime,
    /// Issuer and subject are the same: the signer vouches for themselves.
    pub self_signed: bool,
}

impl CertInfo {
    pub fn of(cert: &Certificate) -> Self {
        let tbs = cert.tbs_certificate();
        let der = cert.to_der().unwrap_or_default();
        let digest = sha2::Sha256::digest(&der);
        Self {
            name: common_name(tbs.subject()).unwrap_or_else(|| tbs.subject().to_string()),
            subject: tbs.subject().to_string(),
            issuer: tbs.issuer().to_string(),
            serial: hex(tbs.serial_number().as_bytes(), ""),
            fingerprint: hex(&digest, ":"),
            not_before: tbs.validity().not_before.to_system_time(),
            not_after: tbs.validity().not_after.to_system_time(),
            self_signed: tbs.subject() == tbs.issuer(),
        }
    }

    /// The validity period, human readable (`2019-08-29 08:27 UTC to 2029-08-29 08:27 UTC`).
    pub fn validity_text(&self) -> String {
        format!(
            "{} to {}",
            crate::sign::human_date(self.not_before),
            crate::sign::human_date(self.not_after)
        )
    }
}

pub(crate) fn common_name(name: &x509_cert::name::Name) -> Option<String> {
    for atv in name.iter() {
        if atv.oid == OID_CN {
            if let Ok(s) = atv.value.decode_as::<der::asn1::Utf8StringRef<'_>>() {
                return Some(s.as_str().to_string());
            }
            if let Ok(s) = atv.value.decode_as::<der::asn1::PrintableStringRef<'_>>() {
                return Some(s.as_str().to_string());
            }
        }
    }
    None
}

pub(crate) fn hex(bytes: &[u8], sep: &str) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(sep)
}

/// A signing key.
pub(crate) enum SignKey {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

/// A loaded certificate with its key, for one signing operation. Dropping it clears the
/// key.
pub struct Signer {
    pub(crate) key: SignKey,
    pub(crate) cert: Certificate,
    pub(crate) chain: Vec<Certificate>,
    pub info: CertInfo,
    /// "RSA 2048", "ECDSA P-256".
    pub key_label: String,
    /// Things worth telling the user that do not block signing.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("name", &self.info.name)
            .field("key", &self.key_label)
            .finish_non_exhaustive()
    }
}

/// Open a PKCS#12 file and check its certificate can sign at `now`.
pub fn load_pkcs12(data: &[u8], password: &Secret, now: SystemTime) -> Result<Signer, PdfError> {
    if data.len() > MAX_P12_BYTES {
        return Err(PdfError::Certificate(format!(
            "the certificate file is larger than {} MiB",
            MAX_P12_BYTES >> 20
        )));
    }
    let (key_der, certs) = guard(|| parse(data, password))?;
    let (key, key_label, spki) = guard(|| decode_key(&key_der))?;
    drop(key_der);
    // The signer certificate is the one carrying the key's public half.
    let leaf_ix = certs
        .iter()
        .position(|c| {
            c.tbs_certificate()
                .subject_public_key_info()
                .to_der()
                .is_ok_and(|d| d == spki)
        })
        .ok_or_else(|| {
            PdfError::Certificate(
                "the certificate file has no certificate for its private key".into(),
            )
        })?;
    let mut certs = certs;
    let cert = certs.remove(leaf_ix);
    let chain = order_chain(&cert, certs);
    let info = CertInfo::of(&cert);
    if now < info.not_before {
        return Err(PdfError::Certificate(
            "the certificate is not valid yet".into(),
        ));
    }
    if now > info.not_after {
        return Err(PdfError::Certificate("the certificate has expired".into()));
    }
    let mut warnings = Vec::new();
    match key_usage(&cert) {
        Some(ku) if !ku.digital_signature() && !ku.non_repudiation() => {
            return Err(PdfError::Certificate(
                "this certificate may not be used for signing (key usage)".into(),
            ));
        }
        Some(_) => {}
        None => warnings.push("The certificate does not state what it may be used for.".into()),
    }
    Ok(Signer {
        key,
        cert,
        chain,
        info,
        key_label,
        warnings,
    })
}

fn key_usage(cert: &Certificate) -> Option<KeyUsage> {
    let exts = cert.tbs_certificate().extensions()?;
    let ext = exts
        .iter()
        .find(|e| e.extn_id == ObjectIdentifier::new_unwrap("2.5.29.15"))?;
    KeyUsage::from_der(ext.extn_value.as_bytes()).ok()
}

/// Issuer order from the leaf, capped, without cycles.
fn order_chain(leaf: &Certificate, mut rest: Vec<Certificate>) -> Vec<Certificate> {
    let mut out: Vec<Certificate> = Vec::new();
    let mut issuer = leaf.tbs_certificate().issuer().clone();
    while out.len() < MAX_CHAIN {
        let Some(ix) = rest
            .iter()
            .position(|c| *c.tbs_certificate().subject() == issuer)
        else {
            break;
        };
        let c = rest.remove(ix);
        let self_signed = c.tbs_certificate().subject() == c.tbs_certificate().issuer();
        issuer = c.tbs_certificate().issuer().clone();
        out.push(c);
        if self_signed {
            break;
        }
    }
    // Unrelated extra certificates are kept too (some files carry cross-certificates).
    for c in rest {
        if out.len() >= MAX_CHAIN {
            break;
        }
        out.push(c);
    }
    out
}

/// The PKCS#8 private key (DER, zeroized) and every certificate in the file.
/// A private key bag: local key id and the decrypted PKCS#8 DER.
type KeyBag = (Option<Vec<u8>>, Zeroizing<Vec<u8>>);

fn parse(
    data: &[u8],
    password: &Secret,
) -> Result<(Zeroizing<Vec<u8>>, Vec<Certificate>), PdfError> {
    let pfx = pkcs12::pfx::Pfx::from_der(data).map_err(|_| bad())?;
    if pfx.auth_safe.content_type != OID_DATA {
        return Err(PdfError::Certificate(
            "public-key protected certificate files are not supported".into(),
        ));
    }
    let auth_bytes = pfx
        .auth_safe
        .content
        .decode_as::<OctetString>()
        .map_err(|_| bad())?;
    if let Some(mac) = &pfx.mac_data {
        verify_mac(mac, auth_bytes.as_bytes(), password)?;
    }
    let safes = Vec::<cms::content_info::ContentInfo>::from_der(auth_bytes.as_bytes())
        .map_err(|_| bad())?;
    let mut keys: Vec<KeyBag> = Vec::new();
    let mut certs: Vec<(Option<Vec<u8>>, Certificate)> = Vec::new();
    for ci in safes.iter().take(64) {
        let contents: Zeroizing<Vec<u8>> = if ci.content_type == OID_DATA {
            Zeroizing::new(
                ci.content
                    .decode_as::<OctetString>()
                    .map_err(|_| bad())?
                    .as_bytes()
                    .to_vec(),
            )
        } else if ci.content_type == OID_ENCRYPTED_DATA {
            let ed = ci
                .content
                .decode_as::<cms::encrypted_data::EncryptedData>()
                .map_err(|_| bad())?;
            let ct = ed
                .enc_content_info
                .encrypted_content
                .as_ref()
                .ok_or_else(bad)?;
            decrypt(
                &ed.enc_content_info.content_enc_alg,
                ct.as_bytes(),
                password,
            )?
        } else {
            continue;
        };
        let bags = Vec::<pkcs12::safe_bag::SafeBag>::from_der(&contents).map_err(|_| bad())?;
        for bag in bags.iter().take(256) {
            let local_id = bag.bag_attributes.as_ref().and_then(|attrs| {
                attrs
                    .iter()
                    .find(|a| a.oid == OID_LOCAL_KEY_ID)
                    .and_then(|a| a.values.iter().next())
                    .and_then(|v| v.decode_as::<OctetString>().ok())
                    .map(|o| o.as_bytes().to_vec())
            });
            // Decoded, `bag_value` is the whole explicitly tagged [0]; its content is the bag.
            let inner = der::asn1::AnyRef::from_der(&bag.bag_value)
                .map_err(|_| bad())?
                .value()
                .to_vec();
            if bag.bag_id == pkcs12::PKCS_12_PKCS8_KEY_BAG_OID {
                let epki = pkcs12::pbe_params::EncryptedPrivateKeyInfo::from_der(&inner)
                    .map_err(|_| bad())?;
                let key = decrypt(
                    &epki.encryption_algorithm,
                    epki.encrypted_data.as_bytes(),
                    password,
                )?;
                keys.push((local_id, key));
            } else if bag.bag_id == pkcs12::PKCS_12_KEY_BAG_OID {
                keys.push((local_id, Zeroizing::new(inner)));
            } else if bag.bag_id == pkcs12::PKCS_12_CERT_BAG_OID {
                let cb = pkcs12::CertBag::from_der(&inner).map_err(|_| bad())?;
                if cb.cert_id == pkcs12::PKCS_12_X509_CERT_OID
                    && certs.len() < MAX_CHAIN + 1
                    && let Ok(c) = Certificate::from_der(cb.cert_value.as_bytes())
                {
                    certs.push((local_id, c));
                }
            }
        }
    }
    let mut keys = keys.into_iter();
    let (_, key) = keys
        .next()
        .ok_or_else(|| PdfError::Certificate("the certificate file has no private key".into()))?;
    if keys.next().is_some() {
        return Err(PdfError::Certificate(
            "the certificate file holds more than one key; export a single certificate".into(),
        ));
    }
    Ok((key, certs.into_iter().map(|(_, c)| c).collect()))
}

fn verify_mac(mac: &pkcs12::MacData, data: &[u8], pw: &Secret) -> Result<(), PdfError> {
    use hmac::{KeyInit, Mac};
    if !(1..=MAX_ITERATIONS).contains(&mac.iterations) {
        return Err(bad());
    }
    let salt = mac.mac_salt.as_bytes();
    let expected = mac.mac.digest.as_bytes();
    let ok = if mac.mac.algorithm.oid == OID_SHA1 {
        let key = Zeroizing::new(
            derive_key_utf8::<sha1::Sha1>(
                pw.expose(),
                salt,
                Pkcs12KeyType::Mac,
                mac.iterations,
                20,
            )
            .map_err(|_| bad())?,
        );
        let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(&key).map_err(|_| bad())?;
        m.update(data);
        m.verify_slice(expected).is_ok()
    } else if mac.mac.algorithm.oid == OID_SHA256 {
        let key = Zeroizing::new(
            derive_key_utf8::<sha2::Sha256>(
                pw.expose(),
                salt,
                Pkcs12KeyType::Mac,
                mac.iterations,
                32,
            )
            .map_err(|_| bad())?,
        );
        let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(&key).map_err(|_| bad())?;
        m.update(data);
        m.verify_slice(expected).is_ok()
    } else {
        // An unknown MAC: decryption padding still catches a wrong password.
        true
    };
    if ok { Ok(()) } else { Err(bad()) }
}

/// Decrypt a bag with PBES2 or a legacy PKCS#12 PBE scheme.
fn decrypt(
    alg: &AlgorithmIdentifierOwned,
    data: &[u8],
    pw: &Secret,
) -> Result<Zeroizing<Vec<u8>>, PdfError> {
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockModeDecrypt, InnerIvInit, KeyIvInit};
    if data.len() > MAX_P12_BYTES {
        return Err(bad());
    }
    if alg.oid == OID_PBES2 {
        let der = alg.to_der().map_err(|_| bad())?;
        let scheme = pkcs5::EncryptionScheme::from_der(&der).map_err(|_| bad())?;
        if let pkcs5::EncryptionScheme::Pbes2(p) = &scheme
            && let pkcs5::pbes2::Kdf::Pbkdf2(k) = &p.kdf
            && k.iteration_count > MAX_ITERATIONS as u32
        {
            return Err(bad());
        }
        return scheme
            .decrypt(pw.expose().as_bytes(), data)
            .map(Zeroizing::new)
            .map_err(|_| bad());
    }
    let params = alg
        .parameters
        .as_ref()
        .ok_or_else(bad)?
        .decode_as::<pkcs12::pbe_params::Pkcs12PbeParams>()
        .map_err(|_| bad())?;
    if !(1..=MAX_ITERATIONS).contains(&params.iterations) {
        return Err(bad());
    }
    let salt = params.salt.as_bytes();
    let derive = |kind, len| -> Result<Zeroizing<Vec<u8>>, PdfError> {
        derive_key_utf8::<sha1::Sha1>(pw.expose(), salt, kind, params.iterations, len)
            .map(Zeroizing::new)
            .map_err(|_| bad())
    };
    let iv = derive(Pkcs12KeyType::Iv, 8)?;
    let out = if alg.oid == pkcs12::PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC {
        let key = derive(Pkcs12KeyType::EncryptionKey, 24)?;
        cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
            .map_err(|_| bad())?
            .decrypt_padded_vec::<Pkcs7>(data)
    } else if alg.oid == pkcs12::PKCS_12_PBE_WITH_SHAAND2_KEY_TRIPLE_DES_CBC {
        let key = derive(Pkcs12KeyType::EncryptionKey, 16)?;
        cbc::Decryptor::<des::TdesEde2>::new_from_slices(&key, &iv)
            .map_err(|_| bad())?
            .decrypt_padded_vec::<Pkcs7>(data)
    } else if alg.oid == pkcs12::PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC
        || alg.oid == pkcs12::PKCS_12_PBE_WITH_SHAAND128_BIT_RC2_CBC
    {
        let bits = if alg.oid == pkcs12::PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC {
            40
        } else {
            128
        };
        let key = derive(Pkcs12KeyType::EncryptionKey, bits / 8)?;
        let cipher = rc2::Rc2::new_with_eff_key_len(&key, bits);
        cbc::Decryptor::<rc2::Rc2>::inner_iv_slice_init(cipher, &iv)
            .map_err(|_| bad())?
            .decrypt_padded_vec::<Pkcs7>(data)
    } else {
        return Err(PdfError::Certificate(
            "the certificate file uses an unsupported encryption".into(),
        ));
    };
    out.map(Zeroizing::new).map_err(|_| bad())
}

/// The key, its label and its public key as SubjectPublicKeyInfo DER.
fn decode_key(der: &[u8]) -> Result<(SignKey, String, Vec<u8>), PdfError> {
    use pkcs8::{DecodePrivateKey, EncodePublicKey};
    let info = pkcs8::PrivateKeyInfoRef::from_der(der).map_err(|_| bad())?;
    let oid = info.algorithm.oid;
    if oid == OID_RSA {
        let key = rsa::RsaPrivateKey::from_pkcs8_der(der).map_err(|_| bad())?;
        let bits = rsa::traits::PublicKeyParts::n(&key).bit_width();
        if !(2048..=8192).contains(&bits) {
            return Err(PdfError::Certificate(format!(
                "RSA keys of {bits} bits are not supported (2048–8192 are)"
            )));
        }
        let spki = key
            .to_public_key()
            .to_public_key_der()
            .map_err(|_| bad())?
            .into_vec();
        return Ok((SignKey::Rsa(Box::new(key)), format!("RSA {bits}"), spki));
    }
    if oid == OID_EC {
        let curve = info
            .algorithm
            .parameters
            .and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        if curve == Some(OID_P256) {
            let key = p256::SecretKey::from_pkcs8_der(der).map_err(|_| bad())?;
            let spki = key
                .public_key()
                .to_public_key_der()
                .map_err(|_| bad())?
                .into_vec();
            return Ok((
                SignKey::P256(p256::ecdsa::SigningKey::from(key)),
                "ECDSA P-256".into(),
                spki,
            ));
        }
        if curve == Some(OID_P384) {
            let key = p384::SecretKey::from_pkcs8_der(der).map_err(|_| bad())?;
            let spki = key
                .public_key()
                .to_public_key_der()
                .map_err(|_| bad())?
                .into_vec();
            return Ok((
                SignKey::P384(p384::ecdsa::SigningKey::from(key)),
                "ECDSA P-384".into(),
                spki,
            ));
        }
        return Err(PdfError::Certificate(
            "only P-256 and P-384 elliptic-curve keys are supported".into(),
        ));
    }
    Err(PdfError::Certificate(
        "only RSA and ECDSA keys are supported".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::certs::{self, KeySpec, P12Spec};
    use std::time::Duration;

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn self_signed_is_recognized() {
        let (_, signer, ca) = certs::material(&P12Spec::default());
        assert!(!CertInfo::of(&signer).self_signed);
        let ca = CertInfo::of(&ca);
        assert!(ca.self_signed);
        assert!(
            ca.validity_text().contains(" UTC to "),
            "{}",
            ca.validity_text()
        );
    }

    #[test]
    fn loads_modern_and_legacy_files() {
        for (key, legacy) in [
            (KeySpec::P256, false),
            (KeySpec::P384, false),
            (KeySpec::Rsa2048, false),
            (KeySpec::P256, true),
        ] {
            let p12 = certs::p12(&P12Spec {
                key,
                legacy,
                ..P12Spec::default()
            });
            let s = load_pkcs12(&p12, &Secret::new("test-pass"), now())
                .unwrap_or_else(|e| panic!("{key:?} legacy={legacy}: {e}"));
            assert_eq!(s.info.name, "Test Signer");
            assert!(s.info.issuer.contains("Test CA"), "{}", s.info.issuer);
            assert_eq!(s.chain.len(), 1, "the CA comes along");
            assert_eq!(s.info.fingerprint.len(), 32 * 3 - 1);
            assert!(!format!("{s:?}").contains("test-pass"));
        }
    }

    #[test]
    fn wrong_password_and_garbage_say_the_same() {
        let p12 = certs::p12(&P12Spec::default());
        let err = load_pkcs12(&p12, &Secret::new("wrong-one"), now()).expect_err("wrong");
        assert_eq!(err.to_string(), BAD_FILE);
        assert!(!err.to_string().contains("wrong-one"));
        let err = load_pkcs12(b"garbage", &Secret::new("x"), now()).expect_err("garbage");
        assert_eq!(err.to_string(), BAD_FILE);
        assert!(load_pkcs12(&vec![0u8; MAX_P12_BYTES + 1], &Secret::new("x"), now()).is_err());
    }

    #[test]
    fn expired_and_wrong_usage_are_refused() {
        let p12 = certs::p12(&P12Spec::default());
        let later = now() + Duration::from_secs(2 * 365 * 24 * 3600);
        let err = load_pkcs12(&p12, &Secret::new("test-pass"), later).expect_err("expired");
        assert!(err.to_string().contains("expired"));
        let earlier = now() - Duration::from_secs(2 * 24 * 3600);
        let err = load_pkcs12(&p12, &Secret::new("test-pass"), earlier).expect_err("not yet");
        assert!(err.to_string().contains("not valid yet"));
        let p12 = certs::p12(&P12Spec {
            encipher_only: true,
            ..P12Spec::default()
        });
        let err = load_pkcs12(&p12, &Secret::new("test-pass"), now()).expect_err("usage");
        assert!(err.to_string().contains("key usage"));
    }
}
