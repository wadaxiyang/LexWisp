use std::{
    net::{IpAddr, Ipv6Addr, ToSocketAddrs as _},
    sync::Arc,
};

use futures_util::StreamExt as _;
use lexwisp_core::{
    ActionDescriptor, ActionRequest, Capability, ExecutionObserver, HostUiCommand, InvocationId,
    PluginId, ScriptFuture, ScriptHostCall, ScriptHttpRequest, ScriptInvocation,
    ScriptInvocationHost, ScriptNetworkRule,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{ActionRegistry, CapabilityAuthority, HostTaskPort, InvocationSupervisor};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const HTTP_BODY_LIMIT: usize = 2 * 1024 * 1024;

pub(crate) struct BoundScriptHost {
    plugin_id: PluginId,
    package_hash: String,
    generation: u64,
    network_rules: Vec<ScriptNetworkRule>,
    actions: ActionRegistry,
    capabilities: CapabilityAuthority,
    supervisor: Arc<InvocationSupervisor>,
    tasks: HostTaskPort,
    content: lexwisp_storage::ContentStore,
    http: Arc<reqwest::Client>,
    ui_commands: async_channel::Sender<HostUiCommand>,
}

impl BoundScriptHost {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        plugin_id: PluginId,
        package_hash: String,
        generation: u64,
        network_rules: Vec<ScriptNetworkRule>,
        actions: ActionRegistry,
        capabilities: CapabilityAuthority,
        supervisor: Arc<InvocationSupervisor>,
        tasks: HostTaskPort,
        content: lexwisp_storage::ContentStore,
        http: Arc<reqwest::Client>,
        ui_commands: async_channel::Sender<HostUiCommand>,
    ) -> Self {
        Self {
            plugin_id,
            package_hash,
            generation,
            network_rules,
            actions,
            capabilities,
            supervisor,
            tasks,
            content,
            http,
            ui_commands,
        }
    }
}

impl ScriptInvocationHost for BoundScriptHost {
    fn begin<'a>(
        &'a self,
        action: &'a ActionDescriptor,
        request: ActionRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> ScriptFuture<'a, Arc<dyn ScriptInvocation>> {
        Box::pin(async move {
            if action.plugin_id() != &self.plugin_id
                || !self.actions.contains(&action.qualified_id())
                || self.actions.generation() != self.generation
            {
                return Err("script action identity or generation is stale".into());
            }
            let invocation_id = InvocationId::new();
            let cancellation = self.supervisor.begin_script(
                invocation_id.clone(),
                action,
                self.generation,
                &request,
                observer,
            )?;
            let invocation: Arc<dyn ScriptInvocation> = Arc::new(BoundScriptInvocation {
                invocation_id,
                plugin_id: self.plugin_id.clone(),
                package_hash: self.package_hash.clone(),
                generation: self.generation,
                context_json: context_json(&request),
                network_rules: self.network_rules.clone(),
                capabilities: self.capabilities.clone(),
                supervisor: self.supervisor.clone(),
                tasks: self.tasks.clone(),
                content: self.content.clone(),
                http: self.http.clone(),
                ui_commands: self.ui_commands.clone(),
                cancellation,
            });
            Ok(invocation)
        })
    }

    fn cancel(&self, invocation_id: &InvocationId) -> Result<(), String> {
        self.supervisor
            .cancel(invocation_id)
            .map_err(|error| error.to_string())
    }
}

struct BoundScriptInvocation {
    invocation_id: InvocationId,
    plugin_id: PluginId,
    package_hash: String,
    generation: u64,
    context_json: String,
    network_rules: Vec<ScriptNetworkRule>,
    capabilities: CapabilityAuthority,
    supervisor: Arc<InvocationSupervisor>,
    tasks: HostTaskPort,
    content: lexwisp_storage::ContentStore,
    http: Arc<reqwest::Client>,
    ui_commands: async_channel::Sender<HostUiCommand>,
    cancellation: CancellationToken,
}

