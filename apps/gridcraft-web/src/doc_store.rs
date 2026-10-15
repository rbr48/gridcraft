//! Active workbook persistence in browser IndexedDB.
//!
//! Stores the active document name and file bytes so workbooks survive browser refresh,
//! tab close, and page reloads seamlessly.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{IdbDatabase, IdbRequest, IdbTransactionMode};

const DB_NAME: &str = "gridcraft_storage";
const STORE: &str = "active_workbook";
const KEY_NAME: &str = "name";
const KEY_BYTES: &str = "bytes";

fn js(e: JsValue) -> String {
    format!("{e:?}")
}

async fn done(req: &IdbRequest) -> Result<JsValue, String> {
    let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
        let ok_req = req.clone();
        let ok = Closure::once_into_js(move || {
            let _ = resolve.call1(&JsValue::NULL, &ok_req.result().unwrap_or(JsValue::UNDEFINED));
        });
        let err = Closure::once_into_js(move || {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("IndexedDB request failed"));
        });
        req.set_onsuccess(Some(ok.unchecked_ref()));
        req.set_onerror(Some(err.unchecked_ref()));
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.map_err(js)
}

async fn open() -> Result<IdbDatabase, String> {
    let window = web_sys::window().ok_or("no window")?;
    let factory = window.indexed_db().map_err(js)?.ok_or("IndexedDB is unavailable")?;
    let req = factory.open_with_u32(DB_NAME, 1).map_err(js)?;
    let upgrade_req = req.clone();
    let upgrade = Closure::<dyn FnMut()>::new(move || {
        if let Ok(db) = upgrade_req.result().and_then(|r| r.dyn_into::<IdbDatabase>().map_err(JsValue::from)) {
            if !db.object_store_names().contains(STORE) {
                if let Err(e) = db.create_object_store(STORE) {
                    log::error!("couldn't create the workbook object store: {e:?}");
                }
            }
        }
    });
    req.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let db = done(&req).await;
    req.set_onupgradeneeded(None);
    drop(upgrade);
    db?.dyn_into::<IdbDatabase>().map_err(js)
}

pub async fn store_async(name: String, bytes: Vec<u8>) -> Result<(), String> {
    let db = open().await?;
    let tx = db.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite).map_err(js)?;
    let store = tx.object_store(STORE).map_err(js)?;
    let req1 = store.put_with_key(&JsValue::from_str(&name), &JsValue::from_str(KEY_NAME)).map_err(js)?;
    done(&req1).await?;
    let uint8 = js_sys::Uint8Array::from(bytes.as_slice());
    let req2 = store.put_with_key(&uint8, &JsValue::from_str(KEY_BYTES)).map_err(js)?;
    done(&req2).await?;
    Ok(())
}

pub fn store(name: &str, bytes: &[u8]) {
    let name = name.to_string();
    let bytes = bytes.to_vec();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = store_async(name.clone(), bytes).await {
            log::warn!("couldn't persist workbook {name} to IndexedDB: {e}");
        }
    });
}

pub async fn load_last() -> Option<(String, Vec<u8>)> {
    let db = open().await.ok()?;
    let tx = db.transaction_with_str(STORE).ok()?;
    let store = tx.object_store(STORE).ok()?;
    let name_req = store.get(&JsValue::from_str(KEY_NAME)).ok()?;
    let name_val = done(&name_req).await.ok()?;
    let name = name_val.as_string()?;

    let bytes_req = store.get(&JsValue::from_str(KEY_BYTES)).ok()?;
    let bytes_val = done(&bytes_req).await.ok()?;
    let uint8 = bytes_val.dyn_into::<js_sys::Uint8Array>().ok()?;
    let bytes = uint8.to_vec();
    if bytes.is_empty() {
        return None;
    }
    Some((name, bytes))
}
