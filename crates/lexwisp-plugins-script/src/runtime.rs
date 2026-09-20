use std::{
    cell::RefCell,
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use lexwisp_core::{
    PluginId, ScriptActivation, ScriptHostCall, ScriptHttpRequest, ScriptInvocation,
    ScriptInvocationHost, ScriptPackageDefinition, ScriptPackageFactory, ScriptPluginLifecycle,
    TextActionUiPort,
};
use rquickjs::{
    Context, Ctx, Error, Function, Module, Object, Persistent, Promise, Runtime, Value,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
    promise::{MaybePromise, PromiseState},
};

use crate::{controller::ScriptController, manifest};

const COMMAND_CAPACITY: usize = 256;
const VM_MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const SLICE_LIMIT: Duration = Duration::from_millis(50);
const INVOCATION_JS_LIMIT: Duration = Duration::from_secs(2);
const INVOCATION_WALL_LIMIT: Duration = Duration::from_secs(180);
const MAX_JOBS_PER_TURN: usize = 64;

#[derive(Clone)]
pub(crate) struct RuntimeHandle {
    sender: mpsc::SyncSender<Command>,
}

impl RuntimeHandle {
    pub(crate) async fn invoke(
        &self,
        plugin_id: PluginId,
        handler: String,
        input_json: String,
        params_json: String,
        invocation: Arc<dyn ScriptInvocation>,
    ) -> Result<String, String> {
        let (sender, receiver) = async_channel::bounded(1);
        self.sender
            .try_send(Command::Invoke {
                plugin_id,
                handler,
                input_json,
                params_json,
                invocation,
                reply: sender,
            })
            .map_err(|_| "script worker command queue is full".to_string())?;
        receiver
            .recv()
            .await
            .map_err(|_| "script worker stopped before completing the invocation".to_string())?
    }
}

pub struct ScriptRuntimeFactory {
    worker: Mutex<Option<WorkerHandle>>,
    activation_nonce: AtomicU64,
}

struct WorkerHandle {
    sender: mpsc::SyncSender<Command>,
    thread: thread::JoinHandle<()>,
}

impl Default for ScriptRuntimeFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptRuntimeFactory {
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
            activation_nonce: AtomicU64::new(1),
        }
    }

    fn handle(&self) -> Result<RuntimeHandle, String> {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if worker.is_none() {
            let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
            let worker_sender = sender.clone();
            let thread = thread::Builder::new()
                .name("lexwisp-script".into())
                .spawn(move || {
                    WORKER_SENDER.with(|slot| *slot.borrow_mut() = Some(worker_sender));
                    worker_main(receiver);
                    WORKER_SENDER.with(|slot| *slot.borrow_mut() = None);
                })
                .map_err(|error| format!("could not start script worker: {error}"))?;
            *worker = Some(WorkerHandle {
                sender: sender.clone(),
                thread,
            });
        }
        Ok(RuntimeHandle {
            sender: worker
                .as_ref()
                .expect("worker was just initialized")
                .sender
                .clone(),
        })
    }
}

impl ScriptPackageFactory for ScriptRuntimeFactory {
    fn inspect(&self, root: &Path) -> Result<ScriptPackageDefinition, String> {
        manifest::inspect(root)
    }

    fn activate(
        &self,
        root: &Path,
        definition: &ScriptPackageDefinition,
        host: Arc<dyn ScriptInvocationHost>,
        settings: Arc<dyn lexwisp_core::SettingsUiPort>,
    ) -> Result<ScriptActivation, String> {
        let handle = self.handle()?;
        let modules = manifest::read_modules(root)?;
        let nonce = self.activation_nonce.fetch_add(1, Ordering::Relaxed);
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        handle
            .sender
            .try_send(Command::Load {
                nonce,
                definition: definition.clone(),
                modules,
                reply: reply_sender,
            })
            .map_err(|_| "script worker command queue is full".to_string())?;
        reply_receiver
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "script worker did not finish loading the package".to_string())??;

        let mut controllers = Vec::new();
        let mut handlers = Vec::new();
        for descriptor in &definition.actions {
            let controller = ScriptController::new(
                descriptor.clone(),
                host.clone(),
                settings.clone(),
                handle.clone(),
            )?;
            let controller_port: Arc<dyn TextActionUiPort> = controller.clone();
            let handler: Arc<dyn lexwisp_core::ActionHandler> = controller;
            controllers.push(controller_port);
            handlers.push(handler);
        }
        Ok(ScriptActivation {
            handlers,
            controllers,
            lifecycle: Arc::new(Lifecycle {
                plugin_id: definition.plugin.id().clone(),
                nonce,
                sender: handle.sender,
            }),
        })
    }

    fn shutdown(&self) {
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(worker) = worker {
            let _ = worker.sender.send(Command::Shutdown);
            let _ = worker.thread.join();
        }
    }
}

