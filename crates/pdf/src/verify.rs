//! Verifying existing signatures (design D13), offline: ByteRange checks, the CMS digest
//! and signature, the certificate's validity at signing time, weak algorithms, and what
//! changed after signing. Identity is never checked against a trust list.

use std::time::SystemTime;

use der::asn1::{ObjectIdentifier, OctetString};
use der::{Decode, Encode};
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier as ObjId};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use crate::cert::CertInfo;
use crate::doc::{Document, MAX_FIELDS, MAX_TREE_DEPTH};
use crate::sign::{OID_MESSAGE_DIGEST, OID_SIGNING_TIME, parse_signed_data};
use crate::strings::decode_text;

/// CMS data read per signature.
pub const MAX_CMS_BYTES: usize = 64 * 1024;
/// Signatures verified per document.
pub const MAX_SIGNATURES: usize = 64;

const OID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const OID_SHA224: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.4");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const OID_RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const OID_RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const OID_RSA_SHA224: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.14");
const OID_RSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
const OID_ECDSA_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.1");
const OID_ECDSA_SHA224: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.1");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const OID_ECDSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");
const OID_EC_PUBLIC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const OID_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const OID_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const OID_P521: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.35");

/// The outcome for one signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigStatus {
    /// Intact, and it covers the whole file.
    Valid,
    /// The signed revision is intact; later changes exist (summarized).
    ChangedAfter(Vec<String>),
    /// Digest or signature mismatch, malformed, or unsupported (why).
    Broken(String),
}

impl SigStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Valid => "Valid",
            Self::ChangedAfter(_) => "Valid, changed after signing",
            Self::Broken(_) => "Broken",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureReport {
    /// Full field name.
    pub field: String,
    pub signer: Option<CertInfo>,
    /// Signing time as claimed (signed attribute, else `/M`), human readable.
    pub time: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
    /// `/SubFilter` (e.g. `ETSI.CAdES.detached`).
    pub kind: String,
    pub status: SigStatus,
    /// Weak algorithms used (SHA-1, RSA under 2048 bits).
    pub weak: Vec<String>,
    /// The certificate was outside its validity period at the claimed signing time.
    pub cert_time_problem: Option<String>,
}

/// One signature value as found in the document.
struct Found {
    field: String,
    byte_range: Option<Vec<i64>>,
    contents: Vec<u8>,
    sub_filter: String,
    m: Option<String>,
    reason: Option<String>,
    location: Option<String>,
    contact: Option<String>,
}

/// Verify every signed signature field of `doc`.
pub fn verify_all(doc: &Document) -> Vec<SignatureReport> {
    let found = match crate::guard(|| Ok(find_signatures(doc))) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    found
        .into_iter()
        .map(|f| {
            crate::guard(|| Ok(verify_one(doc.bytes(), &f))).unwrap_or_else(|_| SignatureReport {
                field: f.field.clone(),
                signer: None,
                time: f.m.clone(),
                reason: f.reason.clone(),
                location: f.location.clone(),
                contact: f.contact.clone(),
                kind: f.sub_filter.clone(),
                status: SigStatus::Broken("the signature could not be checked".into()),
                weak: Vec::new(),
                cert_time_problem: None,
            })
        })
        .collect()
}

fn find_signatures(doc: &Document) -> Vec<Found> {
    let mut out = Vec::new();
    let xref = doc.pdf().xref();
    let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return out;
    };
    let Some(acro) = catalog.get::<Dict<'_>>(b"AcroForm") else {
        return out;
    };
    let Some(fields) = acro.get::<Array<'_>>(b"Fields") else {
        return out;
    };
    let mut seen = std::collections::HashSet::<ObjId>::new();
    let mut visited = 0;
    for f in fields.iter::<Dict<'_>>() {
        walk(
            &f,
            String::new(),
            None,
            0,
            &mut out,
            &mut seen,
            &mut visited,
        );
    }
    out
}

fn text_of(d: &Dict<'_>, key: &[u8]) -> Option<String> {
    d.get::<hayro::hayro_syntax::object::String<'_>>(key)
        .map(|s| decode_text(s.as_bytes()))
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.chars().take(512).collect())
}

