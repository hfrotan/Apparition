//! Signing with the user's own certificate and provisioning profiles, without an Apple ID.
//!
//! A paid developer account can make an Apple Development or Distribution certificate and
//! year-long profiles that list this headset (on the developer website, or in Xcode), export
//! the certificate as a .p12 and bring both here. Apparition then signs on its own, like
//! build-and-sign does on a Mac: no sign-in, no anisette, no 7-day limit, and no App ID or
//! certificate slots used up.
//!
//! What's kept, under `<data_dir>/signing/`:
//! - `identities/<sha1>/certificate.der` and `key.der` (PKCS#8): the .p12's certificate and
//!   private key, decrypted, so a refresh needs nobody to type the .p12 password. They sit in
//!   the app's container like the free account's key does (storage.json). The password itself
//!   is never stored.
//! - `profiles/<UUID>.mobileprovision`: the profiles as imported.

use std::{
    ffi::c_char,
    path::{Path, PathBuf},
    time::SystemTime,
};

use apple_codesign_quick::{BundleSigningSettings, ProvisioningProfile, RustCryptoCmsSigner, sign_bundle};
use isideload::sideload::{application::Application, cert_identity};
use p12_keystore::{KeyStore, KeyStoreEntry, Pkcs12ImportPolicy};
use plist::{Dictionary, Value as Plist};
use rootcause::prelude::*;
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPublicKey, pkcs8::DecodePrivateKey};
use serde_json::{Value, json};
use x509_cert::{
    Certificate,
    der::{Decode, asn1::{PrintableStringRef, Utf8StringRef}},
    name::Name,
};

use crate::{reply, text};

/// Intermediates a developer certificate may be issued by, besides the G3 that isideload
/// bundles, and the root G6 chains to. Usually the .p12 holds only the leaf certificate.
const EXTRA_CHAIN: &[&[u8]] = &[
    include_bytes!("../assets/AppleWWDRCAG4.cer"),
    include_bytes!("../assets/AppleWWDRCAG5.cer"),
    include_bytes!("../assets/AppleWWDRCAG6.cer"),
    include_bytes!("../assets/AppleRootCA-G3.cer"),
];

// MARK: Storage

fn signing_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("signing")
}

fn identities_dir(data_dir: &Path) -> PathBuf {
    signing_dir(data_dir).join("identities")
}

fn profiles_dir(data_dir: &Path) -> PathBuf {
    signing_dir(data_dir).join("profiles")
}

/// A certificate with its private key.
struct Identity {
    id: String,
    certificate: Certificate,
    key: RsaPrivateKey,
}

impl Identity {
    fn load(data_dir: &Path, id: &str) -> Result<Self, Report> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("bad certificate id");
        }
        let dir = identities_dir(data_dir).join(id);
        let certificate_der = std::fs::read(dir.join("certificate.der"))
            .context("That certificate isn't imported (any more)")?;
        let key = RsaPrivateKey::from_pkcs8_der(&std::fs::read(dir.join("key.der"))?)
            .map_err(|e| report!("The stored private key can't be read: {e}"))?;
        let certificate = Certificate::from_der(&certificate_der)?;
        Ok(Self { id: id.to_string(), certificate, key })
    }

    fn all(data_dir: &Path) -> Vec<Self> {
        let Ok(entries) = std::fs::read_dir(identities_dir(data_dir)) else { return Vec::new() };
        let mut identities: Vec<Self> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| Self::load(data_dir, &entry.file_name().to_string_lossy()).ok())
            .collect();
        identities.sort_by_key(|identity| std::cmp::Reverse(not_after(&identity.certificate)));
        identities
    }

    fn json(&self) -> Value {
        let subject = &self.certificate.tbs_certificate.subject;
        let name = attribute(subject, COMMON_NAME).unwrap_or_else(|| "Certificate".into());
        let expires = not_after(&self.certificate);
        json!({
            "id": self.id,
            "name": name,
            "teamId": attribute(subject, ORGANIZATIONAL_UNIT),
            "teamName": attribute(subject, ORGANIZATION),
            "kind": certificate_kind(&name),
            "serialNumber": serial(&self.certificate),
            "expires": iso(expires),
            "expired": expires < SystemTime::now(),
        })
    }
}