impl Drop for ScriptRuntimeFactory {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Lifecycle {
    plugin_id: PluginId,
    nonce: u64,
    sender: mpsc::SyncSender<Command>,
}

impl ScriptPluginLifecycle for Lifecycle {
    fn stop(&self) {
        let _ = self.sender.send(Command::Unload {
            plugin_id: self.plugin_id.clone(),
            nonce: self.nonce,
        });
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        let _ = self.sender.try_send(Command::Unload {
            plugin_id: self.plugin_id.clone(),
            nonce: self.nonce,
        });
    }
}

enum Command {
    Load {
        nonce: u64,
        definition: ScriptPackageDefinition,
        modules: Vec<(String, String)>,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
    Unload {
        plugin_id: PluginId,
        nonce: u64,
    },
    Invoke {
        plugin_id: PluginId,
        handler: String,
        input_json: String,
        params_json: String,
        invocation: Arc<dyn ScriptInvocation>,
        reply: async_channel::Sender<Result<String, String>>,
    },
    HostResolved {
        token: u64,
        result: Result<String, String>,
    },
    Shutdown,
}

#[derive(Clone)]
struct ModuleStore(Arc<Mutex<HashMap<String, String>>>);

impl Resolver for ModuleStore {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if !name.starts_with("./") && !name.starts_with("../") {
            return Err(Error::new_resolving(base, name));
        }
        let mut parts = base.split('/').collect::<Vec<_>>();
        parts.pop();
        for part in name.split('/') {
            match part {
                "." | "" => {}
                ".." if parts.len() > 2 => {
                    parts.pop();
                }
                ".." => return Err(Error::new_resolving(base, name)),
                value => parts.push(value),
            }
        }
        let resolved = parts.join("/");
        if self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&resolved)
        {
            Ok(resolved)
        } else {
            Err(Error::new_resolving(base, name))
        }
    }
}

impl Loader for ModuleStore {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let source = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)
            .cloned()
            .ok_or_else(|| Error::new_loading(name))?;
        Module::declare(ctx.clone(), name, source)
    }
}

struct PluginContext {
    nonce: u64,
    handlers: HashMap<String, Persistent<Function<'static>>>,
    // Context drops after Persistent handlers.
    context: Context,
}

struct PendingHostCall {
    plugin_id: PluginId,
    nonce: u64,
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

#[derive(Clone)]
struct HostPromiseScope {
    invocation: Arc<dyn ScriptInvocation>,
    plugin_id: PluginId,
    nonce: u64,
    tokens: Arc<AtomicU64>,
    pending: Rc<RefCell<Vec<(u64, PendingHostCall)>>>,
    sender: mpsc::SyncSender<Command>,
}

struct RunningInvocation {
    plugin_id: PluginId,
    nonce: u64,
    promise: Persistent<Value<'static>>,
    wall_started: Instant,
    js_spent: Duration,
    invocation: Arc<dyn ScriptInvocation>,
    reply: async_channel::Sender<Result<String, String>>,
}

struct VmState {
    modules: ModuleStore,
    plugins: HashMap<PluginId, PluginContext>,
    pending_calls: HashMap<u64, PendingHostCall>,
    running: HashMap<String, RunningInvocation>,
    next_token: Arc<AtomicU64>,
    pending_new: Rc<RefCell<Vec<(u64, PendingHostCall)>>>,
    interrupt_deadline_ns: Arc<AtomicU64>,
    clock_start: Instant,
    // Runtime must drop last: Persistent JS values above are tied to it.
    runtime: Runtime,
}

impl VmState {
    fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|error| error.to_string())?;
        runtime.set_memory_limit(VM_MEMORY_LIMIT);
        runtime.set_max_stack_size(512 * 1024);
        let modules = ModuleStore(Arc::new(Mutex::new(HashMap::new())));
        runtime.set_loader(modules.clone(), modules.clone());
        let clock_start = Instant::now();
        let interrupt_deadline_ns = Arc::new(AtomicU64::new(0));
        let deadline = interrupt_deadline_ns.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let deadline = deadline.load(Ordering::Relaxed);
            deadline != 0
                && u64::try_from(clock_start.elapsed().as_nanos()).unwrap_or(u64::MAX) >= deadline
        })));
        Ok(Self {
            modules,
            plugins: HashMap::new(),
            pending_calls: HashMap::new(),
            running: HashMap::new(),
            next_token: Arc::new(AtomicU64::new(1)),
            pending_new: Rc::new(RefCell::new(Vec::new())),
            interrupt_deadline_ns,
            clock_start,
            runtime,
        })
    }

    fn with_budget<T>(&self, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.arm_budget();
        let result = operation();
        self.interrupt_deadline_ns.store(0, Ordering::Relaxed);
        result
    }

    fn arm_budget(&self) {
        let now = u64::try_from(self.clock_start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let slice = u64::try_from(SLICE_LIMIT.as_nanos()).unwrap_or(u64::MAX);
        self.interrupt_deadline_ns
            .store(now.saturating_add(slice), Ordering::Relaxed);
    }
}