fn walk(
    field: &Dict<'_>,
    prefix: String,
    ft: Option<String>,
    depth: usize,
    out: &mut Vec<Found>,
    seen: &mut std::collections::HashSet<ObjId>,
    visited: &mut usize,
) {
    *visited += 1;
    if depth > MAX_TREE_DEPTH || *visited > MAX_FIELDS || out.len() >= MAX_SIGNATURES {
        return;
    }
    if let Some(id) = field.obj_id()
        && !seen.insert(id)
    {
        return;
    }
    let name = match text_of(field, b"T") {
        Some(t) if prefix.is_empty() => t,
        Some(t) => format!("{prefix}.{t}"),
        None => prefix,
    };
    let ft = field
        .get::<Name<'_>>(b"FT")
        .map(|n| n.as_str().to_string())
        .or(ft);
    if let Some(kids) = field.get::<Array<'_>>(b"Kids") {
        let kids: Vec<Dict<'_>> = kids.iter::<Dict<'_>>().take(MAX_FIELDS).collect();
        if kids.iter().any(|k| k.contains_key(b"T")) {
            for k in kids.iter().filter(|k| k.contains_key(b"T")) {
                walk(k, name.clone(), ft.clone(), depth + 1, out, seen, visited);
            }
            return;
        }
    }
    if ft.as_deref() != Some("Sig") {
        return;
    }
    let Some(v) = field.get::<Dict<'_>>(b"V") else {
        return;
    };
    let byte_range = v.get::<Array<'_>>(b"ByteRange").map(|a| {
        a.iter::<Object<'_>>()
            .take(16)
            .map(|o| match o {
                Object::Number(n) => n.as_i64(),
                _ => -1,
            })
            .collect()
    });
    let contents = v
        .get::<hayro::hayro_syntax::object::String<'_>>(b"Contents")
        .map(|s| s.as_bytes().to_vec())
        .unwrap_or_default();
    out.push(Found {
        field: name,
        byte_range,
        contents,
        sub_filter: v
            .get::<Name<'_>>(b"SubFilter")
            .map(|n| n.as_str().to_string())
            .unwrap_or_default(),
        m: text_of(&v, b"M").map(|m| pdf_date_text(&m)),
        reason: text_of(&v, b"Reason"),
        location: text_of(&v, b"Location"),
        contact: text_of(&v, b"ContactInfo"),
    });
}

/// `D:20261119120000Z` → `2026-11-19 12:00:00 UTC` (as written; unparseable stays raw).
fn pdf_date_text(m: &str) -> String {
    let d = m.trim_start_matches("D:");
    let digits: String = d.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return m.to_string();
    }
    let part = |a: usize, b: usize| digits.get(a..b).unwrap_or("00").to_string();
    let rest = &d[digits.len().min(d.len())..];
    let zone = match rest.chars().next() {
        Some('Z') | None => "UTC".to_string(),
        Some(_) => rest.replace('\'', "").to_string(),
    };
    format!(
        "{}-{}-{} {}:{}:{} {}",
        part(0, 4),
        part(4, 6),
        part(6, 8),
        part(8, 10),
        part(10, 12),
        part(12, 14),
        zone
    )
}

fn verify_one(bytes: &[u8], f: &Found) -> SignatureReport {
    let mut r = SignatureReport {
        field: f.field.clone(),
        signer: None,
        time: f.m.clone(),
        reason: f.reason.clone(),
        location: f.location.clone(),
        contact: f.contact.clone(),
        kind: f.sub_filter.clone(),
        status: SigStatus::Valid,
        weak: Vec::new(),
        cert_time_problem: None,
    };
    match check(bytes, f, &mut r) {
        Ok(end) => {
            if end < bytes.len() {
                // Trailing whitespace after %%EOF is not a change.
                let tail = bytes.get(end..).unwrap_or(&[]);
                if tail.iter().all(|b| b.is_ascii_whitespace() || *b == 0) {
                    r.status = SigStatus::Valid;
                } else {
                    r.status = SigStatus::ChangedAfter(describe_changes(bytes, end));
                }
            }
        }
        Err(why) => r.status = SigStatus::Broken(why),
    }
    r
}