/// A provisioning profile, parsed.
struct Profile {
    data: Vec<u8>,
    parsed: ProvisioningProfile,
}

impl Profile {
    fn parse(data: Vec<u8>) -> Result<Self, Report> {
        let parsed = ProvisioningProfile::parse(&data).context("This isn't a provisioning profile")?;
        Ok(Self { data, parsed })
    }

    fn all(data_dir: &Path) -> Vec<Self> {
        let Ok(entries) = std::fs::read_dir(profiles_dir(data_dir)) else { return Vec::new() };
        let mut profiles: Vec<Self> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "mobileprovision"))
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|data| Self::parse(data).ok())
            .collect();
        profiles.sort_by(|a, b| a.name().cmp(&b.name()));
        profiles
    }

    fn plist(&self) -> &Dictionary {
        self.parsed.plist()
    }

    fn uuid(&self) -> String {
        self.plist().get("UUID").and_then(Plist::as_string).unwrap_or_default().to_string()
    }

    fn name(&self) -> String {
        self.plist().get("Name").and_then(Plist::as_string).unwrap_or("Profile").to_string()
    }

    fn team(&self) -> &str {
        self.parsed.team_id()
    }

    fn expires(&self) -> Option<SystemTime> {
        self.plist().get("ExpirationDate").and_then(Plist::as_date).map(SystemTime::from)
    }

    fn expired(&self) -> bool {
        self.expires().is_none_or(|expires| expires < SystemTime::now())
    }

    /// What follows the team prefix in `application-identifier`: `com.example.app`,
    /// `com.example.*` or `*`.
    fn app_id_pattern(&self) -> String {
        let identifier = self
            .parsed
            .entitlements()
            .get("application-identifier")
            .and_then(Plist::as_string)
            .unwrap_or_default();
        identifier.strip_prefix(&format!("{}.", self.team())).unwrap_or(identifier).to_string()
    }

    fn get_task_allow(&self) -> bool {
        self.parsed.entitlements().get("get-task-allow").and_then(Plist::as_boolean).unwrap_or(false)
    }

    fn all_devices(&self) -> bool {
        self.plist().get("ProvisionsAllDevices").and_then(Plist::as_boolean).unwrap_or(false)
    }

    fn devices(&self) -> Vec<&str> {
        self.plist()
            .get("ProvisionedDevices")
            .and_then(Plist::as_array)
            .map(|devices| devices.iter().filter_map(Plist::as_string).collect())
            .unwrap_or_default()
    }

    /// development (debuggable, so JIT works), adHoc, enterprise or appStore.
    fn kind(&self) -> &'static str {
        if self.all_devices() {
            "enterprise"
        } else if self.devices().is_empty() {
            "appStore"
        } else if self.get_task_allow() {
            "development"
        } else {
            "adHoc"
        }
    }

    fn includes(&self, udid: Option<&str>) -> Option<bool> {
        udid.map(|udid| self.all_devices() || self.devices().iter().any(|d| d.eq_ignore_ascii_case(udid)))
    }

    /// SHA-1 fingerprints of the certificates the profile allows to sign.
    fn certificate_ids(&self) -> Vec<String> {
        self.plist()
            .get("DeveloperCertificates")
            .and_then(Plist::as_array)
            .map(|certificates| certificates.iter().filter_map(Plist::as_data).map(fingerprint).collect())
            .unwrap_or_default()
    }

    /// Whether apps signed with it may run on visionOS (a tvOS- or iOS-only profile can't).
    fn for_vision(&self) -> bool {
        let platforms = self.platforms();
        platforms.is_empty() || platforms.iter().any(|p| matches!(*p, "xrOS" | "visionOS"))
    }

    fn platforms(&self) -> Vec<&str> {
        self.plist()
            .get("Platform")
            .and_then(Plist::as_array)
            .map(|platforms| platforms.iter().filter_map(Plist::as_string).collect())
            .unwrap_or_default()
    }

    /// The bundle ID this profile signs `original` as, if it can: an explicit App ID always
    /// renames the app to itself; a wildcard keeps `original` when it fits, or prefixes it.
    fn bundle_id_for(&self, original: &str) -> String {
        bundle_id_for(&self.app_id_pattern(), original)
    }

    fn covers(&self, bundle_id: &str) -> bool {
        covers(&self.app_id_pattern(), bundle_id)
    }

    fn json(&self, identities: &[Identity], udid: Option<&str>) -> Value {
        let certificates = self.certificate_ids();
        json!({
            "uuid": self.uuid(),
            "name": self.name(),
            "teamId": self.team(),
            "teamName": self.plist().get("TeamName").and_then(Plist::as_string),
            "appIdPattern": self.app_id_pattern(),
            "kind": self.kind(),
            "debuggable": self.get_task_allow(),
            "platforms": self.platforms(),
            "forVision": self.for_vision(),
            "deviceCount": self.devices().len(),
            "includesDevice": self.includes(udid),
            "expires": self.expires().map(iso),
            "expired": self.expired(),
            "identityIds": identities.iter().filter(|i| certificates.contains(&i.id)).map(|i| i.id.clone()).collect::<Vec<_>>(),
        })
    }
}

