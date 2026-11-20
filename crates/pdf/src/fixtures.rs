//! Small generated PDFs for tests (this crate's, the engine's and the UI snapshots), so
#![allow(clippy::expect_used, clippy::panic)] // Test support: a broken fixture should fail loudly.
//! no binary fixtures or system fonts are needed. Only the standard 14 fonts are used.

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions as LoPermissions};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

const A4: [i64; 4] = [0, 0, 595, 842];

fn s(text: &str) -> Object {
    Object::string_literal(text.as_bytes().to_vec())
}

fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    // Fixtures are built from known-good objects; an empty file fails the tests loudly.
    let _ = doc.save_to(&mut out);
    out
}

struct Builder {
    doc: Document,
    pages_id: ObjectId,
    font_id: ObjectId,
    pages: Vec<ObjectId>,
}

impl Builder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        Self {
            doc,
            pages_id,
            font_id,
            pages: Vec::new(),
        }
    }

    fn page(&mut self, content: &str, extra: Dictionary) -> ObjectId {
        let content_id = self
            .doc
            .add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => A4.iter().map(|v| Object::Integer(*v)).collect::<Vec<_>>(),
            "Contents" => content_id,
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => self.font_id, "Helv" => self.font_id },
            },
        };
        for (k, v) in extra.into_iter() {
            page.set(k, v);
        }
        let id = self.doc.add_object(page);
        self.pages.push(id);
        id
    }

    fn finish(mut self, catalog_extra: Dictionary) -> Document {
        let kids: Vec<Object> = self.pages.iter().map(|id| Object::Reference(*id)).collect();
        let count = kids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => count,
            }),
        );
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => self.pages_id };
        for (k, v) in catalog_extra.into_iter() {
            catalog.set(k, v);
        }
        let catalog_id = self.doc.add_object(catalog);
        self.doc.trailer.set("Root", catalog_id);
        let info = self.doc.add_object(dictionary! {
            "Title" => s("Fixture"),
            "Producer" => s("fm-pdf fixtures"),
        });
        self.doc.trailer.set("Info", info);
        self.doc.trailer.set(
            "ID",
            Object::Array(vec![
                Object::string_literal(vec![1u8; 16]),
                Object::string_literal(vec![2u8; 16]),
            ]),
        );
        self.doc
    }
}

fn text_content(n: usize) -> String {
    format!(
        "BT /F1 24 Tf 72 760 Td (Page {n}) Tj ET\n\
         BT /F1 12 Tf 72 720 Td (Total due on this invoice: 42 EUR) Tj ET\n\
         BT /F1 12 Tf 72 700 Td (Second line of text here) Tj ET\n"
    )
}