/// Check one signature; on success, return where the signed range ends.
fn check(bytes: &[u8], f: &Found, r: &mut SignatureReport) -> Result<usize, String> {
    let malformed = |what: &str| format!("the signature is malformed ({what})");
    let range = f
        .byte_range
        .as_ref()
        .ok_or_else(|| malformed("no byte range"))?;
    let [a, b, c, d] = range.as_slice() else {
        return Err(malformed("the byte range does not have four numbers"));
    };
    let (a, b, c, d) = (*a, *b, *c, *d);
    if a != 0 || b <= 0 || c <= b || d < 0 {
        return Err(malformed("invalid byte range"));
    }
    let (b, c, d) = (b as usize, c as usize, d as usize);
    let end = c
        .checked_add(d)
        .ok_or_else(|| malformed("byte range overflow"))?;
    if end > bytes.len() {
        return Err(malformed("the byte range points past the end of the file"));
    }
    if f.contents.len() > MAX_CMS_BYTES || c - b > MAX_CMS_BYTES * 2 + 2 {
        return Err(malformed("the signature data is too large"));
    }
    let cms = trim_der(&f.contents);
    let sd = parse_signed_data(cms).ok_or_else(|| malformed("unreadable signature data"))?;
    let si = sd
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or_else(|| malformed("no signer"))?;
    let certs: Vec<Certificate> = sd
        .certificates
        .as_ref()
        .map(|cs| {
            cs.0.iter()
                .take(crate::cert::MAX_CHAIN + 4)
                .filter_map(|c| match c {
                    cms::cert::CertificateChoices::Certificate(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let cert = find_signer(&certs, &si.sid).ok_or_else(|| malformed("no signer certificate"))?;
    let info = CertInfo::of(cert);
    r.signer = Some(info.clone());
    let digest_oid = si.digest_alg.oid;
    if digest_oid == OID_SHA1 {
        r.weak.push("SHA-1 digest".into());
    }
    // Who signed and when is read before the byte checks, so a broken signature still
    // shows its signer.
    if let Some(t) = si.signed_attrs.as_ref().and_then(|attrs| {
        attrs
            .iter()
            .find(|a| a.oid == OID_SIGNING_TIME)
            .and_then(|a| a.values.iter().next())
            .and_then(|v| v.decode_as::<x509_cert::time::Time>().ok())
    }) {
        let when = t.to_system_time();
        r.time = Some(crate::sign::human_date(when));
        check_cert_time(&info, when, r);
    }
    // The gap must be exactly the hex string of /Contents.
    let gap = bytes.get(b..c).ok_or_else(|| malformed("byte range"))?;
    if gap.first() != Some(&b'<') || gap.last() != Some(&b'>') {
        if contents_elsewhere(bytes, cms) {
            return Err(rebuilt(end, bytes.len()));
        }
        return Err(malformed(
            "the byte range does not exclude exactly the signature",
        ));
    }
    let data = [
        bytes.get(..b).unwrap_or(&[]),
        bytes.get(c..end).unwrap_or(&[]),
    ];
    let digest = hash(digest_oid, &data)
        .ok_or_else(|| format!("unsupported digest algorithm {digest_oid}"))?;
    // adbe.pkcs7.sha1 carries the SHA-1 of the data as its content.
    let content_digest = match &sd.encap_content_info.econtent {
        Some(any) => {
            let inner = any
                .decode_as::<OctetString>()
                .map_err(|_| malformed("embedded content"))?;
            let d1 = hash(OID_SHA1, &data).unwrap_or_default();
            if inner.as_bytes() != d1.as_slice() {
                return Err("document content does not match the signature".into());
            }
            if !r.weak.iter().any(|w| w.contains("SHA-1")) {
                r.weak.push("SHA-1 digest".into());
            }
            Some(inner.as_bytes().to_vec())
        }
        None => None,
    };
    let (message, prehash) = match &si.signed_attrs {
        Some(attrs) => {
            let md = attrs
                .iter()
                .find(|a| a.oid == OID_MESSAGE_DIGEST)
                .and_then(|a| a.values.iter().next())
                .and_then(|v| v.decode_as::<OctetString>().ok())
                .ok_or_else(|| malformed("no message digest"))?;
            let expected = content_digest.as_deref().map_or(digest.as_slice(), |c| c);
            let expected = if content_digest.is_some() {
                // The digest is over the embedded content, not the file.
                hash(digest_oid, &[expected]).unwrap_or_default()
            } else {
                expected.to_vec()
            };
            if md.as_bytes() != expected.as_slice() {
                return Err("document content does not match the signature".into());
            }
            let der = attrs.to_der().map_err(|_| malformed("signed attributes"))?;
            let pre = hash(digest_oid, &[&der]).ok_or_else(|| malformed("digest"))?;
            (der, pre)
        }
        None => (digest.clone(), digest.clone()),
    };
    if r.time.is_none() {
        r.time = f.m.clone();
    }
    verify_signature(
        cert,
        &si.signature_algorithm,
        digest_oid,
        &message,
        &prehash,
        si.signature.as_bytes(),
        r,
    )?;
    Ok(end)
}

/// The signature value is in the file, just not where its byte range says: the file was
/// written out again (merged, optimized, re-saved) and the signed bytes are gone.
fn contents_elsewhere(bytes: &[u8], cms: &[u8]) -> bool {
    let probe = cms.get(..cms.len().min(32)).unwrap_or(&[]);
    if probe.len() < 16 {
        return false;
    }
    let lower: String = probe.iter().map(|b| format!("{b:02x}")).collect();
    [lower.clone(), lower.to_ascii_uppercase()]
        .iter()
        .any(|h| crate::doc::find(bytes, h.as_bytes()).is_some())
}

fn rebuilt(signed: usize, total: usize) -> String {
    format!(
        "the file was rebuilt after signing (for example merged with other PDFs or re-saved \
         by another program), so the signed content is no longer present; the signature \
         covers {} of the current {}",
        size_text(signed),
        size_text(total)
    )
}

/// Decimal sizes: `167 KB`, `15.2 MB`.
fn size_text(n: usize) -> String {
    let n = n as f64;
    if n < 1_000.0 {
        format!("{n} bytes")
    } else if n < 1_000_000.0 {
        format!("{:.0} KB", n / 1_000.0)
    } else {
        format!("{:.1} MB", n / 1_000_000.0)
    }
}

fn check_cert_time(info: &CertInfo, when: SystemTime, r: &mut SignatureReport) {
    if when < info.not_before {
        r.cert_time_problem = Some("The certificate was not valid yet at the signing time.".into());
    } else if when > info.not_after {
        r.cert_time_problem = Some("The certificate had expired at the signing time.".into());
    }
}

/// `/Contents` is zero-padded after the DER; cut it to the DER length.
fn trim_der(bytes: &[u8]) -> &[u8] {
    let len = der::SliceReader::new(bytes)
        .ok()
        .and_then(|mut r| {
            let h = der::Header::decode(&mut r).ok()?;
            let header_len = u32::from(h.encoded_len().ok()?) as usize;
            Some(header_len + u32::from(h.length()) as usize)
        })
        .unwrap_or(bytes.len());
    bytes.get(..len).unwrap_or(bytes)
}

fn find_signer<'a>(
    certs: &'a [Certificate],
    sid: &cms::signed_data::SignerIdentifier,
) -> Option<&'a Certificate> {
    match sid {
        cms::signed_data::SignerIdentifier::IssuerAndSerialNumber(ias) => certs.iter().find(|c| {
            c.tbs_certificate().issuer() == &ias.issuer
                && c.tbs_certificate().serial_number() == &ias.serial_number
        }),
        cms::signed_data::SignerIdentifier::SubjectKeyIdentifier(ski) => certs.iter().find(|c| {
            c.tbs_certificate()
                .extensions()
                .into_iter()
                .flatten()
                .any(|e| {
                    e.extn_id == ObjectIdentifier::new_unwrap("2.5.29.14")
                        && OctetString::from_der(e.extn_value.as_bytes())
                            .is_ok_and(|k| k.as_bytes() == ski.0.as_bytes())
                })
        }),
    }
}

fn hash(oid: ObjectIdentifier, parts: &[&[u8]]) -> Option<Vec<u8>> {
    fn run<D: sha2::Digest>(parts: &[&[u8]]) -> Vec<u8> {
        let mut h = D::new();
        for p in parts {
            h.update(p);
        }
        h.finalize().to_vec()
    }
    Some(match oid {
        o if o == OID_SHA1 => run::<sha1::Sha1>(parts),
        o if o == OID_SHA224 => run::<sha2::Sha224>(parts),
        o if o == OID_SHA256 => run::<sha2::Sha256>(parts),
        o if o == OID_SHA384 => run::<sha2::Sha384>(parts),
        o if o == OID_SHA512 => run::<sha2::Sha512>(parts),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_signature(
    cert: &Certificate,
    alg: &AlgorithmIdentifierOwned,
    digest_oid: ObjectIdentifier,
    message: &[u8],
    prehash: &[u8],
    sig: &[u8],
    r: &mut SignatureReport,
) -> Result<(), String> {
    use signature::hazmat::PrehashVerifier;
    let spki = cert
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|_| "the signer's public key is unreadable".to_string())?;
    let mismatch = || "the signature does not match (signature value is wrong)".to_string();
    let oid = alg.oid;
    let is_rsa = [
        OID_RSA,
        OID_RSA_SHA1,
        OID_RSA_SHA224,
        OID_RSA_SHA256,
        OID_RSA_SHA384,
        OID_RSA_SHA512,
    ]
    .contains(&oid);
    if is_rsa || oid == OID_RSA_PSS {
        use rsa::pkcs8::DecodePublicKey;
        let pk = rsa::RsaPublicKey::from_public_key_der(&spki)
            .map_err(|_| "the signer's RSA key is unreadable".to_string())?;
        let bits = rsa::traits::PublicKeyParts::n(&pk).bits();
        if bits < 2048 {
            r.weak.push(format!("RSA key of {bits} bits"));
        }
        if oid == OID_RSA_SHA1 && !r.weak.iter().any(|w| w.contains("SHA-1")) {
            r.weak.push("SHA-1 signature".into());
        }
        let s = rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| mismatch())?;
        let ok = if oid == OID_RSA_PSS {
            let pss = rsa::pss::Signature::try_from(sig).map_err(|_| mismatch())?;
            let pss_digest = pss_hash(alg).unwrap_or(OID_SHA1);
            if pss_digest == OID_SHA1 && !r.weak.iter().any(|w| w.contains("SHA-1")) {
                r.weak.push("SHA-1 signature".into());
            }
            let pre = hash(pss_digest, &[message]).ok_or_else(mismatch)?;
            match pss_digest {
                o if o == OID_SHA256 => rsa::pss::VerifyingKey::<sha2::Sha256>::new(pk)
                    .verify_prehash(&pre, &pss)
                    .is_ok(),
                o if o == OID_SHA384 => rsa::pss::VerifyingKey::<sha2::Sha384>::new(pk)
                    .verify_prehash(&pre, &pss)
                    .is_ok(),
                o if o == OID_SHA512 => rsa::pss::VerifyingKey::<sha2::Sha512>::new(pk)
                    .verify_prehash(&pre, &pss)
                    .is_ok(),
                _ => rsa::pss::VerifyingKey::<sha1::Sha1>::new(pk)
                    .verify_prehash(&pre, &pss)
                    .is_ok(),
            }
        } else {
            let d = match oid {
                o if o == OID_RSA_SHA1 => OID_SHA1,
                o if o == OID_RSA_SHA224 => OID_SHA224,
                o if o == OID_RSA_SHA256 => OID_SHA256,
                o if o == OID_RSA_SHA384 => OID_SHA384,
                o if o == OID_RSA_SHA512 => OID_SHA512,
                _ => digest_oid,
            };
            let pre = if d == digest_oid {
                prehash.to_vec()
            } else {
                hash(d, &[message]).ok_or_else(mismatch)?
            };
            match d {
                o if o == OID_SHA1 => rsa::pkcs1v15::VerifyingKey::<sha1::Sha1>::new(pk)
                    .verify_prehash(&pre, &s)
                    .is_ok(),
                o if o == OID_SHA224 => rsa::pkcs1v15::VerifyingKey::<sha2::Sha224>::new(pk)
                    .verify_prehash(&pre, &s)
                    .is_ok(),
                o if o == OID_SHA256 => rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(pk)
                    .verify_prehash(&pre, &s)
                    .is_ok(),
                o if o == OID_SHA384 => rsa::pkcs1v15::VerifyingKey::<sha2::Sha384>::new(pk)
                    .verify_prehash(&pre, &s)
                    .is_ok(),
                o if o == OID_SHA512 => rsa::pkcs1v15::VerifyingKey::<sha2::Sha512>::new(pk)
                    .verify_prehash(&pre, &s)
                    .is_ok(),
                _ => return Err(format!("unsupported digest algorithm {d}")),
            }
        };
        return if ok { Ok(()) } else { Err(mismatch()) };
    }
    let ec_digest = match oid {
        o if o == OID_ECDSA_SHA1 => Some(OID_SHA1),
        o if o == OID_ECDSA_SHA224 => Some(OID_SHA224),
        o if o == OID_ECDSA_SHA256 => Some(OID_SHA256),
        o if o == OID_ECDSA_SHA384 => Some(OID_SHA384),
        o if o == OID_ECDSA_SHA512 => Some(OID_SHA512),
        o if o == OID_EC_PUBLIC => Some(digest_oid),
        _ => None,
    };
    if let Some(d) = ec_digest {
        if d == OID_SHA1 && !r.weak.iter().any(|w| w.contains("SHA-1")) {
            r.weak.push("SHA-1 signature".into());
        }
        let pre = if d == digest_oid {
            prehash.to_vec()
        } else {
            hash(d, &[message]).ok_or_else(mismatch)?
        };
        let curve = cert
            .tbs_certificate()
            .subject_public_key_info()
            .algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        use p256::pkcs8::DecodePublicKey;
        let ok = match curve {
            Some(c) if c == OID_P256 => {
                let vk = p256::ecdsa::VerifyingKey::from_public_key_der(&spki)
                    .map_err(|_| mismatch())?;
                let s = p256::ecdsa::Signature::from_der(sig).map_err(|_| mismatch())?;
                vk.verify_prehash(&pre, &s).is_ok()
            }
            Some(c) if c == OID_P384 => {
                let vk = p384::ecdsa::VerifyingKey::from_public_key_der(&spki)
                    .map_err(|_| mismatch())?;
                let s = p384::ecdsa::Signature::from_der(sig).map_err(|_| mismatch())?;
                vk.verify_prehash(&pre, &s).is_ok()
            }
            Some(c) if c == OID_P521 => {
                let pk = p521::PublicKey::from_public_key_der(&spki).map_err(|_| mismatch())?;
                let vk = p521::ecdsa::VerifyingKey::from_affine(*pk.as_affine())
                    .map_err(|_| mismatch())?;
                let s = p521::ecdsa::Signature::from_der(sig).map_err(|_| mismatch())?;
                vk.verify_prehash(&pre, &s).is_ok()
            }
            _ => return Err("unsupported elliptic curve".into()),
        };
        return if ok { Ok(()) } else { Err(mismatch()) };
    }
    Err(format!("unsupported signature algorithm {oid}"))
}

/// The hash of RSASSA-PSS parameters (`[0] hashAlgorithm`), when present.
fn pss_hash(alg: &AlgorithmIdentifierOwned) -> Option<ObjectIdentifier> {
    #[derive(der::Sequence)]
    struct PssParams {
        #[asn1(context_specific = "0", tag_mode = "EXPLICIT", optional = "true")]
        hash: Option<AlgorithmIdentifierOwned>,
        #[asn1(context_specific = "1", tag_mode = "EXPLICIT", optional = "true")]
        mgf: Option<AlgorithmIdentifierOwned>,
        #[asn1(context_specific = "2", tag_mode = "EXPLICIT", optional = "true")]
        salt: Option<u32>,
        #[asn1(context_specific = "3", tag_mode = "EXPLICIT", optional = "true")]
        trailer: Option<u32>,
    }
    let p = alg.parameters.as_ref()?.decode_as::<PssParams>().ok()?;
    p.hash.map(|h| h.oid)
}

/// A coarse summary of what later revisions changed, by object kind.
fn describe_changes(bytes: &[u8], signed_end: usize) -> Vec<String> {
    let (mut fields, mut sigs, mut other) = (false, false, false);
    let full = lopdf::Document::load_mem(bytes).ok();
    let signed = bytes
        .get(..signed_end)
        .and_then(|b| lopdf::Document::load_mem(b).ok());
    match (full, signed) {
        (Some(full), Some(signed)) => {
            for (id, obj) in &full.objects {
                let before = signed.objects.get(id);
                if before == Some(obj) {
                    continue;
                }
                let dict = match obj {
                    lopdf::Object::Dictionary(d) => Some(d),
                    lopdf::Object::Stream(s) => Some(&s.dict),
                    _ => None,
                };
                let Some(d) = dict else {
                    // Arrays (annotation or field lists) follow the objects they list.
                    continue;
                };
                let name = |k: &[u8]| d.get(k).ok().and_then(|n| n.as_name().ok());
                let is_sig_value = name(b"Type") == Some(b"Sig".as_slice())
                    || name(b"Type") == Some(b"DocTimeStamp".as_slice());
                let is_sig_field = name(b"FT") == Some(b"Sig".as_slice());
                if is_sig_value || (is_sig_field && d.has(b"V")) {
                    sigs = true;
                } else if d.has(b"FT")
                    || d.has(b"T")
                    || name(b"Subtype") == Some(b"Widget".as_slice())
                    || (name(b"Subtype") == Some(b"Form".as_slice()) && before.is_none())
                    || d.has(b"Fields")
                    || name(b"Type") == Some(b"Font".as_slice()) && before.is_none()
                {
                    fields = true;
                } else if name(b"Type") == Some(b"Catalog".as_slice())
                    || name(b"Type") == Some(b"XRef".as_slice())
                    || name(b"Type") == Some(b"ObjStm".as_slice())
                {
                    // Bookkeeping that accompanies any update.
                } else if name(b"Type") == Some(b"Page".as_slice())
                    && before.is_some_and(|b| page_only_annots_changed(b, obj))
                {
                    // A widget was added to the page.
                } else if name(b"Subtype") == Some(b"Image".as_slice()) && before.is_none() {
                    fields = true;
                } else {
                    other = true;
                }
            }
        }
        _ => other = true,
    }
    let mut out = Vec::new();
    if fields {
        out.push("form fields filled".to_string());
    }
    if sigs {
        out.push("signatures added".to_string());
    }
    if other || out.is_empty() {
        out.push("other changes".to_string());
    }
    out
}

fn page_only_annots_changed(before: &lopdf::Object, after: &lopdf::Object) -> bool {
    let (lopdf::Object::Dictionary(a), lopdf::Object::Dictionary(b)) = (before, after) else {
        return false;
    };
    let strip = |d: &lopdf::Dictionary| {
        let mut d = d.clone();
        d.remove(b"Annots");
        d
    };
    strip(a) == strip(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::load_pkcs12;
    use crate::fixtures::{self, certs};
    use crate::secret::Secret;
    use crate::sign::{Request, sign};

    fn signed() -> Vec<u8> {
        let p12 = certs::p12(&certs::P12Spec::default());
        let s = load_pkcs12(&p12, &Secret::new("test-pass"), SystemTime::now()).expect("signer");
        sign(&fixtures::form(), None, &s, &Request::default()).expect("sign")
    }

    fn reports(bytes: Vec<u8>) -> Vec<SignatureReport> {
        let doc = Document::open(bytes.into(), None).expect("open");
        verify_all(&doc)
    }

    #[test]
    fn tampered_byte_is_broken() {
        let mut bytes = signed();
        // Change a byte of page content inside the signed range.
        let at = crate::doc::find(&bytes, b"Application form").expect("text");
        bytes[at] = b'X';
        let r = reports(bytes);
        assert_eq!(
            r[0].status,
            SigStatus::Broken("document content does not match the signature".into())
        );
    }

    #[test]
    fn rebuilt_file_is_explained_and_keeps_its_signer() {
        let mut bytes = signed();
        // Re-saving shifts every object: the byte range no longer lands on /Contents.
        let at = crate::doc::find(&bytes, b"\n").expect("header") + 1;
        bytes.splice(at..at, b"% re-saved\n".iter().copied());
        let r = reports(bytes);
        match &r[0].status {
            SigStatus::Broken(w) => {
                assert!(w.contains("rebuilt after signing"), "{w}");
                assert!(w.contains("covers"), "{w}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            r[0].signer.as_ref().map(|c| c.name.as_str()),
            Some("Test Signer")
        );
        assert!(r[0].time.is_some());
    }

    #[test]
    fn tampered_signature_keeps_its_signer() {
        let mut bytes = signed();
        let at = crate::doc::find(&bytes, b"Application form").expect("text");
        bytes[at] = b'X';
        let r = reports(bytes);
        assert!(matches!(r[0].status, SigStatus::Broken(_)));
        assert!(r[0].signer.is_some());
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(size_text(999), "999 bytes");
        assert_eq!(size_text(167_344), "167 KB");
        assert_eq!(size_text(15_232_462), "15.2 MB");
    }

    #[test]
    fn filled_after_signing_is_reported() {
        let bytes = signed();
        let mut form = crate::forms::FormDoc::open(bytes.into(), None).expect("form");
        form.set("name", crate::forms::FieldInput::Text("Later".into()))
            .expect("fill");
        let r = reports(form.build(&[]).expect("build"));
        match &r[0].status {
            SigStatus::ChangedAfter(c) => {
                assert!(c.iter().any(|k| k == "form fields filled"), "{c:?}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malicious_byte_ranges_are_malformed() {
        let bytes = signed();
        let at = crate::doc::rfind(&bytes, b"/ByteRange").expect("range");
        let open = at + crate::doc::find(&bytes[at..], b"[").expect("[");
        let close = open + crate::doc::find(&bytes[open..], b"]").expect("]");
        let width = close - open - 1;
        for text in [
            format!("0 10 20 {}", 999_999_999),
            "0 10 5 1".to_string(),
            "-1 10 20 30".to_string(),
            "0 10".to_string(),
        ] {
            let mut b = bytes.clone();
            let mut t = text.clone();
            while t.len() < width {
                t.push(' ');
            }
            b.splice(open + 1..close, t.into_bytes());
            let r = reports(b);
            assert!(
                matches!(&r[0].status, SigStatus::Broken(w) if w.contains("malformed")),
                "{text}: {:?}",
                r[0].status
            );
        }
    }

    #[test]
    fn oversized_cms_is_refused() {
        let f = Found {
            field: "x".into(),
            byte_range: Some(vec![0, 1, 3, 0]),
            contents: vec![0u8; MAX_CMS_BYTES + 1],
            sub_filter: String::new(),
            m: None,
            reason: None,
            location: None,
            contact: None,
        };
        let r = verify_one(b"%<>", &f);
        assert!(matches!(r.status, SigStatus::Broken(_)));
    }

    #[test]
    fn dates_are_readable() {
        assert_eq!(
            pdf_date_text("D:20261119120000Z"),
            "2026-11-19 12:00:00 UTC"
        );
        assert_eq!(
            pdf_date_text("D:20261119120000+01'00'"),
            "2026-11-19 12:00:00 +0100"
        );
        assert_eq!(pdf_date_text("garbage"), "garbage");
    }
}