fn bundle_id_for(pattern: &str, original: &str) -> String {
    match pattern.strip_suffix('*') {
        None => pattern.to_string(),
        Some(prefix) if original.starts_with(prefix) => original.to_string(),
        Some(prefix) => format!("{prefix}{original}"),
    }
}

fn covers(pattern: &str, bundle_id: &str) -> bool {
    match pattern.strip_suffix('*') {
        None => pattern == bundle_id,
        Some(prefix) => bundle_id.starts_with(prefix),
    }
}

// MARK: Certificate helpers

const COMMON_NAME: &str = "2.5.4.3";
const ORGANIZATION: &str = "2.5.4.10";
const ORGANIZATIONAL_UNIT: &str = "2.5.4.11";

fn attribute(name: &Name, oid: &str) -> Option<String> {
    name.0.iter().flat_map(|rdn| rdn.0.iter()).find(|a| a.oid.to_string() == oid).and_then(|a| {
        Utf8StringRef::try_from(&a.value)
            .map(|s| s.as_str().to_string())
            .or_else(|_| PrintableStringRef::try_from(&a.value).map(|s| s.as_str().to_string()))
            .ok()
    })
}

fn not_after(certificate: &Certificate) -> SystemTime {
    certificate.tbs_certificate.validity.not_after.to_system_time()
}

fn serial(certificate: &Certificate) -> String {
    let bytes = certificate.tbs_certificate.serial_number.as_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    hex.trim_start_matches('0').to_string()
}

fn certificate_kind(name: &str) -> &'static str {
    if name.contains("Distribution") {
        "distribution"
    } else if name.contains("Development") || name.contains("Developer") {
        "development"
    } else {
        "other"
    }
}

fn fingerprint(der: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, der)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

fn iso(time: SystemTime) -> String {
    plist::Date::from(time).to_xml_format()
}

/// The signer's chain up to its root, from the profile's certificates and the Apple
/// intermediates bundled with Apparition and isideload.
fn chain(certificate: &Certificate, profile: &ProvisioningProfile) -> Result<Vec<Certificate>, Report> {
    let mut candidates: Vec<Certificate> = Vec::new();
    let bundled = [cert_identity::APPLE_WWDR_G3_CERTIFICATE_DER, cert_identity::APPLE_ROOT]
        .into_iter()
        .chain(EXTRA_CHAIN.iter().copied());
    for der in profile.certificate_chain_der().iter().map(Vec::as_slice).chain(bundled) {
        if let Ok(candidate) = Certificate::from_der(der) {
            candidates.push(candidate);
        }
    }
    let mut chain = Vec::new();
    let mut issuer = certificate.tbs_certificate.issuer.clone();
    if issuer == certificate.tbs_certificate.subject {
        return Ok(chain);
    }
    loop {
        let next = candidates
            .iter()
            .find(|candidate| candidate.tbs_certificate.subject == issuer)
            .ok_or_else(|| report!("Apparition doesn't know the certificate that issued this one ({issuer})"))?
            .clone();
        let root = next.tbs_certificate.subject == next.tbs_certificate.issuer;
        issuer = next.tbs_certificate.issuer.clone();
        if chain.contains(&next) {
            bail!("certificate chain contains a cycle");
        }
        chain.push(next);
        if root || chain.len() > 8 {
            return Ok(chain);
        }
    }
}