/// `n` A4 pages of text with an outline, page labels (i, ii, 1, 2, …) and links on the
/// first page (internal, web and an inert Launch action).
pub fn text_pages(n: usize) -> Vec<u8> {
    let n = n.max(1);
    let mut b = Builder::new();
    for i in 0..n {
        b.page(&text_content(i + 1), Dictionary::new());
    }
    let last = |i: usize| b.pages[i.min(n - 1)];
    let (p0, p1, p3, p4) = (last(0), last(1), last(3), last(4));
    let goto = b.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 716.into(), 300.into(), 734.into()],
        "A" => dictionary! { "S" => "GoTo", "D" => vec![p3.into(), "XYZ".into(), 0.into(), 842.into(), 0.into()] },
    });
    let uri = b.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 696.into(), 300.into(), 714.into()],
        "A" => dictionary! { "S" => "URI", "URI" => s("https://example.com/") },
    });
    let launch = b.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 600.into(), 300.into(), 620.into()],
        "A" => dictionary! { "S" => "Launch", "F" => s("calc.exe") },
    });
    if let Ok(Object::Dictionary(page)) = b.doc.get_object_mut(p0) {
        page.set("Annots", vec![goto.into(), uri.into(), launch.into()]);
    }
    let outlines = b.doc.new_object_id();
    let o1 = b.doc.new_object_id();
    let o2 = b.doc.new_object_id();
    let o3 = b.doc.new_object_id();
    b.doc.objects.insert(
        o1,
        Object::Dictionary(dictionary! {
            "Title" => s("Chapter 1"), "Parent" => outlines, "Next" => o3,
            "First" => o2, "Last" => o2, "Count" => 1,
            "Dest" => vec![p0.into(), "XYZ".into(), 0.into(), 842.into(), 0.into()],
        }),
    );
    b.doc.objects.insert(
        o2,
        Object::Dictionary(dictionary! {
            "Title" => s("Section 1.1"), "Parent" => o1,
            "Dest" => vec![p1.into(), "Fit".into()],
        }),
    );
    b.doc.objects.insert(
        o3,
        Object::Dictionary(dictionary! {
            "Title" => s("Chapter 3"), "Parent" => outlines, "Prev" => o1,
            "Dest" => vec![p4.into(), "Fit".into()],
        }),
    );
    b.doc.objects.insert(
        outlines,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines", "First" => o1, "Last" => o3, "Count" => 3,
        }),
    );
    let labels = dictionary! {
        "Nums" => vec![
            0.into(), Object::Dictionary(dictionary! { "S" => "r" }),
            2.into(), Object::Dictionary(dictionary! { "S" => "D" }),
        ],
    };
    let mut doc = b.finish(dictionary! { "Outlines" => outlines, "PageLabels" => labels });
    save(&mut doc)
}

/// One landscape-displayed page (`/Rotate 90`).
pub fn rotated_page() -> Vec<u8> {
    let mut b = Builder::new();
    b.page(&text_content(1), dictionary! { "Rotate" => 90 });
    let mut doc = b.finish(Dictionary::new());
    save(&mut doc)
}

/// A page with only an image (like a scan): no extractable text.
pub fn image_only() -> Vec<u8> {
    let mut b = Builder::new();
    let img = b.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0, 128, 200, 255],
    ));
    let content = "q 400 0 0 400 100 300 cm /Im0 Do Q";
    let page = b.page(content, Dictionary::new());
    if let Ok(Object::Dictionary(p)) = b.doc.get_object_mut(page)
        && let Ok(Object::Dictionary(res)) = p.get_mut(b"Resources")
    {
        res.set("XObject", dictionary! { "Im0" => img });
    }
    let mut doc = b.finish(Dictionary::new());
    save(&mut doc)
}

fn rc4_state(doc: &Document, user: &str, perms: LoPermissions) -> Option<EncryptionState> {
    EncryptionState::try_from(EncryptionVersion::V2 {
        document: doc,
        owner_password: "owner-pass",
        user_password: user,
        key_length: 128,
        permissions: perms,
    })
    .ok()
}

/// Encryption handlers for [`encrypted_with`].
#[derive(Clone, Copy, Debug)]
pub enum Cipher {
    Rc4,
    Aes128,
    Aes256,
}

/// [`text_pages`]`(2)` protected with user password `password` (RC4 128-bit).
pub fn encrypted(password: &str) -> Vec<u8> {
    encrypted_with(
        text_pages_doc(2),
        password,
        Cipher::Rc4,
        LoPermissions::all(),
    )
}

/// The form fixture, encrypted with an empty user password and form filling forbidden.
pub fn restricted_form() -> Vec<u8> {
    let perms = LoPermissions::all() - LoPermissions::FILLABLE - LoPermissions::ANNOTABLE;
    encrypted_with(form_doc(), "", Cipher::Rc4, perms)
}

/// `doc` encrypted with `cipher` and user password `password`.
pub fn encrypted_with(
    mut doc: Document,
    password: &str,
    cipher: Cipher,
    perms: LoPermissions,
) -> Vec<u8> {
    let state = match cipher {
        Cipher::Rc4 => rc4_state(&doc, password, perms),
        Cipher::Aes128 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V4 {
                document: &doc,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: "owner-pass",
                user_password: password,
                permissions: perms,
            })
            .ok()
        }
        Cipher::Aes256 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V5 {
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                file_encryption_key: &[7u8; 32],
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: "owner-pass",
                user_password: password,
                permissions: perms,
            })
            .ok()
        }
    };
    if let Some(state) = state {
        let _ = doc.encrypt(&state);
    }
    save(&mut doc)
}