fn worker_main(receiver: mpsc::Receiver<Command>) {
    let mut vm: Option<VmState> = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(2)) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(command) => handle_command(command, &mut vm),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(state) = vm.as_mut() {
            drain_pending_calls(state);
            pump_jobs(state);
            drain_pending_calls(state);
            settle_invocations(state);
            if state.plugins.is_empty() {
                state.runtime.set_interrupt_handler(None);
                vm = None;
            }
        }
    }
    if let Some(mut state) = vm {
        fail_all(&mut state, "script runtime shut down");
    }
}

fn handle_command(command: Command, vm: &mut Option<VmState>) {
    match command {
        Command::Load {
            nonce,
            definition,
            modules,
            reply,
        } => {
            let state = match vm {
                Some(state) => state,
                None => match VmState::new() {
                    Ok(state) => vm.insert(state),
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        return;
                    }
                },
            };
            let result = load_plugin(state, nonce, &definition, modules);
            let _ = reply.send(result);
        }
        Command::Unload { plugin_id, nonce } => {
            if let Some(state) = vm.as_mut() {
                unload_plugin(state, &plugin_id, nonce);
            }
        }
        Command::Invoke {
            plugin_id,
            handler,
            input_json,
            params_json,
            invocation,
            reply,
        } => {
            let result = vm
                .as_mut()
                .ok_or_else(|| "script VM is not active".to_string())
                .and_then(|state| {
                    start_invocation(
                        state,
                        plugin_id,
                        handler,
                        input_json,
                        params_json,
                        invocation,
                        reply.clone(),
                    )
                });
            if let Err(error) = result {
                let _ = reply.try_send(Err(error));
            }
        }
        Command::HostResolved { token, result } => {
            if let Some(state) = vm.as_mut() {
                resolve_host_call(state, token, result);
            }
        }
        Command::Shutdown => unreachable!(),
    }
}

fn load_plugin(
    state: &mut VmState,
    nonce: u64,
    definition: &ScriptPackageDefinition,
    modules: Vec<(String, String)>,
) -> Result<(), String> {
    let prefix = format!("{}/{nonce}", definition.plugin.id());
    {
        let mut store = state
            .modules
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (path, source) in modules {
            store.insert(format!("{prefix}/{path}"), source);
        }
    }
    let entry_name = format!("{prefix}/{}", definition.entry);
    let source = state
        .modules
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&entry_name)
        .cloned()
        .ok_or_else(|| "script entry was not found in the package".to_string())?;
    let context = Context::full(&state.runtime).map_err(|error| error.to_string())?;
    let handlers = state.with_budget(|| {
        context.with(|ctx| {
            let module = Module::declare(ctx.clone(), entry_name.as_str(), source)
                .map_err(|error| js_error(&ctx, error))?;
            let (module, promise) = module.eval().map_err(|error| js_error(&ctx, error))?;
            promise
                .finish::<()>()
                .map_err(|error| js_error(&ctx, error))?;
            let mut handlers = HashMap::new();
            for descriptor in &definition.actions {
                let handler_name = match descriptor.kind() {
                    lexwisp_core::ActionKind::Script(script) => &script.handler,
                    _ => return Err("script package contains a non-script action".into()),
                };
                let function: Function = module
                    .get(handler_name.as_str())
                    .map_err(|error| js_error(&ctx, error))?;
                handlers.insert(handler_name.clone(), Persistent::save(&ctx, function));
            }
            Ok(handlers)
        })
    });
    match handlers {
        Ok(handlers) => {
            state.plugins.insert(
                definition.plugin.id().clone(),
                PluginContext {
                    nonce,
                    handlers,
                    context,
                },
            );
            Ok(())
        }
        Err(error) => {
            state
                .modules
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|name, _| !name.starts_with(&format!("{prefix}/")));
            Err(error)
        }
    }
}

fn unload_plugin(state: &mut VmState, plugin_id: &PluginId, nonce: u64) {
    if state
        .plugins
        .get(plugin_id)
        .is_none_or(|plugin| plugin.nonce != nonce)
    {
        return;
    }
    state.plugins.remove(plugin_id);
    state
        .modules
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|name, _| !name.starts_with(&format!("{plugin_id}/{nonce}/")));
    let running = state
        .running
        .iter()
        .filter(|(_, run)| &run.plugin_id == plugin_id && run.nonce == nonce)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in running {
        if let Some(run) = state.running.remove(&id) {
            let _ = run.reply.try_send(Err("script plugin was disabled".into()));
        }
    }
    state
        .pending_calls
        .retain(|_, call| &call.plugin_id != plugin_id || call.nonce != nonce);
}