/// The profile's entitlements made concrete for `bundle_id`: wildcard App IDs and keychain
/// groups become the bundle's own, and any other wildcard (associated domains, iCloud
/// containers a wildcard profile allows) is left out, since a signature can't claim it.
fn entitlements_for(profile: &Profile, bundle_id: &str) -> Dictionary {
    let team = profile.team();
    let concrete = format!("{team}.{bundle_id}");
    let mut entitlements = Dictionary::new();
    for (key, value) in profile.parsed.entitlements() {
        let value = match (key.as_str(), value) {
            ("application-identifier", _) => Plist::String(concrete.clone()),
            ("com.apple.developer.team-identifier", _) => Plist::String(team.to_string()),
            ("keychain-access-groups", Plist::Array(groups)) => Plist::Array(
                groups
                    .iter()
                    .filter_map(Plist::as_string)
                    .map(|group| if group.ends_with('*') { concrete.clone() } else { group.to_string() })
                    .map(Plist::String)
                    .collect(),
            ),
            (_, Plist::String(text)) if text.contains('*') => continue,
            (_, Plist::Array(items)) => {
                let kept: Vec<Plist> = items
                    .iter()
                    .filter(|item| !item.as_string().is_some_and(|text| text.contains('*')))
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                Plist::Array(kept)
            }
            (_, other) => other.clone(),
        };
        entitlements.insert(key.clone(), value);
    }
    sorted(entitlements)
}

/// DER entitlements must list keys in order, or the OS ignores them all ("invalid
/// entitlements blob"). Apple's own profiles list them unsorted; the free account's few keys
/// happen to be in order already.
fn sorted(dictionary: Dictionary) -> Dictionary {
    let mut entries: Vec<(String, Plist)> = dictionary.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(key, value)| match value {
            Plist::Dictionary(inner) => (key, Plist::Dictionary(sorted(inner))),
            other => (key, other),
        })
        .collect()
}

fn list(data_dir: &Path, udid: Option<&str>) -> Value {
    let identities = Identity::all(data_dir);
    let profiles = Profile::all(data_dir);
    json!({
        "identities": identities.iter().map(Identity::json).collect::<Vec<_>>(),
        "profiles": profiles.iter().map(|p| p.json(&identities, udid)).collect::<Vec<_>>(),
    })
}

unsafe fn optional(pointer: *const c_char) -> Result<Option<String>, Report> {
    if pointer.is_null() {
        return Ok(None);
    }
    Ok(Some(unsafe { text(pointer) }?).filter(|s| !s.is_empty()))
}

// MARK: C ABI

/// The imported certificates and profiles: `{ok, identities: [{id, name, teamId, teamName,
/// kind, serialNumber, expires, expired}], profiles: [{uuid, name, teamId, appIdPattern, kind,
/// debuggable, platforms, deviceCount, includesDevice, expires, expired, identityIds}]}`.
/// `udid` (may be NULL) fills in `includesDevice`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_signing_list(data_dir: *const c_char, udid: *const c_char) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let udid = unsafe { optional(udid) }?;
        Ok(list(&data_dir, udid.as_deref()))
    })())
}

