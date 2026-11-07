//! Small JS boundary; all bytecode, mapping and naming work lives in sporeflower.
use sporeflower::session::{Session, SessionMappings, SessionOptions};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Default)]
pub struct SporeflowerSession {
    inner: Session,
}

#[wasm_bindgen]
impl SporeflowerSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    #[wasm_bindgen(js_name = addClass)]
    pub fn add_class(&mut self, path: &str, bytes: &[u8]) -> Result<String, JsValue> {
        self.inner.add_class(path, bytes).map_err(js_error)
    }

    #[wasm_bindgen(js_name = setManifest)]
    pub fn set_manifest(&mut self, text: &str) {
        self.inner.set_manifest(text);
    }

    #[wasm_bindgen(js_name = setMappings)]
    pub fn set_mappings(&mut self, json: &str) -> Result<(), JsValue> {
        let mappings: SessionMappings = serde_json::from_str(json).map_err(js_error)?;
        self.inner.set_mappings(mappings).map_err(js_error)
    }

    pub fn prepare(&mut self, options_json: &str) -> Result<String, JsValue> {
        let options: SessionOptions = serde_json::from_str(options_json).map_err(js_error)?;
        let metadata = self.inner.prepare(options).map_err(js_error)?;
        serde_json::to_string(metadata).map_err(js_error)
    }

    pub fn decompile(&self, original_name: &str) -> Result<String, JsValue> {
        let result = self.inner.decompile(original_name).map_err(js_error)?;
        serde_json::to_string(&result).map_err(js_error)
    }

    #[wasm_bindgen(js_name = mappingTiny)]
    pub fn mapping_tiny(&self) -> Result<String, JsValue> {
        self.inner.mapping_tiny().map_err(js_error)
    }

    #[wasm_bindgen(js_name = domainSources)]
    pub fn domain_sources(&self) -> Result<String, JsValue> {
        let sources = self.inner.domain_sources().map_err(js_error)?;
        serde_json::to_string(&sources).map_err(js_error)
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsError::new(&format!("{error:#}")).into()
}
