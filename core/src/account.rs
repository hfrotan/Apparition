//! The account's development certificates and App IDs, so the user can see what uses up a
//! free account's slots and get them back.
//!
//! A free account has room for two development certificates and ten App IDs a week. Every
//! installation of a sideloader makes its own certificate (its private key never leaves the
//! device that made it), so a reinstall, a second device or another tool (Xcode, SideStore)
//! can use both up, and signing then fails with Apple's error 7460. Nothing here revokes or
//! deletes on its own: the app asks the user first.

use std::ffi::c_char;

use isideload::dev::{app_ids::AppIdsApi, certificates::CertificatesApi};
use rootcause::prelude::*;
use serde_json::{Value, json};

use crate::{SESSION, reply, runtime, text};

/// The machine name this app's certificates are made under (see `apparition_login`).
pub const MACHINE_NAME: &str = "Apparition";

fn date(value: &Option<plist::Date>) -> Value {
    value.as_ref().map(|date| Value::String(date.to_xml_format())).unwrap_or(Value::Null)
}

/// The team's iOS development certificates (the kind signing uses):
/// `{ok, certificates: [{name, serialNumber, machineName, status, expires, madeByApparition}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn apparition_list_certificates() -> *mut c_char {
    reply((|| {
        let mut guard = SESSION.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| report!("Not signed in"))?;
        let team = session.team.clone();
        runtime().block_on(async {
            let certs = session.sideloader.get_dev_session().list_ios_certs(&team).await?;
            let list: Vec<Value> = certs
                .iter()
                .map(|cert| {
                    json!({
                        "name": cert.name,
                        "serialNumber": cert.serial_number,
                        "machineName": cert.machine_name,
                        "status": cert.status,
                        "expires": date(&cert.expiration_date),
                        "madeByApparition": cert.machine_name.as_deref() == Some(MACHINE_NAME),
                    })
                })
                .collect();
            Ok(json!({ "certificates": list }))
        })
    })())
}

/// Revokes the development certificate with `serial_number`. Apps signed with it stop
/// launching until they are signed again. Returns `{ok}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_revoke_certificate(serial_number: *const c_char) -> *mut c_char {
    reply((|| {
        let serial = unsafe { text(serial_number) }?;
        let mut guard = SESSION.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| report!("Not signed in"))?;
        let team = session.team.clone();
        runtime().block_on(async {
            session.sideloader.get_dev_session().revoke_development_cert(&team, &serial, None).await?;
            Ok(json!({}))
        })
    })())
}

/// The team's App IDs and how many more it may make:
/// `{ok, appIds: [{appIdId, identifier, name, expires}], maxQuantity, availableQuantity}`.
#[unsafe(no_mangle)]
pub extern "C" fn apparition_list_app_ids() -> *mut c_char {
    reply((|| {
        let mut guard = SESSION.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| report!("Not signed in"))?;
        let team = session.team.clone();
        runtime().block_on(async {
            let response = session.sideloader.get_dev_session().list_app_ids(&team, None).await?;
            let list: Vec<Value> = response
                .app_ids
                .iter()
                .map(|id| {
                    json!({
                        "appIdId": id.app_id_id,
                        "identifier": id.identifier,
                        "name": id.name,
                        "expires": date(&id.expiration_date),
                    })
                })
                .collect();
            Ok(json!({
                "appIds": list,
                "maxQuantity": response.max_quantity,
                "availableQuantity": response.available_quantity,
            }))
        })
    })())
}

/// Deletes an App ID (by its `appIdId`). On a free account Apple may refuse until the ID
/// expires, and a deleted ID may still count against the week's ten. Returns `{ok}`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apparition_delete_app_id(app_id_id: *const c_char) -> *mut c_char {
    reply((|| {
        let id = unsafe { text(app_id_id) }?;
        let mut guard = SESSION.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| report!("Not signed in"))?;
        let team = session.team.clone();
        runtime().block_on(async {
            session.sideloader.get_dev_session().delete_app_id(&team, &id, None).await?;
            Ok(json!({}))
        })
    })())
}
