use std::{future::Future, path::Path, pin::Pin, sync::Arc};

use crate::{
    ActionDescriptor, ActionRequest, ExecutionObserver, InvocationId, PluginDescriptor, PluginId,
    TextActionUiPort,
};

pub type ScriptFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptNetworkRule {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub methods: Vec<String>,
    pub path_prefixes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ScriptPackageDefinition {
    pub plugin: PluginDescriptor,
    pub version: String,
    pub entry: String,
    pub actions: Vec<ActionDescriptor>,
    pub network_rules: Vec<ScriptNetworkRule>,
}

#[derive(Clone, Debug)]
pub struct ScriptHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub enum ScriptHostCall {
    Ai {
        system: Option<String>,
        input: String,
    },
    Http(ScriptHttpRequest),
    StorageGet {
        key: String,
    },
    StorageSet {
        key: String,
        value: String,
    },
    StorageDelete {
        key: String,
    },
}

pub trait ScriptInvocation: Send + Sync {
    fn id(&self) -> &InvocationId;
    fn plugin_id(&self) -> &PluginId;
    fn context_json(&self) -> String;
    fn is_cancelled(&self) -> bool;
    fn output_len(&self) -> usize;
    fn append(&self, text: &str) -> Result<(), String>;
    fn dispatch(
        &self,
        call: ScriptHostCall,
        completion: Box<dyn FnOnce(Result<String, String>) + Send>,
    );
    fn show_result(&self) -> Result<(), String>;
    fn log(&self, level: &str, category: &str);
    fn finish<'a>(&'a self, result: Result<String, String>) -> ScriptFuture<'a, String>;
}

pub trait ScriptInvocationHost: Send + Sync {
    fn begin<'a>(
        &'a self,
        action: &'a ActionDescriptor,
        request: ActionRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> ScriptFuture<'a, Arc<dyn ScriptInvocation>>;
    fn cancel(&self, invocation_id: &InvocationId) -> Result<(), String>;
}

pub trait ScriptPluginLifecycle: Send + Sync {
    fn stop(&self);
}

pub struct ScriptActivation {
    pub handlers: Vec<Arc<dyn crate::ActionHandler>>,
    pub controllers: Vec<Arc<dyn TextActionUiPort>>,
    pub lifecycle: Arc<dyn ScriptPluginLifecycle>,
}

pub trait ScriptPackageFactory: Send + Sync {
    fn inspect(&self, root: &Path) -> Result<ScriptPackageDefinition, String>;
    fn activate(
        &self,
        root: &Path,
        definition: &ScriptPackageDefinition,
        host: Arc<dyn ScriptInvocationHost>,
        settings: Arc<dyn crate::SettingsUiPort>,
    ) -> Result<ScriptActivation, String>;
    fn shutdown(&self);
}
