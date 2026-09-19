use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use lexwisp_core::{
    ActionDescriptor, ActionHandler, ActionId, Capability, PluginDescriptor, PluginId,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryEvent {
    PackageRegistered {
        plugin_id: PluginId,
        generation: u64,
    },
    PackageRemoved {
        plugin_id: PluginId,
        generation: u64,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RegistryError {
    #[error("plugin '{0}' is already registered")]
    DuplicatePlugin(PluginId),
    #[error("action '{0}' is already registered")]
    DuplicateAction(ActionId),
    #[error("action '{action}' belongs to '{actual}', not '{expected}'")]
    ActionOwnership {
        action: ActionId,
        expected: PluginId,
        actual: PluginId,
    },
    #[error("plugin display name cannot be empty")]
    EmptyPluginName,
    #[error("action display name cannot be empty")]
    EmptyActionName,
}

#[derive(Default)]
struct RegistryState {
    generation: u64,
    plugins: HashMap<PluginId, PluginDescriptor>,
    actions: HashMap<ActionId, (ActionDescriptor, Arc<dyn ActionHandler>)>,
    subscribers: Vec<async_channel::Sender<RegistryEvent>>,
}

#[derive(Clone, Default)]
pub struct PluginRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl PluginRegistry {
    pub fn register_package(
        &self,
        descriptor: PluginDescriptor,
        actions: Vec<(ActionDescriptor, Arc<dyn ActionHandler>)>,
    ) -> Result<u64, RegistryError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if descriptor.display_name().trim().is_empty() {
            return Err(RegistryError::EmptyPluginName);
        }
        if state.plugins.contains_key(descriptor.id()) {
            return Err(RegistryError::DuplicatePlugin(descriptor.id().clone()));
        }
        let mut package_action_ids = HashSet::new();
        for (action, _) in &actions {
            if action.plugin_id() != descriptor.id() {
                return Err(RegistryError::ActionOwnership {
                    action: action.id().clone(),
                    expected: descriptor.id().clone(),
                    actual: action.plugin_id().clone(),
                });
            }
            if action.display_name().trim().is_empty() {
                return Err(RegistryError::EmptyActionName);
            }
            if state.actions.contains_key(action.id())
                || !package_action_ids.insert(action.id().clone())
            {
                return Err(RegistryError::DuplicateAction(action.id().clone()));
            }
        }

        let plugin_id = descriptor.id().clone();
        state.plugins.insert(plugin_id.clone(), descriptor);
        for (descriptor, handler) in actions {
            state
                .actions
                .insert(descriptor.id().clone(), (descriptor, handler));
        }
        state.generation = state.generation.saturating_add(1);
        let generation = state.generation;
        publish(
            &mut state.subscribers,
            RegistryEvent::PackageRegistered {
                plugin_id,
                generation,
            },
        );
        Ok(generation)
    }

    pub fn descriptors(&self) -> Vec<PluginDescriptor> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .plugins
            .values()
            .cloned()
            .collect()
    }

    pub fn remove_package(&self, plugin_id: &PluginId) -> bool {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.plugins.remove(plugin_id).is_none() {
            return false;
        }
        state
            .actions
            .retain(|_, (descriptor, _)| descriptor.plugin_id() != plugin_id);
        state.generation = state.generation.saturating_add(1);
        let generation = state.generation;
        publish(
            &mut state.subscribers,
            RegistryEvent::PackageRemoved {
                plugin_id: plugin_id.clone(),
                generation,
            },
        );
        true
    }

    pub fn generation(&self) -> u64 {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
    }

    pub fn subscribe(&self, capacity: usize) -> async_channel::Receiver<RegistryEvent> {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .subscribers
            .push(sender);
        receiver
    }

    pub fn actions(&self) -> ActionRegistry {
        ActionRegistry {
            state: self.state.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct ActionRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl ActionRegistry {
    pub fn handler(&self, id: &ActionId) -> Option<Arc<dyn ActionHandler>> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .actions
            .get(id)
            .map(|(_, handler)| handler.clone())
    }

    pub fn descriptors(&self) -> Vec<ActionDescriptor> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .actions
            .values()
            .map(|(descriptor, _)| descriptor.clone())
            .collect()
    }
}

#[derive(Clone, Default)]
pub struct CapabilityAuthority {
    grants: Arc<RwLock<HashMap<PluginId, HashSet<Capability>>>>,
}

impl CapabilityAuthority {
    pub fn replace_grants(&self, plugin: PluginId, grants: impl IntoIterator<Item = Capability>) {
        self.grants
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(plugin, grants.into_iter().collect());
    }

    pub fn is_granted(&self, plugin: &PluginId, capability: Capability) -> bool {
        self.grants
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(plugin)
            .is_some_and(|grants| grants.contains(&capability))
    }

    pub fn revoke(&self, plugin: &PluginId) {
        self.grants
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(plugin);
    }
}

fn publish(subscribers: &mut Vec<async_channel::Sender<RegistryEvent>>, event: RegistryEvent) {
    subscribers.retain(|subscriber| match subscriber.try_send(event.clone()) {
        Ok(()) | Err(async_channel::TrySendError::Full(_)) => true,
        Err(async_channel::TrySendError::Closed(_)) => false,
    });
}

#[cfg(test)]
mod tests {
    use std::{future, pin::Pin};

    use lexwisp_core::{ActionError, ActionRequest, ActionResult};

    use super::*;

    struct FixtureHandler;

    impl ActionHandler for FixtureHandler {
        fn execute<'a>(
            &'a self,
            request: ActionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<ActionResult, ActionError>> + Send + 'a>> {
            Box::pin(future::ready(Ok(ActionResult {
                output: request.input,
            })))
        }
    }

    fn fixture() -> (PluginDescriptor, ActionDescriptor) {
        let plugin = PluginId::parse("org.lexwisp.fixture").expect("fixture ID is valid");
        (
            PluginDescriptor::new(plugin.clone(), "Fixture", vec![Capability::StorageRead]),
            ActionDescriptor::new(
                plugin,
                ActionId::parse("echo").expect("fixture action ID is valid"),
                "Echo",
            ),
        )
    }

    #[test]
    fn package_registration_is_atomic_on_conflict() {
        let registry = PluginRegistry::default();
        let (plugin, action) = fixture();
        registry
            .register_package(
                plugin.clone(),
                vec![(action.clone(), Arc::new(FixtureHandler))],
            )
            .expect("first registration succeeds");

        let result = registry.register_package(plugin, vec![(action, Arc::new(FixtureHandler))]);
        assert!(matches!(result, Err(RegistryError::DuplicatePlugin(_))));
        assert_eq!(registry.descriptors().len(), 1);
        assert_eq!(registry.actions().descriptors().len(), 1);
    }

    #[test]
    fn action_owner_must_match_package() {
        let registry = PluginRegistry::default();
        let (plugin, _) = fixture();
        let other = PluginId::parse("org.lexwisp.other").expect("fixture ID is valid");
        let action = ActionDescriptor::new(
            other,
            ActionId::parse("echo").expect("fixture action ID is valid"),
            "Echo",
        );
        assert!(matches!(
            registry.register_package(plugin, vec![(action, Arc::new(FixtureHandler))]),
            Err(RegistryError::ActionOwnership { .. })
        ));
        assert!(registry.descriptors().is_empty());
    }

    #[test]
    fn removing_a_package_removes_only_its_actions() {
        let registry = PluginRegistry::default();
        let (plugin, action) = fixture();
        let plugin_id = plugin.id().clone();
        registry
            .register_package(plugin, vec![(action, Arc::new(FixtureHandler))])
            .expect("fixture should register");

        assert!(registry.remove_package(&plugin_id));
        assert!(registry.descriptors().is_empty());
        assert!(registry.actions().descriptors().is_empty());
        assert_eq!(registry.generation(), 2);
    }
}