/// [`text_pages`] as a lopdf document (for encryption fixtures).
pub fn text_pages_doc(n: usize) -> Document {
    Document::load_mem(&text_pages(n)).unwrap_or_default()
}

fn ap_stream(doc: &mut Document, content: &str, w: i64, h: i64) -> ObjectId {
    doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
        },
        content.as_bytes().to_vec(),
    ))
}

fn rect(r: [i64; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Integer(*v)).collect())
}

/// A one-page AcroForm: text `name` (required), multi-line `address`, comb `zip`
/// (max 5), checkbox `agree`, radio group `choice` (A/B), dropdown `country`,
/// multi-select list `colors` and an empty signature field `sig`.
pub fn form() -> Vec<u8> {
    save(&mut form_doc())
}

/// [`form`] whose `name` field has a calculate script (`/AA /C`), which is never run.
pub fn scripted_form() -> Vec<u8> {
    let mut doc = form_doc();
    let script = dictionary! {
        "S" => "JavaScript",
        "JS" => s("AFSimple_Calculate(\"SUM\", new Array(\"zip\"));"),
    };
    let ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, o)| {
            o.as_dict()
                .ok()
                .and_then(|d| d.get(b"T").ok())
                .and_then(|t| t.as_str().ok())
                == Some(b"name".as_slice())
        })
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("AA", dictionary! { "C" => script.clone() });
        }
    }
    save(&mut doc)
}

/// [`form`] as a lopdf document.
pub fn form_doc() -> Document {
    let mut b = Builder::new();
    let page = b.page(
        "BT /F1 18 Tf 72 780 Td (Application form) Tj ET\n\
         BT /F1 10 Tf 30 705 Td (Name) Tj ET",
        Dictionary::new(),
    );
    let doc = &mut b.doc;
    let da = s("/Helv 12 Tf 0 g");
    let mut widgets: Vec<ObjectId> = Vec::new();
    let mut fields: Vec<ObjectId> = Vec::new();
    let mut field = |doc: &mut Document, d: Dictionary| {
        let id = doc.add_object(d);
        widgets.push(id);
        fields.push(id);
        id
    };
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Tx", "T" => s("name"), "Ff" => 2, "DA" => da.clone(),
            "Rect" => rect([100, 700, 300, 720]), "F" => 4,
        },
    );
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Tx", "T" => s("address"), "Ff" => 4096, "DA" => s("/Helv 10 Tf 0 g"),
            "Rect" => rect([100, 620, 300, 690]), "F" => 4,
        },
    );
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Tx", "T" => s("zip"), "Ff" => 16_777_216, "MaxLen" => 5, "DA" => da.clone(),
            "Rect" => rect([320, 700, 420, 720]), "F" => 4, "Q" => 0,
        },
    );
    let yes = ap_stream(doc, "q 0 g 2 2 11 11 re f Q", 15, 15);
    let off = ap_stream(doc, "", 15, 15);
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Btn", "T" => s("agree"), "V" => "Off", "AS" => "Off",
            "Rect" => rect([100, 590, 115, 605]), "F" => 4,
            "AP" => dictionary! { "N" => dictionary! { "Yes" => yes, "Off" => off } },
        },
    );
    // Radio group: a parent field with two widget kids.
    let radio = doc.new_object_id();
    let mut kids = Vec::new();
    for (i, state) in ["A", "B"].iter().enumerate() {
        let on = ap_stream(doc, "q 0 g 4 4 7 7 re f Q", 15, 15);
        let off = ap_stream(doc, "", 15, 15);
        let x = 100 + 40 * i as i64;
        let mut n = Dictionary::new();
        n.set(*state, on);
        n.set("Off", off);
        let kid = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page, "Parent" => radio,
            "AS" => "Off", "Rect" => rect([x, 560, x + 15, 575]), "F" => 4,
            "AP" => dictionary! { "N" => n },
        });
        widgets.push(kid);
        kids.push(Object::Reference(kid));
    }
    doc.objects.insert(
        radio,
        Object::Dictionary(dictionary! {
            "FT" => "Btn", "T" => s("choice"), "Ff" => 49152, "V" => "Off", "Kids" => kids,
        }),
    );
    fields.push(radio);
    let mut field = |doc: &mut Document, d: Dictionary| {
        let id = doc.add_object(d);
        widgets.push(id);
        fields.push(id);
        id
    };
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Ch", "T" => s("country"), "Ff" => 131_072, "DA" => da.clone(),
            "Opt" => vec![s("Netherlands"), s("Belgium"), s("Germany")],
            "Rect" => rect([100, 520, 300, 540]), "F" => 4,
        },
    );
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Ch", "T" => s("colors"), "Ff" => 2_097_152, "DA" => da.clone(),
            "Opt" => vec![s("Red"), s("Green"), s("Blue")],
            "Rect" => rect([100, 450, 300, 510]), "F" => 4,
        },
    );
    field(
        doc,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "FT" => "Sig", "T" => s("sig"),
            "Rect" => rect([100, 350, 300, 410]), "F" => 4,
        },
    );
    if let Ok(Object::Dictionary(p)) = doc.get_object_mut(page) {
        p.set(
            "Annots",
            widgets
                .iter()
                .map(|w| Object::Reference(*w))
                .collect::<Vec<_>>(),
        );
    }
    let font = b.font_id;
    let acro = dictionary! {
        "Fields" => fields.iter().map(|f| Object::Reference(*f)).collect::<Vec<_>>(),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } },
        "DA" => s("/Helv 0 Tf 0 g"),
    };
    let acro_id = b.doc.add_object(acro);
    b.finish(dictionary! { "AcroForm" => acro_id })
}