fn start_invocation(
    state: &mut VmState,
    plugin_id: PluginId,
    handler: String,
    input_json: String,
    params_json: String,
    invocation: Arc<dyn ScriptInvocation>,
    reply: async_channel::Sender<Result<String, String>>,
) -> Result<(), String> {
    if state.running.values().any(|run| run.plugin_id == plugin_id) {
        return Err("this script plugin already has an active invocation".into());
    }
    let plugin = state
        .plugins
        .get(&plugin_id)
        .ok_or_else(|| "script plugin is not active".to_string())?;
    let nonce = plugin.nonce;
    let function = plugin
        .handlers
        .get(&handler)
        .cloned()
        .ok_or_else(|| format!("script handler '{handler}' is not exported"))?;
    let context = plugin.context.clone();
    let command_sender =
        current_sender().ok_or_else(|| "script worker sender is unavailable".to_string())?;
    let next_token = state.next_token.clone();
    let pending_capture = state.pending_new.clone();
    let invocation_capture = invocation.clone();
    let entry_started = Instant::now();
    let promise = state.with_budget(|| {
        context.with(|ctx| {
            let function = function
                .restore(&ctx)
                .map_err(|error| js_error(&ctx, error))?;
            let ctx_object = build_host_context(
                &ctx,
                invocation_capture,
                plugin_id.clone(),
                nonce,
                command_sender,
                next_token,
                pending_capture,
            )?;
            let input = ctx
                .json_parse(input_json)
                .map_err(|error| js_error(&ctx, error))?;
            let params = ctx
                .json_parse(params_json)
                .map_err(|error| js_error(&ctx, error))?;
            let result: Value = function
                .call((ctx_object, input, params))
                .map_err(|error| js_error(&ctx, error))?;
            Ok(Persistent::save(&ctx, result))
        })
    })?;
    drain_pending_calls(state);
    state.running.insert(
        invocation.id().to_string(),
        RunningInvocation {
            plugin_id,
            nonce,
            promise,
            wall_started: Instant::now(),
            js_spent: entry_started.elapsed(),
            invocation,
            reply,
        },
    );
    Ok(())
}

thread_local! {
    static WORKER_SENDER: std::cell::RefCell<Option<mpsc::SyncSender<Command>>> = const { std::cell::RefCell::new(None) };
}

fn current_sender() -> Option<mpsc::SyncSender<Command>> {
    WORKER_SENDER.with(|sender| sender.borrow().clone())
}