/// Imports the certificate and private key from a .p12 (as exported from Keychain Access or
/// Xcode). The password opens the file and is then forgotten. Returns `{ok, identity}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_identity_import(
    data_dir: *const c_char,
    p12_path: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let p12 = std::fs::read(unsafe { text(p12_path) }?)?;
        let password = unsafe { text(password) }?;
        let store = KeyStore::from_pkcs12(&p12, &password, Pkcs12ImportPolicy::Relaxed).map_err(|error| {
            report!("Can't open the .p12: {error:?}. Check the password; it is the one chosen when the certificate was exported.")
        })?;
        drop(password);

        let (_, keys) = store
            .private_key_chain()
            .ok_or_else(|| report!("The .p12 has no private key. Export the certificate together with its key (in Keychain Access, from My Certificates)."))?;
        let key = RsaPrivateKey::from_pkcs8_der(keys.key().as_der())
            .map_err(|_| report!("The .p12's key isn't an RSA key, which Apple code signing certificates use"))?;
        let public = key.to_public_key().to_pkcs1_der()?;

        // The leaf is whichever certificate holds the key's public half.
        let mut certificates: Vec<Vec<u8>> = keys.certs().iter().map(|c| c.as_der().to_vec()).collect();
        for (_, entry) in store.entries() {
            if let KeyStoreEntry::Certificate(certificate) = entry {
                certificates.push(certificate.as_der().to_vec());
            }
        }
        let leaf = certificates
            .iter()
            .find(|der| {
                Certificate::from_der(der).ok().is_some_and(|c| {
                    c.tbs_certificate.subject_public_key_info.subject_public_key.raw_bytes() == public.as_bytes()
                })
            })
            .cloned()
            .ok_or_else(|| report!("None of the .p12's certificates belongs to its private key"))?;
        let certificate = Certificate::from_der(&leaf)?;
        let issuer = attribute(&certificate.tbs_certificate.issuer, COMMON_NAME).unwrap_or_default();
        if !issuer.contains("Apple Worldwide Developer Relations") {
            bail!("This isn't an Apple developer certificate (it was issued by \"{issuer}\")");
        }
        if not_after(&certificate) < SystemTime::now() {
            bail!("This certificate expired on {}", iso(not_after(&certificate)));
        }

        let id = fingerprint(&leaf);
        let dir = identities_dir(&data_dir).join(&id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("certificate.der"), &leaf)?;
        std::fs::write(dir.join("key.der"), keys.key().as_der())?;
        tracing::info!("imported a signing certificate");
        Ok(json!({ "identity": Identity::load(&data_dir, &id)?.json() }))
    })())
}

/// Deletes an imported certificate and its key. Returns `{ok}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_identity_delete(data_dir: *const c_char, id: *const c_char) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let id = unsafe { text(id) }?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("bad certificate id");
        }
        std::fs::remove_dir_all(identities_dir(&data_dir).join(id))?;
        Ok(json!({}))
    })())
}

/// Imports a .mobileprovision (replacing one with the same UUID). Returns `{ok, profile}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_profile_import(
    data_dir: *const c_char,
    path: *const c_char,
    udid: *const c_char,
) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let udid = unsafe { optional(udid) }?;
        let profile = Profile::parse(std::fs::read(unsafe { text(path) }?)?)?;
        let uuid = profile.uuid();
        if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            bail!("The profile has no valid UUID");
        }
        let dir = profiles_dir(&data_dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{uuid}.mobileprovision")), &profile.data)?;
        Ok(json!({ "profile": profile.json(&Identity::all(&data_dir), udid.as_deref()) }))
    })())
}

/// Deletes an imported profile. Returns `{ok}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_profile_delete(data_dir: *const c_char, uuid: *const c_char) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let uuid = unsafe { text(uuid) }?;
        if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            bail!("bad profile UUID");
        }
        std::fs::remove_file(profiles_dir(&data_dir).join(format!("{uuid}.mobileprovision")))?;
        Ok(json!({}))
    })())
}