impl BoundScriptInvocation {
    fn authorize(&self, capability: Capability) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("script invocation was cancelled".into());
        }
        self.capabilities
            .is_bound_grant_valid(
                &self.plugin_id,
                capability,
                &self.package_hash,
                self.generation,
            )
            .then_some(())
            .ok_or_else(|| {
                format!(
                    "capability '{}' is not granted for this plugin generation",
                    capability.manifest_name()
                )
            })
    }

    async fn call(self: Arc<Self>, call: ScriptHostCall) -> Result<String, String> {
        match call {
            ScriptHostCall::Ai { system, input } => {
                self.authorize(Capability::AiInvoke)?;
                self.supervisor
                    .script_ai(system, input, &self.cancellation)
                    .await
            }
            ScriptHostCall::Http(request) => {
                self.authorize(Capability::NetworkRequest)?;
                self.http_request(request).await
            }
            ScriptHostCall::StorageGet { key } => {
                self.authorize(Capability::StorageRead)?;
                let content = self.content.clone();
                let plugin = self.plugin_id.to_string();
                tokio::task::spawn_blocking(move || content.plugin_kv_get(&plugin, &key))
                    .await
                    .map_err(|error| error.to_string())?
                    .map(|value| value.unwrap_or_else(|| "null".into()))
                    .map_err(|error| error.to_string())
            }
            ScriptHostCall::StorageSet { key, value } => {
                self.authorize(Capability::StorageWrite)?;
                let content = self.content.clone();
                let plugin = self.plugin_id.to_string();
                tokio::task::spawn_blocking(move || content.plugin_kv_set(&plugin, &key, &value))
                    .await
                    .map_err(|error| error.to_string())?
                    .map(|()| "true".into())
                    .map_err(|error| error.to_string())
            }
            ScriptHostCall::StorageDelete { key } => {
                self.authorize(Capability::StorageWrite)?;
                let content = self.content.clone();
                let plugin = self.plugin_id.to_string();
                tokio::task::spawn_blocking(move || content.plugin_kv_delete(&plugin, &key))
                    .await
                    .map_err(|error| error.to_string())?
                    .map(|()| "true".into())
                    .map_err(|error| error.to_string())
            }
        }
    }

    async fn http_request(&self, request: ScriptHttpRequest) -> Result<String, String> {
        let url = Url::parse(&request.url).map_err(|error| format!("invalid HTTP URL: {error}"))?;
        let method = request.method.to_ascii_uppercase();
        authorize_url(&self.network_rules, &url, &method)?;
        if let Some(host) = url.host_str()
            && let Ok(ip) = host.parse::<IpAddr>()
            && is_private_ip(ip)
        {
            return Err("local and private network addresses are not allowed".into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "HTTP URL has no host".to_string())?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| "HTTP URL has no known port".to_string())?;
        let resolved = tokio::task::spawn_blocking(move || {
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>())
        })
        .await
        .map_err(|error| format!("DNS task failed: {error}"))?
        .map_err(|error| format!("DNS resolution failed: {error}"))?;
        if resolved.is_empty() || resolved.iter().any(|address| is_private_ip(address.ip())) {
            return Err("resolved HTTP address is local, private, or unavailable".into());
        }
        if request.headers.iter().any(|(name, _)| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization" | "proxy-authorization" | "cookie"
            )
        }) {
            return Err("authentication and cookie headers are not available to scripts".into());
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|error| format!("invalid HTTP method: {error}"))?;
        let mut builder = self.http.request(method, url.clone());
        for (name, value) in request.headers {
            builder = builder.header(&name, &value);
        }
        if let Some(body) = request.body {
            if body.len() > HTTP_BODY_LIMIT {
                return Err("HTTP request body exceeds 2 MiB".into());
            }
            builder = builder.body(body);
        }
        let response = tokio::select! {
            _ = self.cancellation.cancelled() => return Err("script invocation was cancelled".into()),
            response = builder.send() => response.map_err(|error| format!("HTTP request failed: {error}"))?,
        };
        if response.status().is_redirection() {
            return Err("script HTTP redirects are disabled".into());
        }
        if let Some(address) = response.remote_addr()
            && is_private_ip(address.ip())
        {
            return Err("resolved HTTP address is local or private".into());
        }
        let status = response.status();
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = tokio::select! {
            _ = self.cancellation.cancelled() => return Err("script invocation was cancelled".into()),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk.map_err(|error| format!("HTTP response failed: {error}"))?;
            if body.len().saturating_add(chunk.len()) > HTTP_BODY_LIMIT {
                return Err("HTTP response exceeds 2 MiB".into());
            }
            body.extend_from_slice(&chunk);
        }
        let text =
            String::from_utf8(body).map_err(|_| "HTTP response is not UTF-8 text".to_string())?;
        Ok(serde_json::json!({ "status": status.as_u16(), "body": text }).to_string())
    }
}