fn build_host_context<'js>(
    ctx: &Ctx<'js>,
    invocation: Arc<dyn ScriptInvocation>,
    plugin_id: PluginId,
    nonce: u64,
    command_sender: mpsc::SyncSender<Command>,
    next_token: Arc<AtomicU64>,
    pending: Rc<RefCell<Vec<(u64, PendingHostCall)>>>,
) -> Result<Object<'js>, String> {
    let scope = HostPromiseScope {
        invocation: invocation.clone(),
        plugin_id,
        nonce,
        tokens: next_token,
        pending,
        sender: command_sender,
    };
    let root = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let output = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let output_invocation = invocation.clone();
    output
        .set(
            "append",
            Function::new(ctx.clone(), move |text: String| {
                if text.len() > OUTPUT_LIMIT {
                    return Err(Error::new_from_js_message(
                        "String",
                        "output",
                        "script output exceeds 2 MiB",
                    ));
                }
                output_invocation
                    .append(&text)
                    .map_err(|error| Error::new_from_js_message("String", "output", error))
            })
            .map_err(|error| js_error(ctx, error))?,
        )
        .map_err(|error| js_error(ctx, error))?;
    root.set("output", output)
        .map_err(|error| js_error(ctx, error))?;

    let ai = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let ai_invocation = invocation.clone();
    ai.set(
        "invoke",
        host_promise_function(
            ctx,
            HostPromiseScope {
                invocation: ai_invocation,
                ..scope.clone()
            },
            |input: String, system: Option<String>| ScriptHostCall::Ai { system, input },
        )?,
    )
    .map_err(|error| js_error(ctx, error))?;
    root.set("ai", ai).map_err(|error| js_error(ctx, error))?;

    let http = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let http_invocation = invocation.clone();
    let http_ctx = ctx.clone();
    let http_scope = scope.clone();
    let function = Function::new(
        ctx.clone(),
        move |method: String, url: String, body: Option<String>| {
            make_host_promise(
                &http_ctx,
                HostPromiseScope {
                    invocation: http_invocation.clone(),
                    ..http_scope.clone()
                },
                ScriptHostCall::Http(ScriptHttpRequest {
                    method,
                    url,
                    headers: Vec::new(),
                    body: body.map(String::into_bytes),
                }),
            )
        },
    )
    .map_err(|error| js_error(ctx, error))?;
    http.set("request", function)
        .map_err(|error| js_error(ctx, error))?;
    root.set("http", http)
        .map_err(|error| js_error(ctx, error))?;

    let storage = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let operations = ["get", "set", "delete"];
    for operation in operations {
        let call_invocation = invocation.clone();
        let call_scope = scope.clone();
        let call_ctx = ctx.clone();
        let function = match operation {
            "get" => Function::new(ctx.clone(), move |key: String| {
                make_host_promise(
                    &call_ctx,
                    HostPromiseScope {
                        invocation: call_invocation.clone(),
                        ..call_scope.clone()
                    },
                    ScriptHostCall::StorageGet { key },
                )
            })
            .map_err(|error| js_error(ctx, error))?,
            "set" => Function::new(ctx.clone(), move |key: String, value: String| {
                make_host_promise(
                    &call_ctx,
                    HostPromiseScope {
                        invocation: call_invocation.clone(),
                        ..call_scope.clone()
                    },
                    ScriptHostCall::StorageSet { key, value },
                )
            })
            .map_err(|error| js_error(ctx, error))?,
            _ => Function::new(ctx.clone(), move |key: String| {
                make_host_promise(
                    &call_ctx,
                    HostPromiseScope {
                        invocation: call_invocation.clone(),
                        ..call_scope.clone()
                    },
                    ScriptHostCall::StorageDelete { key },
                )
            })
            .map_err(|error| js_error(ctx, error))?,
        };
        storage
            .set(operation, function)
            .map_err(|error| js_error(ctx, error))?;
    }
    root.set("storage", storage)
        .map_err(|error| js_error(ctx, error))?;

    let context = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let snapshot = ctx
        .json_parse(invocation.context_json())
        .map_err(|error| js_error(ctx, error))?;
    context
        .set("snapshot", snapshot)
        .map_err(|error| js_error(ctx, error))?;
    root.set("context", context)
        .map_err(|error| js_error(ctx, error))?;
    let signal = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let signal_invocation = invocation.clone();
    signal
        .set(
            "isAborted",
            Function::new(ctx.clone(), move || signal_invocation.is_cancelled())
                .map_err(|error| js_error(ctx, error))?,
        )
        .map_err(|error| js_error(ctx, error))?;
    root.set("signal", signal)
        .map_err(|error| js_error(ctx, error))?;
    let ui = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let ui_invocation = invocation.clone();
    ui.set(
        "showResult",
        Function::new(ctx.clone(), move || {
            ui_invocation
                .show_result()
                .map_err(|error| Error::new_from_js_message("request", "ui", error))
        })
        .map_err(|error| js_error(ctx, error))?,
    )
    .map_err(|error| js_error(ctx, error))?;
    root.set("ui", ui).map_err(|error| js_error(ctx, error))?;
    let log = Object::new(ctx.clone()).map_err(|error| js_error(ctx, error))?;
    let log_invocation = invocation;
    log.set(
        "write",
        Function::new(ctx.clone(), move |level: String, category: String| {
            log_invocation.log(&level, &category)
        })
        .map_err(|error| js_error(ctx, error))?,
    )
    .map_err(|error| js_error(ctx, error))?;
    root.set("log", log).map_err(|error| js_error(ctx, error))?;
    Ok(root)
}

fn host_promise_function<'js, F>(
    ctx: &Ctx<'js>,
    scope: HostPromiseScope,
    make: F,
) -> Result<Function<'js>, String>
where
    F: Fn(String, Option<String>) -> ScriptHostCall + Clone + 'static,
{
    let promise_ctx = ctx.clone();
    Function::new(ctx.clone(), move |input: String, system: Option<String>| {
        make_host_promise(&promise_ctx, scope.clone(), make.clone()(input, system))
    })
    .map_err(|error| js_error(ctx, error))
}

fn make_host_promise<'js>(
    ctx: &Ctx<'js>,
    scope: HostPromiseScope,
    call: ScriptHostCall,
) -> rquickjs::Result<Promise<'js>> {
    let token = scope.tokens.fetch_add(1, Ordering::Relaxed);
    let (promise, resolve, reject) = Promise::new(ctx)?;
    scope.pending.borrow_mut().push((
        token,
        PendingHostCall {
            plugin_id: scope.plugin_id.clone(),
            nonce: scope.nonce,
            resolve: Persistent::save(ctx, resolve),
            reject: Persistent::save(ctx, reject),
        },
    ));
    scope.invocation.dispatch(
        call,
        Box::new(move |result| {
            let _ = scope
                .sender
                .try_send(Command::HostResolved { token, result });
        }),
    );
    Ok(promise)
}

