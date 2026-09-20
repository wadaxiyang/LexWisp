use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use lexwisp_core::{
    ActionDescriptor, ActionId, ActionInputSource, ActionOutputPolicy, ActionParameter, Capability,
    DismissPolicy, ParameterKind, PluginDescriptor, PluginId, ScriptActionDefinition,
    ScriptNetworkRule, ScriptPackageDefinition,
};
use semver::{Version, VersionReq};
use serde::Deserialize;

const MAX_MODULE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MODULES: usize = 256;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    plugin: ManifestPlugin,
    capabilities: ManifestCapabilities,
    actions: Vec<ManifestAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPlugin {
    id: String,
    name: String,
    version: String,
    kind: String,
    host_api: String,
    entry: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCapabilities {
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    optional: Vec<String>,
    #[serde(default)]
    network: Vec<ManifestNetworkRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestNetworkRule {
    scheme: String,
    host: String,
    port: Option<u16>,
    methods: Vec<String>,
    #[serde(default)]
    path_prefixes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestAction {
    id: String,
    name: String,
    handler: String,
    input_kind: String,
    allowed_sources: Vec<String>,
    dismiss_policy: String,
    #[serde(default)]
    parameters: Vec<ManifestParameter>,
    output: ManifestOutput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestParameter {
    key: String,
    label: String,
    kind: String,
    required: bool,
    default: Option<toml::Value>,
    #[serde(default)]
    choices: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestOutput {
    format: String,
    allow_copy: bool,
    allow_favorite: bool,
    allow_replace: bool,
}

pub(crate) fn inspect(root: &Path) -> Result<ScriptPackageDefinition, String> {
    let manifest_path = root.join("manifest.toml");
    let manifest_source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("could not read script manifest: {error}"))?;
    let manifest: Manifest = toml::from_str(&manifest_source)
        .map_err(|error| format!("invalid script manifest: {error}"))?;
    if manifest.schema_version != 1 || manifest.plugin.kind != "script" {
        return Err("script packages require schema_version = 1 and kind = 'script'".into());
    }
    let host_api = VersionReq::parse(&manifest.plugin.host_api)
        .map_err(|error| format!("invalid Host API requirement: {error}"))?;
    if !host_api.matches(&Version::new(1, 0, 0)) {
        return Err("script package Host API must include 1.0".into());
    }
    let version = Version::parse(&manifest.plugin.version)
        .map_err(|error| format!("invalid plugin version: {error}"))?;
    let entry = validate_relative_js_path(&manifest.plugin.entry)?;
    let entry_path = root.join(&entry);
    validate_module(&entry_path)?;

    let mut capabilities = Vec::new();
    for name in manifest
        .capabilities
        .required
        .iter()
        .chain(&manifest.capabilities.optional)
    {
        let capability = Capability::parse_manifest(name)?;
        if !capabilities.contains(&capability) {
            capabilities.push(capability);
        }
    }
    if !manifest.capabilities.network.is_empty()
        && !capabilities.contains(&Capability::NetworkRequest)
    {
        return Err("network rules require the network.request capability".into());
    }
    let network_rules = manifest
        .capabilities
        .network
        .into_iter()
        .map(validate_network_rule)
        .collect::<Result<Vec<_>, _>>()?;
    let plugin_id = PluginId::parse(manifest.plugin.id).map_err(|error| error.to_string())?;
    let plugin = PluginDescriptor::new(plugin_id.clone(), manifest.plugin.name, capabilities);
    let mut actions = Vec::new();
    for action in manifest.actions {
        if action.input_kind != "text" || action.output.format != "text" {
            return Err("script actions currently support text input and output only".into());
        }
        if !valid_export_name(&action.handler) {
            return Err(format!("invalid exported handler '{}'", action.handler));
        }
        let sources = action
            .allowed_sources
            .into_iter()
            .map(|source| match source.as_str() {
                "selection" => Ok(ActionInputSource::Selection),
                "manual" => Ok(ActionInputSource::Manual),
                "clipboard" => Ok(ActionInputSource::Clipboard),
                _ => Err(format!("unknown action source '{source}'")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dismiss_policy = match action.dismiss_policy.as_str() {
            "cancel" => DismissPolicy::Cancel,
            "continue" => DismissPolicy::Continue,
            value => return Err(format!("unknown dismiss policy '{value}'")),
        };
        let parameters = action
            .parameters
            .into_iter()
            .map(parse_parameter)
            .collect::<Result<Vec<_>, _>>()?;
        actions.push(ActionDescriptor::script(
            plugin_id.clone(),
            ActionId::parse(action.id).map_err(|error| error.to_string())?,
            action.name,
            ScriptActionDefinition {
                handler: action.handler,
                parameters,
                allowed_sources: sources,
                dismiss_policy,
                output: ActionOutputPolicy {
                    allow_copy: action.output.allow_copy,
                    allow_favorite: action.output.allow_favorite,
                    allow_replace: action.output.allow_replace,
                },
            },
        ));
    }
    if actions.is_empty() {
        return Err("script package has no actions".into());
    }
    Ok(ScriptPackageDefinition {
        plugin,
        version: version.to_string(),
        entry: entry.to_string_lossy().replace('\\', "/"),
        actions,
        network_rules,
    })
}

pub(crate) fn read_modules(root: &Path) -> Result<Vec<(String, String)>, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("could not resolve script package root: {error}"))?;
    let mut pending = vec![canonical_root.clone()];
    let mut modules = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("could not scan script modules: {error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                return Err("script packages cannot contain links".into());
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if entry.path().extension().is_some_and(|value| value == "js") {
                validate_module(&entry.path())?;
                let canonical = entry
                    .path()
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                if !canonical.starts_with(&canonical_root) {
                    return Err("script module escapes the package root".into());
                }
                let relative = canonical
                    .strip_prefix(&canonical_root)
                    .map_err(|_| "script module path is invalid")?
                    .to_string_lossy()
                    .replace('\\', "/");
                let source = fs::read_to_string(&canonical)
                    .map_err(|error| format!("script module is not UTF-8: {error}"))?;
                modules.push((relative, source));
                if modules.len() > MAX_MODULES {
                    return Err("script package contains too many modules".into());
                }
            }
        }
    }
    Ok(modules)
}

fn validate_relative_js_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if value.contains(':')
        || value.contains('\\')
        || path.extension().is_none_or(|extension| extension != "js")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("script entry must be a package-relative .js path".into());
    }
    Ok(path)
}

fn validate_module(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "could not inspect script module '{}': {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_MODULE_BYTES {
        return Err(format!(
            "script module '{}' is invalid or too large",
            path.display()
        ));
    }
    Ok(())
}

fn validate_network_rule(rule: ManifestNetworkRule) -> Result<ScriptNetworkRule, String> {
    let scheme = rule.scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err("network rules support only http and https".into());
    }
    let host = rule.host.to_ascii_lowercase();
    if host.is_empty() || host.contains(['/', ':', '@']) || host == "localhost" {
        return Err("network rule host is invalid".into());
    }
    let methods = rule
        .methods
        .into_iter()
        .map(|method| method.to_ascii_uppercase())
        .collect::<Vec<_>>();
    if methods.is_empty()
        || methods
            .iter()
            .any(|method| !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE"))
    {
        return Err("network rule contains an unsupported method".into());
    }
    if rule.path_prefixes.iter().any(|path| !path.starts_with('/')) {
        return Err("network path prefixes must start with '/'".into());
    }
    Ok(ScriptNetworkRule {
        scheme,
        host,
        port: rule.port,
        methods,
        path_prefixes: rule.path_prefixes,
    })
}

fn parse_parameter(parameter: ManifestParameter) -> Result<ActionParameter, String> {
    if !valid_parameter_key(&parameter.key) {
        return Err(format!("invalid parameter key '{}'", parameter.key));
    }
    let kind = match parameter.kind.as_str() {
        "text" => ParameterKind::Text,
        "enum" => ParameterKind::Enum,
        "boolean" => ParameterKind::Boolean,
        "number" => ParameterKind::Number,
        value => return Err(format!("unknown parameter kind '{value}'")),
    };
    let default_value = parameter.default.map(|value| match value {
        toml::Value::String(value) => value,
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        _ => String::new(),
    });
    if default_value.as_deref() == Some("") {
        return Err("parameter defaults must be scalar values".into());
    }
    Ok(ActionParameter {
        key: parameter.key,
        label: parameter.label,
        kind,
        required: parameter.required,
        default_value,
        choices: parameter.choices,
    })
}

fn valid_export_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_parameter_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_localhost_network_rule() {
        let rule = ManifestNetworkRule {
            scheme: "http".into(),
            host: "localhost".into(),
            port: Some(80),
            methods: vec!["GET".into()],
            path_prefixes: vec!["/".into()],
        };
        assert!(validate_network_rule(rule).is_err());
    }

    #[test]
    fn shipped_script_examples_match_the_real_parser() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
        let text = inspect(&examples.join("script-text")).expect("text example parses");
        assert_eq!(text.plugin.id().as_str(), "org.example.script-text");
        let multistep =
            inspect(&examples.join("script-multistep")).expect("multi-step example parses");
        assert_eq!(multistep.network_rules.len(), 1);
    }
}