/// [`form`] with an `/XFA` entry as well (a hybrid form).
pub fn hybrid_xfa() -> Vec<u8> {
    let mut doc = form_doc();
    let xfa = doc.add_object(Stream::new(
        dictionary! {},
        b"<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"></xdp:xdp>".to_vec(),
    ));
    let acro_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| o.as_reference().ok());
    if let Some(id) = acro_id
        && let Ok(Object::Dictionary(acro)) = doc.get_object_mut(id)
    {
        acro.set("XFA", xfa);
    }
    save(&mut doc)
}

/// Test certificates and PKCS#12 files, generated at run time (no private key is
/// committed). A P-256 "Test CA" issues a "Test Signer" certificate.
pub mod certs {
    use std::time::{Duration, SystemTime};

    use der::asn1::{Any, OctetString, SetOfVec};
    use der::{Decode, Encode};
    use pkcs8::{EncodePrivateKey, EncodePublicKey};
    use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned, SubjectPublicKeyInfoRef};
    use x509_cert::builder::profile::BuilderProfile;
    use x509_cert::builder::{Builder, CertificateBuilder};
    use x509_cert::ext::Extension;
    use x509_cert::ext::pkix::{KeyUsage, KeyUsages};
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::{Time, Validity};
    use x509_cert::{Certificate, TbsCertificate};

    /// Key of the signer certificate.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum KeySpec {
        #[default]
        P256,
        P384,
        Rsa2048,
    }

    #[derive(Clone, Debug)]
    pub struct P12Spec {
        pub key: KeySpec,
        /// 3DES-shrouded key and RC2-40 encrypted certificates (old exports).
        pub legacy: bool,
        /// Key usage only allows encipherment (no signing).
        pub encipher_only: bool,
        pub password: String,
        pub name: String,
    }

    impl Default for P12Spec {
        fn default() -> Self {
            Self {
                key: KeySpec::P256,
                legacy: false,
                encipher_only: false,
                password: "test-pass".into(),
                name: "Test Signer".into(),
            }
        }
    }

    struct Profile {
        subject: Name,
        issuer: Name,
        usage: Option<der::flagset::FlagSet<KeyUsages>>,
    }

    impl BuilderProfile for Profile {
        fn get_issuer(&self, _subject: &Name) -> Name {
            self.issuer.clone()
        }
        fn get_subject(&self) -> Name {
            self.subject.clone()
        }
        fn build_extensions(
            &self,
            _spk: SubjectPublicKeyInfoRef<'_>,
            _issuer_spk: SubjectPublicKeyInfoRef<'_>,
            _tbs: &TbsCertificate,
        ) -> x509_cert::builder::Result<Vec<Extension>> {
            let Some(usage) = self.usage else {
                return Ok(Vec::new());
            };
            let value = KeyUsage(usage).to_der()?;
            Ok(vec![Extension {
                extn_id: der::oid::ObjectIdentifier::new_unwrap("2.5.29.15"),
                critical: true,
                extn_value: OctetString::new(value)?,
            }])
        }
    }

    fn name(cn: &str) -> Name {
        format!("CN={cn},O=Example Test Org")
            .parse()
            .unwrap_or_default()
    }

    fn validity() -> Validity {
        let now = SystemTime::now();
        let start = now - Duration::from_secs(3600);
        let end = now + Duration::from_secs(365 * 24 * 3600);
        Validity::new(
            Time::try_from(start).unwrap_or(Time::INFINITY),
            Time::try_from(end).unwrap_or(Time::INFINITY),
        )
    }

    fn ca_key() -> p256::ecdsa::SigningKey {
        // Deterministic test key (not a secret: tests only).
        p256::ecdsa::SigningKey::from_slice(&[0x11; 32]).unwrap_or_else(|_| {
            p256::ecdsa::SigningKey::from_slice(&[0x22; 32]).unwrap_or_else(|_| unreachable_key())
        })
    }

    #[allow(clippy::panic)]
    fn unreachable_key() -> p256::ecdsa::SigningKey {
        panic!("fixed test key rejected")
    }

    fn issue(
        ca: &p256::ecdsa::SigningKey,
        subject: Name,
        issuer: Name,
        spki: SubjectPublicKeyInfoOwned,
        usage: Option<der::flagset::FlagSet<KeyUsages>>,
        serial: u32,
    ) -> Certificate {
        let profile = Profile {
            subject,
            issuer,
            usage,
        };
        let serial = SerialNumber::from(serial);
        let builder = CertificateBuilder::new(profile, serial, validity(), spki);
        #[allow(clippy::expect_used)]
        builder
            .expect("builder")
            .build::<_, p256::ecdsa::DerSignature>(ca)
            .expect("certificate")
    }

    fn spki_of(der: &[u8]) -> SubjectPublicKeyInfoOwned {
        SubjectPublicKeyInfoOwned::from_der(der).unwrap_or_else(|_| {
            SubjectPublicKeyInfoOwned::from_der(&[]).unwrap_or_else(|_| unreachable_spki())
        })
    }

    #[allow(clippy::panic)]
    fn unreachable_spki() -> SubjectPublicKeyInfoOwned {
        panic!("public key encoding failed")
    }

    /// `(pkcs8 key DER, signer certificate, CA certificate)`.
    #[allow(clippy::expect_used)]
    pub fn material(spec: &P12Spec) -> (Vec<u8>, Certificate, Certificate) {
        let ca = ca_key();
        let ca_spki = spki_of(
            ca.verifying_key()
                .to_public_key_der()
                .expect("ca spki")
                .as_bytes(),
        );
        let ca_name = name("Test CA");
        let ca_cert = issue(
            &ca,
            ca_name.clone(),
            ca_name.clone(),
            ca_spki,
            Some(KeyUsages::KeyCertSign | KeyUsages::CRLSign),
            1,
        );
        let (key_der, spki) = match spec.key {
            KeySpec::P256 => {
                let k = p256::SecretKey::from_slice(&[0x33; 32]).expect("p256");
                (
                    k.to_pkcs8_der().expect("pkcs8").as_bytes().to_vec(),
                    k.public_key().to_public_key_der().expect("spki"),
                )
            }
            KeySpec::P384 => {
                let k = p384::SecretKey::from_slice(&[0x44; 48]).expect("p384");
                (
                    k.to_pkcs8_der().expect("pkcs8").as_bytes().to_vec(),
                    k.public_key().to_public_key_der().expect("spki"),
                )
            }
            KeySpec::Rsa2048 => {
                let mut rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
                let k = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
                (
                    k.to_pkcs8_der().expect("pkcs8").as_bytes().to_vec(),
                    k.to_public_key().to_public_key_der().expect("spki"),
                )
            }
        };
        let usage = if spec.encipher_only {
            KeyUsages::KeyEncipherment | KeyUsages::DataEncipherment
        } else {
            KeyUsages::DigitalSignature | KeyUsages::NonRepudiation
        };
        let cert = issue(
            &ca,
            name(&spec.name),
            ca_name,
            spki_of(spki.as_bytes()),
            Some(usage),
            2,
        );
        (key_der, cert, ca_cert)
    }

    fn local_id_attr() -> x509_cert::attr::Attributes {
        let mut values = SetOfVec::new();
        #[allow(clippy::expect_used)]
        values
            .insert(Any::encode_from(&OctetString::new(vec![1, 2, 3, 4]).expect("o")).expect("a"))
            .expect("insert");
        let mut attrs = SetOfVec::new();
        #[allow(clippy::expect_used)]
        attrs
            .insert(x509_cert::attr::Attribute {
                oid: der::oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.21"),
                values,
            })
            .expect("insert");
        attrs
    }

    fn legacy_encrypt(
        oid: der::oid::ObjectIdentifier,
        data: &[u8],
        pw: &str,
    ) -> (AlgorithmIdentifierOwned, Vec<u8>) {
        use cbc::cipher::block_padding::Pkcs7;
        use cbc::cipher::{BlockModeEncrypt, InnerIvInit, KeyIvInit};
        use pkcs12::kdf::{Pkcs12KeyType, derive_key_utf8};
        let salt = [9u8; 8];
        let iterations = 2048;
        #[allow(clippy::expect_used)]
        let iv =
            derive_key_utf8::<sha1::Sha1>(pw, &salt, Pkcs12KeyType::Iv, iterations, 8).expect("iv");
        let ct = if oid == pkcs12::PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC {
            #[allow(clippy::expect_used)]
            let key = derive_key_utf8::<sha1::Sha1>(
                pw,
                &salt,
                Pkcs12KeyType::EncryptionKey,
                iterations,
                24,
            )
            .expect("key");
            #[allow(clippy::expect_used)]
            cbc::Encryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .expect("3des")
                .encrypt_padded_vec::<Pkcs7>(data)
        } else {
            #[allow(clippy::expect_used)]
            let key = derive_key_utf8::<sha1::Sha1>(
                pw,
                &salt,
                Pkcs12KeyType::EncryptionKey,
                iterations,
                5,
            )
            .expect("key");
            let cipher = rc2::Rc2::new_with_eff_key_len(&key, 40);
            #[allow(clippy::expect_used)]
            cbc::Encryptor::<rc2::Rc2>::inner_iv_slice_init(cipher, &iv)
                .expect("rc2")
                .encrypt_padded_vec::<Pkcs7>(data)
        };
        #[allow(clippy::expect_used)]
        let params = pkcs12::pbe_params::Pkcs12PbeParams {
            salt: OctetString::new(salt.to_vec()).expect("salt"),
            iterations,
        };
        #[allow(clippy::expect_used)]
        let alg = AlgorithmIdentifierOwned {
            oid,
            parameters: Some(Any::encode_from(&params).expect("params")),
        };
        (alg, ct)
    }

    fn data_info(content: &[u8]) -> cms::content_info::ContentInfo {
        #[allow(clippy::expect_used)]
        cms::content_info::ContentInfo {
            content_type: der::oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
            content: Any::encode_from(&OctetString::new(content.to_vec()).expect("o")).expect("a"),
        }
    }

    /// A complete PKCS#12 file.
    #[allow(clippy::expect_used)]
    pub fn p12(spec: &P12Spec) -> Vec<u8> {
        let (key_der, cert, ca) = material(spec);
        let pw = spec.password.as_str();
        // Shrouded key bag.
        let (alg, enc) = if spec.legacy {
            legacy_encrypt(
                pkcs12::PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC,
                &key_der,
                pw,
            )
        } else {
            let params = pkcs5::pbes2::Parameters::generate_pbkdf2_sha256_aes256cbc(
                2048, &[7u8; 16], [5u8; 16],
            )
            .expect("pbes2");
            let scheme = pkcs5::EncryptionScheme::Pbes2(params);
            let ct = scheme.encrypt(pw.as_bytes(), &key_der).expect("encrypt");
            let alg =
                AlgorithmIdentifierOwned::from_der(&scheme.to_der().expect("der")).expect("alg");
            (alg, ct)
        };
        let epki = pkcs12::pbe_params::EncryptedPrivateKeyInfo {
            encryption_algorithm: alg,
            encrypted_data: OctetString::new(enc).expect("o"),
        };
        let key_bag = pkcs12::safe_bag::SafeBag {
            bag_id: pkcs12::PKCS_12_PKCS8_KEY_BAG_OID,
            bag_value: epki.to_der().expect("epki"),
            bag_attributes: Some(local_id_attr()),
        };
        let cert_bag = |c: &Certificate, attrs| {
            let cb = pkcs12::CertBag {
                cert_id: pkcs12::PKCS_12_X509_CERT_OID,
                cert_value: OctetString::new(c.to_der().expect("cert")).expect("o"),
            };
            pkcs12::safe_bag::SafeBag {
                bag_id: pkcs12::PKCS_12_CERT_BAG_OID,
                bag_value: cb.to_der().expect("cb"),
                bag_attributes: attrs,
            }
        };
        let key_contents = vec![key_bag].to_der().expect("key safe");
        let cert_contents = vec![cert_bag(&cert, Some(local_id_attr())), cert_bag(&ca, None)]
            .to_der()
            .expect("cert safe");
        let cert_info = if spec.legacy {
            let (alg, ct) = legacy_encrypt(
                pkcs12::PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC,
                &cert_contents,
                pw,
            );
            let ed = cms::encrypted_data::EncryptedData {
                version: cms::content_info::CmsVersion::V0,
                enc_content_info: cms::enveloped_data::EncryptedContentInfo {
                    content_type: der::oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
                    content_enc_alg: alg,
                    encrypted_content: Some(OctetString::new(ct).expect("o")),
                },
                unprotected_attrs: None,
            };
            cms::content_info::ContentInfo {
                content_type: der::oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.6"),
                content: Any::encode_from(&ed).expect("ed"),
            }
        } else {
            data_info(&cert_contents)
        };
        let auth_safe = vec![data_info(&key_contents), cert_info]
            .to_der()
            .expect("auth safe");
        // HMAC-SHA256 over the authenticated safe.
        use hmac::{KeyInit, Mac};
        let salt = [3u8; 8];
        let mac_key = pkcs12::kdf::derive_key_utf8::<sha2::Sha256>(
            pw,
            &salt,
            pkcs12::kdf::Pkcs12KeyType::Mac,
            2048,
            32,
        )
        .expect("mac key");
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&mac_key).expect("hmac");
        mac.update(&auth_safe);
        let digest = mac.finalize().into_bytes().to_vec();
        let pfx = pkcs12::pfx::Pfx {
            version: pkcs12::pfx::Version::V3,
            auth_safe: data_info(&auth_safe),
            mac_data: Some(pkcs12::MacData {
                mac: pkcs12::DigestInfo {
                    algorithm: AlgorithmIdentifierOwned {
                        oid: der::oid::ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1"),
                        parameters: None,
                    },
                    digest: OctetString::new(digest).expect("o"),
                },
                mac_salt: OctetString::new(salt.to_vec()).expect("o"),
                iterations: 2048,
            }),
        };
        pfx.to_der().expect("pfx")
    }
}