fn resolve_host_call(state: &mut VmState, token: u64, result: Result<String, String>) {
    let Some(call) = state.pending_calls.remove(&token) else {
        return;
    };
    let Some(plugin) = state.plugins.get(&call.plugin_id) else {
        return;
    };
    if plugin.nonce != call.nonce {
        return;
    }
    plugin.context.with(|ctx| match result {
        Ok(value) => {
            if let Ok(resolve) = call.resolve.restore(&ctx) {
                let _ = resolve.call::<_, ()>((value,));
            }
        }
        Err(error) => {
            if let Ok(reject) = call.reject.restore(&ctx) {
                let _ = reject.call::<_, ()>((error,));
            }
        }
    });
}

fn drain_pending_calls(state: &mut VmState) {
    let mut pending = state.pending_new.borrow_mut();
    for (token, call) in pending.drain(..) {
        state.pending_calls.insert(token, call);
    }
}

fn pump_jobs(state: &mut VmState) {
    for _ in 0..MAX_JOBS_PER_TURN {
        let started = Instant::now();
        state.arm_budget();
        let ran = state.runtime.execute_pending_job();
        state.interrupt_deadline_ns.store(0, Ordering::Relaxed);
        match ran {
            Ok(true) => {
                let elapsed = started.elapsed().max(Duration::from_micros(100));
                for invocation in state.running.values_mut() {
                    invocation.js_spent = invocation.js_spent.saturating_add(elapsed);
                }
            }
            Ok(false) => break,
            Err(_) => {
                fail_all(state, "script VM job failed");
                break;
            }
        }
    }
}

fn settle_invocations(state: &mut VmState) {
    let ids = state.running.keys().cloned().collect::<Vec<_>>();
    for id in ids {
        let Some(run) = state.running.get(&id) else {
            continue;
        };
        if run.invocation.is_cancelled() {
            finish_running(state, &id, Err("script invocation was cancelled".into()));
            continue;
        }
        if run.js_spent > INVOCATION_JS_LIMIT {
            finish_running(
                state,
                &id,
                Err("script JS execution budget exceeded".into()),
            );
            continue;
        }
        if run.wall_started.elapsed() > INVOCATION_WALL_LIMIT {
            finish_running(
                state,
                &id,
                Err("script invocation exceeded 180 seconds".into()),
            );
            continue;
        }
        let Some(plugin) = state.plugins.get(&run.plugin_id) else {
            finish_running(state, &id, Err("script plugin is no longer active".into()));
            continue;
        };
        let result = plugin.context.with(|ctx| {
            let value = run
                .promise
                .clone()
                .restore(&ctx)
                .map_err(|error| js_error(&ctx, error))?;
            let promise = MaybePromise::from_value(value);
            match promise.state() {
                PromiseState::Pending => Ok(None),
                PromiseState::Resolved => {
                    let value: Value = promise
                        .result()
                        .expect("resolved promise has a result")
                        .map_err(|error| js_error(&ctx, error))?;
                    parse_result(&ctx, value, run.invocation.output_len()).map(Some)
                }
                PromiseState::Rejected => Err(promise
                    .result::<Value>()
                    .expect("rejected promise has a result")
                    .expect_err("rejected result is an error"))
                .map_err(|error| js_error(&ctx, error)),
            }
        });
        match result {
            Ok(Some(output)) => finish_running(state, &id, Ok(output)),
            Ok(None) => {}
            Err(error) => finish_running(state, &id, Err(error)),
        }
    }
}

fn parse_result<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    existing_output: usize,
) -> Result<String, String> {
    let text =
        if let Some(text) = value.as_string() {
            text.to_string().map_err(|error| js_error(ctx, error))?
        } else if let Some(object) = value.as_object() {
            let result_type: String = object.get("type").map_err(|error| js_error(ctx, error))?;
            match result_type.as_str() {
                "text" => {
                    let text: rquickjs::String =
                        object.get("text").map_err(|error| js_error(ctx, error))?;
                    let text = text.to_cstring().map_err(|error| js_error(ctx, error))?;
                    if text.len() > OUTPUT_LIMIT {
                        return Err("script output exceeds 2 MiB".into());
                    }
                    text.as_str().to_owned()
                }
                "complete" => String::new(),
                _ => return Err(
                    "script result must be text, { type: 'text', text }, or { type: 'complete' }"
                        .into(),
                ),
            }
        } else {
            return Err(
                "script returned undefined; return a text result or { type: 'complete' }".into(),
            );
        };
    if text.len().saturating_add(existing_output) > OUTPUT_LIMIT {
        return Err("script output exceeds 2 MiB".into());
    }
    if existing_output > 0 && !text.is_empty() {
        return Err("a script cannot both append streamed output and return final text".into());
    }
    Ok(text)
}