impl ScriptInvocation for BoundScriptInvocation {
    fn id(&self) -> &InvocationId {
        &self.invocation_id
    }
    fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }
    fn context_json(&self) -> String {
        self.context_json.clone()
    }
    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    fn output_len(&self) -> usize {
        self.supervisor.script_output_len(&self.invocation_id)
    }

    fn append(&self, text: &str) -> Result<(), String> {
        self.authorize_live()?;
        if self.output_len().saturating_add(text.len()) > OUTPUT_LIMIT {
            return Err("script output exceeds 2 MiB".into());
        }
        self.supervisor
            .append_script(&self.invocation_id, self.generation, text)
    }

    fn dispatch(
        &self,
        call: ScriptHostCall,
        completion: Box<dyn FnOnce(Result<String, String>) + Send>,
    ) {
        let this = Arc::new(self.clone_for_task());
        let scope = self.tasks.scope(lexwisp_core::TaskOwner::Invocation(
            self.invocation_id.to_string(),
        ));
        scope.spawn(async move {
            completion(this.call(call).await);
        });
    }

    fn show_result(&self) -> Result<(), String> {
        self.authorize_live()?;
        self.ui_commands
            .try_send(HostUiCommand::ShowMainShell)
            .map_err(|error| error.to_string())
    }

    fn log(&self, level: &str, category: &str) {
        let safe_level = if matches!(level, "debug" | "info" | "warn" | "error") {
            level
        } else {
            "info"
        };
        let safe_category = category
            .chars()
            .filter(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
            .take(64)
            .collect::<String>();
        eprintln!(
            "script plugin={} invocation={} level={} category={}",
            self.plugin_id, self.invocation_id, safe_level, safe_category
        );
    }

    fn finish<'a>(&'a self, result: Result<String, String>) -> ScriptFuture<'a, String> {
        Box::pin(
            self.supervisor
                .finish_script(&self.invocation_id, self.generation, result),
        )
    }
}

impl BoundScriptInvocation {
    fn authorize_live(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("script invocation was cancelled".into());
        }
        if self
            .capabilities
            .is_binding_valid(&self.plugin_id, &self.package_hash, self.generation)
        {
            Ok(())
        } else {
            Err("script plugin generation is no longer authorized".into())
        }
    }

    fn clone_for_task(&self) -> Self {
        Self {
            invocation_id: self.invocation_id.clone(),
            plugin_id: self.plugin_id.clone(),
            package_hash: self.package_hash.clone(),
            generation: self.generation,
            context_json: self.context_json.clone(),
            network_rules: self.network_rules.clone(),
            capabilities: self.capabilities.clone(),
            supervisor: self.supervisor.clone(),
            tasks: self.tasks.clone(),
            content: self.content.clone(),
            http: self.http.clone(),
            ui_commands: self.ui_commands.clone(),
            cancellation: self.cancellation.clone(),
        }
    }
}

fn context_json(request: &ActionRequest) -> String {
    serde_json::json!({
        "source": format!("{:?}", request.source).to_ascii_lowercase(),
        "token": request.context_token.as_ref().map(ToString::to_string),
    })
    .to_string()
}

fn authorize_url(rules: &[ScriptNetworkRule], url: &Url, method: &str) -> Result<(), String> {
    let host = url.host_str().ok_or("HTTP URL has no host")?;
    let port = url.port_or_known_default();
    let allowed = rules.iter().any(|rule| {
        rule.scheme == url.scheme()
            && rule.host.eq_ignore_ascii_case(host)
            && rule
                .port
                .or_else(|| (rule.scheme == "https").then_some(443))
                .or_else(|| (rule.scheme == "http").then_some(80))
                == port
            && rule.methods.iter().any(|allowed| allowed == method)
            && (rule.path_prefixes.is_empty()
                || rule
                    .path_prefixes
                    .iter()
                    .any(|prefix| url.path().starts_with(prefix)))
    });
    allowed
        .then_some(())
        .ok_or_else(|| "HTTP target is outside the approved network rules".into())
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || is_unique_local(ip)
                || is_link_local_v6(ip)
        }
    }
}

fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}
fn is_link_local_v6(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}