/// Signs the app or IPA at `app_path` with an imported certificate, choosing for the app and
/// each extension an imported profile that allows the certificate, lists `udid`, hasn't
/// expired and covers the bundle ID (an explicit App ID matching the app first, then the
/// narrowest wildcard; `profile_uuid`, when given, is used for the main app instead).
/// Returns `{ok, signedPath, bundleId, profile, expires, debuggable, strippedExtensions}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_sign_offline(
    data_dir: *const c_char,
    app_path: *const c_char,
    identity_id: *const c_char,
    profile_uuid: *const c_char,
    udid: *const c_char,
    strip_extensions: bool,
) -> *mut c_char {
    reply((|| {
        let data_dir = PathBuf::from(unsafe { text(data_dir) }?);
        let app_path = PathBuf::from(unsafe { text(app_path) }?);
        let identity = Identity::load(&data_dir, &unsafe { text(identity_id) }?)?;
        let chosen = unsafe { optional(profile_uuid) }?;
        let udid = unsafe { text(udid) }?;
        if not_after(&identity.certificate) < SystemTime::now() {
            bail!("The certificate expired on {}; import a new one", iso(not_after(&identity.certificate)));
        }

        // Profiles this certificate may sign with on this device.
        let usable: Vec<Profile> = Profile::all(&data_dir)
            .into_iter()
            .filter(|p| p.certificate_ids().contains(&identity.id))
            .filter(|p| !p.expired() && p.kind() != "appStore" && p.for_vision() && p.includes(Some(&udid)) == Some(true))
            .collect();
        if usable.is_empty() {
            bail!(
                "No imported profile can sign with this certificate on this headset. Import a development, ad hoc or enterprise profile that includes the certificate and this headset (UDID {udid})."
            );
        }

        let mut app = Application::new(app_path)?;
        let original = app.main_bundle_id()?;

        let main = match &chosen {
            Some(uuid) => usable
                .iter()
                .find(|p| &p.uuid() == uuid)
                .ok_or_else(|| report!("The chosen profile can't sign with this certificate on this headset"))?,
            None => best_profile(&usable, &original).ok_or_else(|| report!("No usable profile"))?,
        };
        let main_id = main.bundle_id_for(&original);
        if main_id != original {
            app.update_bundle_id(&original, &main_id)?;
        }
        app.bundle.write_info()?;
        for extension in app.bundle.app_extensions_mut() {
            extension.write_info()?;
        }

        // Every extension, PlugIns/ and ExtensionKit's Extensions/, needs a profile covering its
        // (renamed) ID. One no profile covers is removed when the app allows that, else it's an error.
        let mut extensions: Vec<(PathBuf, String)> = app
            .bundle
            .app_extensions()
            .iter()
            .filter_map(|e| Some((e.bundle_dir.clone(), e.bundle_identifier()?.to_string())))
            .collect();
        extensions.extend(rename_extensionkit(&app.bundle.bundle_dir, &original, &main_id)?);
        let mut by_bundle: Vec<(String, &Profile)> = Vec::new();
        let mut stripped = Vec::new();
        for (dir, id) in extensions {
            let profile = if main.covers(&id) { Some(main) } else { best_profile(&usable, &id).filter(|p| p.covers(&id)) };
            match profile {
                Some(profile) => by_bundle.push((id, profile)),
                None if strip_extensions => {
                    std::fs::remove_dir_all(&dir)?;
                    stripped.push(dir.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
                None => bail!(
                    "No imported profile covers the extension {id}. Import one for it, use a wildcard profile, or turn on removing extensions."
                ),
            }
        }

        let certificate_chain = chain(&identity.certificate, &main.parsed)?;
        let signer = RustCryptoCmsSigner::new(identity.key.clone(), identity.certificate.clone(), certificate_chain);
        let mut settings =
            BundleSigningSettings::new(main.team(), entitlements_for(main, &main_id), Some(&signer));
        settings.embedded_mobileprovision = Some(&main.data);
        for (id, profile) in &by_bundle {
            settings.embedded_mobileprovisions_by_bundle_id.insert(id.clone(), &profile.data);
            settings.entitlements_by_bundle_id.insert(id.clone(), entitlements_for(profile, id));
        }
        sign_bundle(&app.bundle.bundle_dir, &settings).context("Signing failed")?;
        tracing::info!("signed offline with profile {}", main.uuid());

        Ok(json!({
            "signedPath": app.bundle.bundle_dir.to_string_lossy(),
            "bundleId": main_id,
            "profile": main.uuid(),
            "profileName": main.name(),
            "expires": main.expires().map(iso),
            "debuggable": main.get_task_allow(),
            "strippedExtensions": stripped,
        }))
    })())
}

/// ExtensionKit extensions (`Extensions/*.appex`) and the extension points the app declares
/// for them (`Extensions/*.appexpt`). An app's own extension point is named after its bundle
/// ID (UTM: `com.utmapp.UTM.qemu-helper`, which its code derives from the running app's ID),
/// so a renamed app needs the point, the extension's ID and the extension's reference to the
/// point renamed alike, or installd refuses the extension. Returns each extension's folder and
/// (renamed) ID.
fn rename_extensionkit(app: &Path, original: &str, renamed: &str) -> Result<Vec<(PathBuf, String)>, Report> {
    let rename = |id: &str| match id.strip_prefix(original) {
        Some(rest) if renamed != original && rest.starts_with('.') => format!("{renamed}{rest}"),
        _ => id.to_string(),
    };
    let Ok(entries) = std::fs::read_dir(app.join("Extensions")) else { return Ok(Vec::new()) };
    let mut extensions = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("appex") => {
                let info_path = path.join("Info.plist");
                let Ok(mut info) = plist::from_file::<_, Dictionary>(&info_path) else { continue };
                let Some(id) = info.get("CFBundleIdentifier").and_then(Plist::as_string).map(rename) else { continue };
                info.insert("CFBundleIdentifier".into(), Plist::String(id.clone()));
                if let Some(Plist::Dictionary(attributes)) = info.get_mut("EXAppExtensionAttributes") {
                    if let Some(point) = attributes.get("EXExtensionPointIdentifier").and_then(Plist::as_string).map(rename) {
                        attributes.insert("EXExtensionPointIdentifier".into(), Plist::String(point));
                    }
                }
                plist::to_file_binary(&info_path, &info)?;
                extensions.push((path, id));
            }
            Some("appexpt") if renamed != original => {
                let Ok(points) = plist::from_file::<_, Dictionary>(&path) else { continue };
                let points: Dictionary = points.into_iter().map(|(key, value)| (rename(&key), value)).collect();
                plist::to_file_xml(&path, &points)?;
            }
            _ => {}
        }
    }
    extensions.sort();
    Ok(extensions)
}