fn finish_running(state: &mut VmState, id: &str, result: Result<String, String>) {
    if let Some(run) = state.running.remove(id) {
        let _ = run.reply.try_send(result);
    }
}

fn fail_all(state: &mut VmState, reason: &str) {
    for (_, run) in state.running.drain() {
        let _ = run.reply.try_send(Err(reason.into()));
    }
    state.pending_calls.clear();
    state.plugins.clear();
}

fn js_error(ctx: &Ctx<'_>, error: Error) -> String {
    if matches!(error, Error::Exception) {
        let caught = ctx.catch();
        if let Ok(Some(json)) = ctx.json_stringify(caught.clone())
            && let Ok(json) = json.to_string()
        {
            return format!("JavaScript error: {json}");
        }
        return format!("JavaScript exception: {caught:?}");
    }
    format!("JavaScript error: {error}")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use lexwisp_core::{
        ActionRequest, AppSettings, ExecutionObserver, InvocationId, PluginId, ScriptFuture,
        ScriptHostCall, ScriptInvocation, ScriptInvocationHost, ScriptPackageFactory,
        SettingsError, SettingsFuture, SettingsSnapshot, SettingsUiPort,
    };

    use super::ScriptRuntimeFactory;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct FakeSettings;

    impl SettingsUiPort for FakeSettings {
        fn snapshot(&self) -> SettingsSnapshot {
            SettingsSnapshot::new(AppSettings::default(), 1)
        }

        fn apply(&self, _: AppSettings) -> SettingsFuture<'_> {
            Box::pin(async { Err(SettingsError::Save("fixture is read-only".into())) })
        }

        fn reload(&self) -> SettingsFuture<'_> {
            Box::pin(async { Err(SettingsError::Load("fixture is read-only".into())) })
        }
    }

    struct FakeHost {
        delay: Duration,
    }

    impl ScriptInvocationHost for FakeHost {
        fn begin<'a>(
            &'a self,
            action: &'a lexwisp_core::ActionDescriptor,
            _: ActionRequest,
            _: Arc<dyn ExecutionObserver>,
        ) -> ScriptFuture<'a, Arc<dyn ScriptInvocation>> {
            let invocation: Arc<dyn ScriptInvocation> = Arc::new(FakeInvocation {
                id: InvocationId::new(),
                plugin_id: action.plugin_id().clone(),
                output: Mutex::new(String::new()),
                cancelled: AtomicBool::new(false),
                delay: self.delay,
            });
            Box::pin(async move { Ok(invocation) })
        }

        fn cancel(&self, _: &InvocationId) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeInvocation {
        id: InvocationId,
        plugin_id: PluginId,
        output: Mutex<String>,
        cancelled: AtomicBool,
        delay: Duration,
    }

    impl ScriptInvocation for FakeInvocation {
        fn id(&self) -> &InvocationId {
            &self.id
        }
        fn plugin_id(&self) -> &PluginId {
            &self.plugin_id
        }
        fn context_json(&self) -> String {
            "{}".into()
        }
        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }
        fn output_len(&self) -> usize {
            self.output.lock().expect("output lock").len()
        }
        fn append(&self, text: &str) -> Result<(), String> {
            self.output.lock().expect("output lock").push_str(text);
            Ok(())
        }
        fn dispatch(
            &self,
            call: ScriptHostCall,
            completion: Box<dyn FnOnce(Result<String, String>) + Send>,
        ) {
            let delay = self.delay;
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let result = match call {
                    ScriptHostCall::Ai { input, .. } => Ok(format!("AI:{input}")),
                    _ => Err("unsupported fake call".into()),
                };
                completion(result);
            });
        }
        fn show_result(&self) -> Result<(), String> {
            Ok(())
        }
        fn log(&self, _: &str, _: &str) {}
        fn finish<'a>(&'a self, result: Result<String, String>) -> ScriptFuture<'a, String> {
            Box::pin(async move {
                let text = result?;
                self.output.lock().expect("output lock").push_str(&text);
                Ok(self.output.lock().expect("output lock").clone())
            })
        }
    }

    fn root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir()
            .join("lexwisp-script-tests")
            .join(format!(
                "{label}-{}-{nonce}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ))
    }

    fn write_package(root: &PathBuf, id: &str, body: &str) {
        fs::create_dir_all(root).expect("fixture directory");
        let manifest = format!(
            r#"
schema_version = 1
[plugin]
id = "{id}"
name = "Fixture"
version = "1.0.0"
kind = "script"
host_api = "^1.0"
entry = "main.js"
[capabilities]
required = []
optional = []
[[actions]]
id = "run"
name = "Run"
handler = "run"
input_kind = "text"
allowed_sources = ["manual"]
dismiss_policy = "cancel"
[actions.output]
format = "text"
allow_copy = true
allow_favorite = true
allow_replace = false
"#
        );
        fs::write(root.join("manifest.toml"), manifest).expect("manifest fixture");
        fs::write(root.join("main.js"), body).expect("module fixture");
    }

    fn run(
        factory: &ScriptRuntimeFactory,
        root: &Path,
        input: &str,
    ) -> Result<String, lexwisp_core::ActionError> {
        let definition = factory.inspect(root).expect("package inspects");
        let activation = factory
            .activate(
                root,
                &definition,
                Arc::new(FakeHost {
                    delay: Duration::ZERO,
                }),
                Arc::new(FakeSettings),
            )
            .expect("package activates");
        let result = futures_lite::future::block_on(
            activation.handlers[0].execute(ActionRequest::manual(input)),
        );
        activation.lifecycle.stop();
        result.map(|result| result.output)
    }

    #[test]
    fn runs_packaged_module_without_node() {
        let root = root("pure-text");
        write_package(
            &root,
            "org.example.script-text",
            "import { upper } from './modules/text.js'; export async function run(_ctx, input) { return { type: 'text', text: upper(input.text) }; }",
        );
        fs::create_dir_all(root.join("modules")).expect("modules directory");
        fs::write(
            root.join("modules/text.js"),
            "export const upper = value => value.toUpperCase();",
        )
        .expect("module");
        let factory = ScriptRuntimeFactory::new();
        assert_eq!(run(&factory, &root, "hello").expect("script runs"), "HELLO");
        factory.shutdown();
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn interrupts_sync_loop_and_oversized_output() {
        let loop_root = root("sync-loop");
        write_package(
            &loop_root,
            "org.example.script-loop",
            "export function run() { while (true) {} }",
        );
        let factory = ScriptRuntimeFactory::new();
        assert!(
            run(&factory, &loop_root, "x")
                .expect_err("loop is interrupted")
                .to_string()
                .contains("JavaScript")
        );
        factory.shutdown();
        fs::remove_dir_all(loop_root).expect("fixture cleanup");

        let large_root = root("large-output");
        write_package(
            &large_root,
            "org.example.script-large",
            "export function run() { return { type: 'text', text: 'x'.repeat(2097153) }; }",
        );
        let factory = ScriptRuntimeFactory::new();
        assert!(
            run(&factory, &large_root, "x")
                .expect_err("large output is rejected")
                .to_string()
                .contains("2 MiB")
        );
        factory.shutdown();
        fs::remove_dir_all(large_root).expect("fixture cleanup");
    }

    #[test]
    fn bounds_microtasks_and_rejects_late_host_completion_after_disable() {
        let microtask_root = root("microtasks");
        write_package(
            &microtask_root,
            "org.example.script-microtasks",
            "export async function run() { while (true) { await Promise.resolve(); } }",
        );
        let factory = ScriptRuntimeFactory::new();
        let started = std::time::Instant::now();
        let _error =
            run(&factory, &microtask_root, "x").expect_err("unbounded microtasks are interrupted");
        assert!(started.elapsed() < Duration::from_secs(5));
        factory.shutdown();
        fs::remove_dir_all(microtask_root).expect("fixture cleanup");

        let delayed_root = root("late-host");
        write_package(
            &delayed_root,
            "org.example.script-late-host",
            "export async function run(ctx) { const value = await ctx.ai.invoke('slow'); ctx.output.append(value); return { type: 'complete' }; }",
        );
        let factory = Arc::new(ScriptRuntimeFactory::new());
        let definition = factory.inspect(&delayed_root).expect("package inspects");
        let activation = factory
            .activate(
                &delayed_root,
                &definition,
                Arc::new(FakeHost {
                    delay: Duration::from_millis(200),
                }),
                Arc::new(FakeSettings),
            )
            .expect("package activates");
        let handler = activation.handlers[0].clone();
        let task = std::thread::spawn(move || {
            futures_lite::future::block_on(handler.execute(ActionRequest::manual("x")))
        });
        std::thread::sleep(Duration::from_millis(25));
        activation.lifecycle.stop();
        let result = task.join().expect("execution thread joins");
        let _error = result.expect_err("disabled plugin cannot accept a late Promise");
        std::thread::sleep(Duration::from_millis(250));
        factory.shutdown();
        fs::remove_dir_all(delayed_root).expect("fixture cleanup");
    }
}