/// The profile to sign `bundle_id` with: an explicit App ID equal to it, then the wildcard
/// with the longest prefix that covers it, then any explicit one (which renames the app),
/// preferring debuggable profiles (JIT needs get-task-allow) and later expiry.
fn best_profile<'a>(profiles: &'a [Profile], bundle_id: &str) -> Option<&'a Profile> {
    profiles.iter().max_by_key(|p| {
        let pattern = p.app_id_pattern();
        let rank = if pattern == bundle_id {
            3_000
        } else if p.covers(bundle_id) {
            1_000 + pattern.len()
        } else {
            0
        };
        (rank, p.get_task_allow(), p.expires())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_bundle_ids() {
        for (pattern, original, expected, covered) in [
            ("*", "com.utmapp.UTM", "com.utmapp.UTM", true),
            ("com.utmapp.*", "com.utmapp.UTM", "com.utmapp.UTM", true),
            ("com.example.*", "com.utmapp.UTM", "com.example.com.utmapp.UTM", false),
            ("com.example.vm", "com.utmapp.UTM", "com.example.vm", false),
            ("com.utmapp.UTM", "com.utmapp.UTM", "com.utmapp.UTM", true),
        ] {
            assert_eq!(bundle_id_for(pattern, original), expected, "{pattern}");
            assert_eq!(covers(pattern, original), covered, "{pattern}");
        }
    }

    #[test]
    fn bundled_chain_parses() {
        for der in EXTRA_CHAIN {
            Certificate::from_der(der).unwrap();
        }
    }
}

#[cfg(test)]
mod local {
    use super::*;

    /// `APPARITION_PROFILES=<dir> cargo test --lib profiles_parse -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn profiles_parse() {
        let dir = std::env::var("APPARITION_PROFILES").unwrap();
        for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            if entry.path().extension().is_none_or(|e| e != "mobileprovision") {
                continue;
            }
            let profile = Profile::parse(std::fs::read(entry.path()).unwrap()).unwrap();
            let mut summary = profile.json(&[], None);
            summary.as_object_mut().unwrap().remove("uuid");
            println!("{summary} certs={} ent={:?}", profile.certificate_ids().len(),
                     entitlements_for(&profile, "com.utmapp.UTM").keys().collect::<Vec<_>>());
        }
    }

    fn call(reply: *mut c_char) -> Value {
        let text = unsafe { std::ffi::CStr::from_ptr(reply) }.to_string_lossy().to_string();
        unsafe { crate::apparition_string_free(reply) };
        serde_json::from_str(&text).unwrap()
    }

    /// `APPARITION_OFFLINE=<dir with id.p12 (password testpw), fake.mobileprovision, app.ipa>`
    /// `cargo test --lib offline_sign -- --ignored --nocapture`, with a self-made certificate.
    #[test]
    #[ignore]
    fn offline_sign() {
        use std::ffi::CString;
        let dir = PathBuf::from(std::env::var("APPARITION_OFFLINE").unwrap());
        let data = dir.join("data");
        let _ = std::fs::remove_dir_all(&data);
        let c = |s: &str| CString::new(s).unwrap();
        let data_c = c(data.to_str().unwrap());
        let bad = call(unsafe { apparition_identity_import(data_c.as_ptr(), c(dir.join("id.p12").to_str().unwrap()).as_ptr(), c("wrong").as_ptr()) });
        println!("wrong password: {bad}");
        assert_eq!(bad["ok"], false);
        let imported = call(unsafe { apparition_identity_import(data_c.as_ptr(), c(dir.join("id.p12").to_str().unwrap()).as_ptr(), c("testpw").as_ptr()) });
        println!("identity: {imported}");
        let id = imported["identity"]["id"].as_str().unwrap().to_string();
        let profile = call(unsafe { apparition_profile_import(data_c.as_ptr(), c(dir.join("fake.mobileprovision").to_str().unwrap()).as_ptr(), c("FAKE-UDID-0001").as_ptr()) });
        println!("profile: {profile}");
        let refused = call(unsafe { apparition_sign_offline(data_c.as_ptr(), c(dir.join("app.ipa").to_str().unwrap()).as_ptr(), c(&id).as_ptr(), std::ptr::null(), c("OTHER-UDID").as_ptr(), false) });
        println!("other device: {refused}");
        assert_eq!(refused["ok"], false);
        let started = std::time::Instant::now();
        let signed = call(unsafe { apparition_sign_offline(data_c.as_ptr(), c(dir.join("app.ipa").to_str().unwrap()).as_ptr(), c(&id).as_ptr(), std::ptr::null(), c("FAKE-UDID-0001").as_ptr(), true) });
        println!("signed in {:?}: {signed}", started.elapsed());
        assert_eq!(signed["ok"], true);
        let kept = call(unsafe { apparition_sign_offline(data_c.as_ptr(), c(dir.join("app.ipa").to_str().unwrap()).as_ptr(), c(&id).as_ptr(), std::ptr::null(), c("FAKE-UDID-0001").as_ptr(), false) });
        println!("with extensions: {kept}");
        assert_eq!(kept["ok"], true);
        assert_eq!(kept["strippedExtensions"].as_array().unwrap().len(), 0);

        // A prefix wildcard renames the app, its ExtensionKit helper and the extension point.
        call(unsafe { apparition_profile_delete(data_c.as_ptr(), c("11111111-2222-3333-4444-555555555555").as_ptr()) });
        call(unsafe { apparition_profile_import(data_c.as_ptr(), c(dir.join("fake-prefix.mobileprovision").to_str().unwrap()).as_ptr(), std::ptr::null()) });
        let renamed = call(unsafe { apparition_sign_offline(data_c.as_ptr(), c(dir.join("app.ipa").to_str().unwrap()).as_ptr(), c(&id).as_ptr(), std::ptr::null(), c("FAKE-UDID-0001").as_ptr(), false) });
        println!("renamed: {renamed}");
        assert_eq!(renamed["ok"], true);
        let app = PathBuf::from(renamed["signedPath"].as_str().unwrap());
        let main = renamed["bundleId"].as_str().unwrap();
        if let Ok(entries) = std::fs::read_dir(app.join("Extensions")) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let text = if path.extension().is_some_and(|e| e == "appex") {
                    format!("{:?}", plist::from_file::<_, Dictionary>(path.join("Info.plist")).unwrap())
                } else {
                    std::fs::read_to_string(&path).unwrap_or_default()
                };
                println!("{}: {}", path.display(), text.len());
                assert!(!text.contains("\"com.utmapp.UTM.") && !text.contains(">com.utmapp.UTM."), "{text}");
                assert!(text.contains(main), "{text}");
            }
        }
    }
}
