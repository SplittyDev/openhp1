use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use openhp1_package::{
    ObjectReader, ObjectReference, Package, PackageStore, PropertyKind, ResolveError,
    ResolvedObject,
};
use openhp1_script::{
    Bytecode, PropertyMetadata, ScriptExport, ScriptMetadata, class_defaults_reader,
};
use thiserror::Error;

use crate::{Frame, FrameRequest, FrameResponse, FunctionCall, StructMember, Value};

const GOTO_STATE: u16 = 0x071;
const ENABLE: u16 = 0x075;
const DISABLE: u16 = 0x076;
const DESTROY: u16 = 0x117;
const ALL_ACTORS: u16 = 0x130;
const LOOP_ANIM: u16 = 0x104;
const SET_COLLISION: u16 = 0x106;
const SET_LOCATION: u16 = 0x10b;
const SET_TIMER: u16 = 0x118;
const MAX_CALL_DEPTH: usize = 64;
const PROPERTY_PARAMETER: u32 = 0x80;
const PROPERTY_RETURN: u32 = 0x400;
const STATE_AUTO: u32 = 0x0000_0002;
const PROBE_EVENTS: [&str; 64] = [
    "Spawned",
    "Destroyed",
    "GainedChild",
    "LostChild",
    "Probe4",
    "Probe5",
    "Trigger",
    "UnTrigger",
    "Timer",
    "HitWall",
    "Falling",
    "Landed",
    "ZoneChange",
    "Touch",
    "UnTouch",
    "Bump",
    "BeginState",
    "EndState",
    "BaseChange",
    "Attach",
    "Detach",
    "ActorEntered",
    "ActorLeaving",
    "KillCredit",
    "AnimEnd",
    "EndedRotation",
    "InterpolateEnd",
    "EncroachingOn",
    "EncroachedBy",
    "FootZoneChange",
    "HeadZoneChange",
    "PainTimer",
    "SpeechTimer",
    "MayFall",
    "Probe34",
    "Die",
    "Tick",
    "PlayerTick",
    "Expired",
    "Probe39",
    "SeePlayer",
    "EnemyNotVisible",
    "HearNoise",
    "UpdateEyeHeight",
    "SeeMonster",
    "SeeFriend",
    "SpecialHandling",
    "BotDesireability",
    "Probe48",
    "Probe49",
    "Probe50",
    "Probe51",
    "Probe52",
    "Probe53",
    "Probe54",
    "Probe55",
    "Probe56",
    "Probe57",
    "Probe58",
    "Probe59",
    "Probe60",
    "Probe61",
    "Probe62",
    "All",
];

pub type DispatchResult<T> = std::result::Result<T, DispatchError>;

#[derive(Clone, Debug, PartialEq)]
pub enum ActorAction {
    LoopAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
    },
    SetLocation {
        actor: usize,
        location: [f32; 3],
    },
    DestroyActor {
        actor: usize,
    },
    DeferredCall {
        actor: usize,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error(transparent)]
    Resolve(#[from] ResolveError),

    #[error(transparent)]
    Script(#[from] openhp1_script::Error),

    #[error(transparent)]
    Vm(#[from] crate::Error),

    #[error("script call depth exceeds {MAX_CALL_DEPTH}")]
    CallDepth,

    #[error("script class export {export_index} has no class metadata")]
    InvalidClass { export_index: usize },

    #[error("runtime object handle space is exhausted")]
    ObjectLimit,

    #[error("runtime object handle {handle} is invalid")]
    InvalidObjectHandle { handle: i32 },

    #[error("runtime object handle {handle} does not identify a registered actor")]
    InvalidActorHandle { handle: i32 },

    #[error("actor {actor} instance is already active")]
    ActiveActorContext { actor: usize },

    #[error("runtime actor {actor} is not registered")]
    UnregisteredActor { actor: usize },

    #[error("name `{name}` is missing from package `{package}`")]
    MissingName { package: Arc<str>, name: String },

    #[error("runtime object is unresolved: {message}")]
    UnresolvedObject { message: String },

    #[error("runtime delta time {value} is invalid")]
    InvalidDeltaTime { value: f32 },
}

pub struct ScriptRuntime {
    packages: PackageStore,
    instances: HashMap<usize, InstanceState>,
    class_defaults: HashMap<ObjectId, InstanceState>,
    class_relations: HashMap<(ObjectId, ObjectId), bool>,
    fields: HashMap<(ObjectId, String), Option<ObjectId>>,
    actor_classes: HashMap<usize, ObjectId>,
    // ponytail: store state identity only; add label/IP state frames when state code ticks.
    actor_states: HashMap<usize, Option<String>>,
    disabled_events: HashMap<(usize, String), HashSet<String>>,
    object_actors: HashMap<ObjectId, usize>,
    actor_objects: HashMap<usize, ObjectId>,
    destroyed: HashSet<usize>,
    timers: HashMap<usize, ActorTimer>,
    timer_callbacks: usize,
    object_handles: HashMap<ObjectId, i32>,
    handle_objects: Vec<ObjectId>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ObjectId {
    package: Arc<str>,
    export_index: usize,
}

#[derive(Clone, Debug)]
enum StoredValue {
    Value(Value),
    Name(String),
    Object(Option<ObjectId>),
    UnresolvedObject(String),
    SelfObject,
}

#[derive(Clone, Copy, Debug)]
struct ActorTimer {
    remaining: f32,
    rate: f32,
    looping: bool,
}

type InstanceState = HashMap<ObjectId, StoredValue>;

impl ScriptRuntime {
    pub fn new(game_root: impl AsRef<Path>) -> DispatchResult<Self> {
        Ok(Self {
            packages: PackageStore::scan_game_root(game_root)?,
            instances: HashMap::new(),
            class_defaults: HashMap::new(),
            class_relations: HashMap::new(),
            fields: HashMap::new(),
            actor_classes: HashMap::new(),
            actor_states: HashMap::new(),
            disabled_events: HashMap::new(),
            object_actors: HashMap::new(),
            actor_objects: HashMap::new(),
            destroyed: HashSet::new(),
            timers: HashMap::new(),
            timer_callbacks: 0,
            object_handles: HashMap::new(),
            handle_objects: Vec::new(),
        })
    }

    pub fn register_actor(
        &mut self,
        actor: usize,
        actor_package: impl AsRef<Path>,
        actor_export: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
    ) -> DispatchResult<()> {
        let actor_package = self.packages.load_path(actor_package)?;
        let actor_entry = actor_package.summary().exports.get(actor_export).ok_or(
            openhp1_package::Error::InvalidExportIndex {
                package: Arc::clone(&actor_package.summary().source),
                index: actor_export,
                export_count: actor_package.summary().exports.len(),
            },
        )?;
        let class = ResolvedObject {
            package: self.packages.load_path(class_package)?,
            export_index: class_export,
        };
        let object = object_id(&actor_package, actor_export);
        self.object_handle(object.clone())?;
        self.object_actors.insert(object.clone(), actor);
        self.actor_objects.insert(actor, object.clone());
        self.actor_classes
            .insert(actor, object_id(&class.package, class.export_index));

        let mut instance = self.load_class_defaults(&class, 0)?;
        let mut reader = actor_package.export_reader(actor_export)?;
        let stack = reader.read_object_stack(actor_entry.object_flags)?;
        let state = stack
            .and_then(|stack| {
                (stack.function != ObjectReference::None)
                    .then_some(stack.function)
                    .or((stack.state != ObjectReference::None).then_some(stack.state))
            })
            .map(|state| self.packages.resolve(&actor_package, state))
            .transpose()?
            .flatten()
            .map(|state| {
                state
                    .package
                    .summary()
                    .name(state.package.summary().exports[state.export_index].object_name)
                    .to_owned()
            });
        if let Some(stack) = stack {
            for (index, event) in PROBE_EVENTS.iter().enumerate() {
                if stack.probe_mask & (1_u64 << index) != 0 {
                    set_event_disabled(
                        &mut self.disabled_events,
                        actor,
                        state.as_deref(),
                        event,
                        true,
                    );
                }
            }
        }
        self.actor_states.insert(actor, state);
        self.apply_properties(&class, &actor_package, &mut reader, &mut instance)?;
        self.instances.insert(actor, instance);
        Ok(())
    }

    pub fn dispatch_event(
        &mut self,
        actor: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
        event: &str,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.dispatch_event_with_arguments(actor, class_package, class_export, event, &[])
    }

    pub fn tick(&mut self, delta_time: f32) -> DispatchResult<Vec<ActorAction>> {
        if !delta_time.is_finite() || delta_time < 0.0 {
            return Err(DispatchError::InvalidDeltaTime { value: delta_time });
        }
        let mut due = Vec::new();
        let actors = self.timers.keys().copied().collect::<Vec<_>>();
        for actor in actors {
            let Some(timer) = self.timers.get_mut(&actor) else {
                continue;
            };
            if !advance_timer(timer, delta_time) {
                continue;
            }
            due.push(actor);
            if !timer.looping {
                self.timers.remove(&actor);
            }
        }

        let mut actions = Vec::new();
        for actor in due {
            let Some(class) = self.actor_classes.get(&actor).cloned() else {
                continue;
            };
            self.timer_callbacks = self.timer_callbacks.saturating_add(1);
            match self.dispatch_event(
                actor,
                Path::new(class.package.as_ref()),
                class.export_index,
                "Timer",
            ) {
                Ok(mut actor_actions) => actions.append(&mut actor_actions),
                Err(error) => actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Timer: {error}"),
                }),
            }
        }
        Ok(actions)
    }

    pub fn timer_callbacks(&self) -> usize {
        self.timer_callbacks
    }

    pub fn dispatch_event_with_arguments(
        &mut self,
        actor: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
        event: &str,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        if self.destroyed.contains(&actor) && !event.eq_ignore_ascii_case("Destroyed") {
            return Ok(Vec::new());
        }
        let package = self.packages.load_path(class_package)?;
        let class = ResolvedObject {
            package,
            export_index: class_export,
        };
        let actor_class = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        if event_disabled(
            &self.disabled_events,
            actor,
            self.actor_states
                .get(&actor)
                .and_then(|state| state.as_deref()),
            event,
        ) || self.state_ignores_event(actor, &class, event)?
        {
            return Ok(Vec::new());
        }
        let Some(function) = self.find_actor_function(actor, class, event, 0)? else {
            return Ok(Vec::new());
        };
        let mut actions = Vec::new();
        let mut instance = self.instances.remove(&actor).unwrap_or_default();
        let result = self.execute_function(
            actor,
            &actor_class,
            &function,
            arguments,
            &mut instance,
            &mut actions,
            0,
        );
        self.instances.insert(actor, instance);
        result?;
        Ok(actions)
    }

    fn find_function(
        &mut self,
        mut class: ResolvedObject,
        name: &str,
        mut depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        loop {
            if depth >= MAX_CALL_DEPTH {
                return Err(DispatchError::CallDepth);
            }
            if let Some(export_index) = class.package.summary().exports.iter().position(|export| {
                export.outer == ObjectReference::Export(class.export_index)
                    && class
                        .package
                        .summary()
                        .class_name(export)
                        .is_some_and(|class| class.eq_ignore_ascii_case("Function"))
                    && class
                        .package
                        .summary()
                        .name(export.object_name)
                        .eq_ignore_ascii_case(name)
            }) {
                return Ok(Some(ResolvedObject {
                    package: class.package,
                    export_index,
                }));
            }

            let Some(base) = self.base_class(&class)? else {
                return Ok(None);
            };
            class = base;
            depth += 1;
        }
    }

    fn base_class(&mut self, class: &ResolvedObject) -> DispatchResult<Option<ResolvedObject>> {
        let metadata = ScriptExport::decode(&class.package, class.export_index)?;
        if !matches!(metadata.metadata, ScriptMetadata::Class(_)) {
            return Err(DispatchError::InvalidClass {
                export_index: class.export_index,
            });
        }
        Ok(self.packages.resolve(&class.package, metadata.base_field)?)
    }

    fn find_actor_function(
        &mut self,
        actor: usize,
        class: ResolvedObject,
        name: &str,
        depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        if let Some(state) = self.actor_states.get(&actor).and_then(Clone::clone)
            && let Some(function) = self.find_state_function(
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                &state,
                name,
                depth,
            )?
        {
            return Ok(Some(function));
        }
        self.find_function(class, name, depth)
    }

    fn find_state_function(
        &mut self,
        mut class: ResolvedObject,
        state_name: &str,
        function_name: &str,
        mut depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        loop {
            if depth >= MAX_CALL_DEPTH {
                return Err(DispatchError::CallDepth);
            }
            let summary = class.package.summary();
            if let Some(state) = summary.exports.iter().position(|export| {
                export.outer == ObjectReference::Export(class.export_index)
                    && summary
                        .class_name(export)
                        .is_some_and(|name| name.eq_ignore_ascii_case("State"))
                    && summary
                        .name(export.object_name)
                        .eq_ignore_ascii_case(state_name)
            }) && let Some(function) = summary.exports.iter().position(|export| {
                export.outer == ObjectReference::Export(state)
                    && summary
                        .class_name(export)
                        .is_some_and(|name| name.eq_ignore_ascii_case("Function"))
                    && summary
                        .name(export.object_name)
                        .eq_ignore_ascii_case(function_name)
            }) {
                return Ok(Some(ResolvedObject {
                    package: class.package,
                    export_index: function,
                }));
            }

            let Some(base) = self.base_class(&class)? else {
                return Ok(None);
            };
            class = base;
            depth += 1;
        }
    }

    fn find_state(
        &mut self,
        class: &ResolvedObject,
        name: &str,
    ) -> DispatchResult<Option<ResolvedObject>> {
        if name.eq_ignore_ascii_case("Auto")
            && let Some(state) = self.find_matching_state(
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                None,
                0,
            )?
        {
            return Ok(Some(state));
        }
        self.find_matching_state(
            ResolvedObject {
                package: Arc::clone(&class.package),
                export_index: class.export_index,
            },
            Some(name),
            0,
        )
    }

    fn state_ignores_event(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        event: &str,
    ) -> DispatchResult<bool> {
        let Some(index) = probe_event_index(event) else {
            return Ok(false);
        };
        let Some(state_name) = self.actor_states.get(&actor).and_then(Clone::clone) else {
            return Ok(false);
        };
        let Some(state) = self.find_state(class, &state_name)? else {
            return Ok(false);
        };
        let metadata = ScriptExport::decode(&state.package, state.export_index)?;
        Ok(matches!(
            metadata.metadata,
            ScriptMetadata::State(state) if state.ignore_mask & (1_u64 << index) == 0
        ))
    }

    fn find_matching_state(
        &mut self,
        mut class: ResolvedObject,
        name: Option<&str>,
        mut depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        loop {
            if depth >= MAX_CALL_DEPTH {
                return Err(DispatchError::CallDepth);
            }
            let states = class
                .package
                .summary()
                .exports
                .iter()
                .enumerate()
                .filter(|(_, export)| {
                    export.outer == ObjectReference::Export(class.export_index)
                        && class
                            .package
                            .summary()
                            .class_name(export)
                            .is_some_and(|name| name.eq_ignore_ascii_case("State"))
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            for export_index in states {
                let state_name = class
                    .package
                    .summary()
                    .name(class.package.summary().exports[export_index].object_name);
                let matches = match name {
                    Some(name) => state_name.eq_ignore_ascii_case(name),
                    None => matches!(
                        ScriptExport::decode(&class.package, export_index)?.metadata,
                        ScriptMetadata::State(state) if state.flags & STATE_AUTO != 0
                    ),
                };
                if matches {
                    return Ok(Some(ResolvedObject {
                        package: class.package,
                        export_index,
                    }));
                }
            }

            let Some(base) = self.base_class(&class)? else {
                return Ok(None);
            };
            class = base;
            depth += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let script = ScriptExport::decode(&function.package, function.export_index)?;
        if let ScriptMetadata::Function(metadata) = &script.metadata
            && script.bytecode.bytes.is_empty()
            && metadata.native_index != 0
        {
            return self
                .native(
                    actor,
                    actor_class,
                    &function.package,
                    metadata.native_index,
                    arguments,
                    instance,
                    actions,
                )
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(metadata.native_index),
                    message,
                })
                .map_err(Into::into);
        }

        let mut frame = Frame::new(&script.bytecode);
        self.bind_struct_members(&function.package, &script.bytecode, &mut frame)?;
        self.bind_frame_arguments(&function.package, &script, arguments, &mut frame)?;
        let mut frame_instance = HashMap::new();
        self.load_frame_instance(
            &function.package,
            &script.bytecode,
            instance,
            &mut frame_instance,
        )?;
        let result = frame.execute_with_instance(&mut frame_instance, |request, frame_instance| {
            self.store_frame_instance(&function.package, frame_instance, instance)
                .map_err(|error| error.to_string())?;
            let result = match request {
                FrameRequest::Call {
                    receiver,
                    function: call,
                    arguments,
                } => self
                    .dispatch_context_call(
                        actor,
                        actor_class,
                        receiver,
                        &function.package,
                        call,
                        &arguments,
                        instance,
                        actions,
                        depth + 1,
                    )
                    .map(FrameResponse::Value),
                FrameRequest::CallIterator {
                    receiver,
                    function: call,
                    arguments,
                } => self
                    .dispatch_iterator_call(
                        actor,
                        receiver,
                        &function.package,
                        call,
                        &arguments,
                        instance,
                    )
                    .map(FrameResponse::Iterator),
                FrameRequest::GetInstance { receiver, field } => self
                    .context_field_value(actor, receiver, &function.package, field, instance)
                    .map(FrameResponse::Value),
                FrameRequest::SetInstance {
                    receiver,
                    field,
                    value,
                } => self
                    .set_context_field(actor, receiver, &function.package, field, value, instance)
                    .map(|()| FrameResponse::Value(Value::None)),
            };
            self.load_frame_instance(
                &function.package,
                &script.bytecode,
                instance,
                frame_instance,
            )
            .map_err(|error| error.to_string())?;
            result.map_err(|error| error.to_string())
        });
        self.store_frame_instance(&function.package, &frame_instance, instance)?;
        result.map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        match call {
            FunctionCall::Native(index) => self
                .native(
                    actor,
                    actor_class,
                    source,
                    index,
                    arguments,
                    instance,
                    actions,
                )
                .map_err(|message| crate::Error::Call { call, message }.into()),
            FunctionCall::Final(index) => {
                let reference = object_reference(index);
                let Some(function) = self.packages.resolve(source, reference)? else {
                    return Ok(Value::None);
                };
                match self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                ) {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        // ponytail: keep bootstrapping the subclass while the VM is
                        // incomplete; remove this deferral once the corpus executes.
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: error.to_string(),
                        });
                        Ok(Value::None)
                    }
                }
            }
            FunctionCall::Virtual(name) | FunctionCall::Global(name) => {
                let Some(name) = usize::try_from(name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .map(|name| source.summary().name(name).to_owned())
                else {
                    return Err(crate::Error::Call {
                        call,
                        message: "invalid function name".to_owned(),
                    }
                    .into());
                };
                let class = ResolvedObject {
                    package: Arc::clone(&actor_class.package),
                    export_index: actor_class.export_index,
                };
                let function = if matches!(call, FunctionCall::Virtual(_)) {
                    self.find_actor_function(actor, class, &name, depth)?
                } else {
                    self.find_function(class, &name, depth)?
                };
                let Some(function) = function else {
                    return Ok(Value::None);
                };
                self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_context_call(
        &mut self,
        current_actor: usize,
        current_class: &ResolvedObject,
        receiver: i32,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        current_instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        if receiver == -1 {
            return self.dispatch_call(
                current_actor,
                current_class,
                source,
                call,
                arguments,
                current_instance,
                actions,
                depth,
            );
        }
        let actor = self.actor_for_handle(receiver)?;
        if actor == current_actor {
            return self.dispatch_call(
                current_actor,
                current_class,
                source,
                call,
                arguments,
                current_instance,
                actions,
                depth,
            );
        }
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::InvalidActorHandle { handle: receiver })?;
        let class = ResolvedObject {
            package: self.packages.load_path(Path::new(class.package.as_ref()))?,
            export_index: class.export_index,
        };
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = self.dispatch_call(
            actor,
            &class,
            source,
            call,
            arguments,
            &mut instance,
            actions,
            depth,
        );
        self.instances.insert(actor, instance);
        result
    }

    fn dispatch_iterator_call(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<Value>> {
        if receiver != -1 {
            self.actor_for_handle(receiver)?;
        }
        let FunctionCall::Native(ALL_ACTORS) = call else {
            return Err(crate::Error::Call {
                call,
                message: "iterator function is not implemented".to_owned(),
            }
            .into());
        };
        let [Value::Object(base_class), Value::None, rest @ ..] = arguments else {
            return Err(crate::Error::Call {
                call,
                message: format!(
                    "AllActors expects a class and output actor, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
            .into());
        };
        if rest.len() > 1 {
            return Err(crate::Error::Call {
                call,
                message: format!(
                    "AllActors expects at most 3 arguments, found {}",
                    arguments.len()
                ),
            }
            .into());
        }
        let base_class = self
            .packages
            .resolve(source, object_reference(*base_class))?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "AllActors base class is null".to_owned(),
            })?;
        let match_tag = rest
            .first()
            .filter(|value| !matches!(value, Value::None))
            .map(|value| runtime_name(source, value))
            .transpose()
            .map_err(|message| DispatchError::UnresolvedObject { message })?
            .filter(|tag| !tag.eq_ignore_ascii_case("None"));

        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        let tag_field = match match_tag {
            Some(_) => Some(self.find_property(&base_class, "Tag", 0)?.ok_or_else(|| {
                DispatchError::UnresolvedObject {
                    message: "Actor.Tag is missing".to_owned(),
                }
            })?),
            None => None,
        };
        let mut values = Vec::new();
        for actor in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = ResolvedObject {
                package: self.packages.load_path(Path::new(class.package.as_ref()))?,
                export_index: class.export_index,
            };
            if !self.class_is_a(class, &base_class)? {
                continue;
            }
            if let (Some(match_tag), Some(field)) = (&match_tag, &tag_field) {
                let instance = if actor == current_actor {
                    current_instance
                } else {
                    self.instances
                        .get(&actor)
                        .ok_or(DispatchError::ActiveActorContext { actor })?
                };
                if !matches!(
                    instance.get(field),
                    Some(StoredValue::Name(tag)) if tag.eq_ignore_ascii_case(match_tag)
                ) {
                    continue;
                }
            }
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            values.push(Value::Object(self.object_handle(object)?));
        }
        Ok(values)
    }

    fn class_is_a(
        &mut self,
        mut class: ResolvedObject,
        base: &ResolvedObject,
    ) -> DispatchResult<bool> {
        let base = object_id(&base.package, base.export_index);
        let key = (object_id(&class.package, class.export_index), base.clone());
        if let Some(result) = self.class_relations.get(&key) {
            return Ok(*result);
        }
        for _ in 0..MAX_CALL_DEPTH {
            if object_id(&class.package, class.export_index) == base {
                self.class_relations.insert(key, true);
                return Ok(true);
            }
            let Some(parent) = self.base_class(&class)? else {
                self.class_relations.insert(key, false);
                return Ok(false);
            };
            class = parent;
        }
        Err(DispatchError::CallDepth)
    }

    fn context_field_value(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        field: i32,
        current_instance: &InstanceState,
    ) -> DispatchResult<Value> {
        let actor = self.actor_for_handle(receiver)?;
        let Some(resolved) = self.packages.resolve(source, object_reference(field))? else {
            return Ok(Value::None);
        };
        let id = object_id(&resolved.package, resolved.export_index);
        let value = if actor == current_actor {
            current_instance.get(&id).cloned()
        } else {
            self.instances
                .get(&actor)
                .ok_or(DispatchError::ActiveActorContext { actor })?
                .get(&id)
                .cloned()
        };
        match value {
            Some(value) => self.frame_value(&value),
            None => Ok(self.zero_field_value(&resolved)?.unwrap_or(Value::None)),
        }
    }

    fn set_context_field(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        field: i32,
        value: Value,
        current_instance: &mut InstanceState,
    ) -> DispatchResult<()> {
        let actor = self.actor_for_handle(receiver)?;
        let Some(field) = self.resolve_field(source, field)? else {
            return Ok(());
        };
        let value = self.stored_value(source, &value)?;
        if actor == current_actor {
            current_instance.insert(field, value);
        } else {
            self.instances
                .get_mut(&actor)
                .ok_or(DispatchError::ActiveActorContext { actor })?
                .insert(field, value);
        }
        Ok(())
    }

    fn actor_for_handle(&self, handle: i32) -> DispatchResult<usize> {
        let index = usize::try_from(handle - 1)
            .ok()
            .filter(|index| *index < self.handle_objects.len())
            .ok_or(DispatchError::InvalidObjectHandle { handle })?;
        self.object_actors
            .get(&self.handle_objects[index])
            .copied()
            .ok_or(DispatchError::InvalidActorHandle { handle })
    }

    #[allow(clippy::too_many_arguments)]
    fn native(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        index: u16,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        if matches!(index, ENABLE | DISABLE) {
            let [event] = arguments else {
                return Err(format!(
                    "{} expects one name, found {}",
                    if index == ENABLE { "Enable" } else { "Disable" },
                    arguments.len()
                ));
            };
            let event = runtime_name(source, event)?;
            let state = self
                .actor_states
                .get(&actor)
                .and_then(|state| state.as_deref());
            set_event_disabled(
                &mut self.disabled_events,
                actor,
                state,
                &event,
                index == DISABLE,
            );
            return Ok(Value::None);
        }
        if index == DESTROY {
            for name in ["bStatic", "bNoDelete"] {
                let field = self
                    .find_property(actor_class, name, 0)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("Destroy property {name} is missing"))?;
                match instance.get(&field) {
                    Some(StoredValue::Value(Value::Bool(true))) => {
                        return Ok(Value::Bool(false));
                    }
                    Some(StoredValue::Value(Value::Bool(false))) | None => {}
                    Some(value) => {
                        return Err(format!("Destroy property {name} is {value:?}"));
                    }
                }
            }
            if !self.destroyed.insert(actor) {
                return Ok(Value::Bool(true));
            }
            let field = self
                .find_property(actor_class, "bDeleteMe", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Destroy property bDeleteMe is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Bool(true)));
            self.timers.remove(&actor);
            actions.push(ActorAction::DestroyActor { actor });
            return Ok(Value::Bool(true));
        }
        if index == GOTO_STATE {
            if arguments.len() > 2 {
                return Err(format!(
                    "GotoState expects at most 2 arguments, found {}",
                    arguments.len()
                ));
            }
            let state = match arguments.first() {
                Some(Value::None) | None => self
                    .actor_states
                    .get(&actor)
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| "None".to_owned()),
                Some(state) => runtime_name(source, state)?,
            };
            if let Some(label) = arguments.get(1)
                && !matches!(label, Value::None)
            {
                runtime_name(source, label)?;
            }
            let state = if state.eq_ignore_ascii_case("None") {
                None
            } else {
                self.find_state(actor_class, &state)
                    .map_err(|error| error.to_string())?
                    .map(|state| {
                        state
                            .package
                            .summary()
                            .name(state.package.summary().exports[state.export_index].object_name)
                            .to_owned()
                    })
            };
            self.actor_states.insert(actor, state);
            return Ok(Value::None);
        }
        if index == LOOP_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let rate = match rest.first() {
                Some(Value::Float(rate)) => *rate,
                Some(Value::None) | None => 1.0,
                Some(value) => {
                    return Err(format!("LoopAnim rate is {}", value.kind()));
                }
            };
            actions.push(ActorAction::LoopAnimation {
                actor,
                sequence: name,
                rate,
            });
            return Ok(Value::None);
        }
        if matches!(index, 0xfe | 0xff)
            && let [left, right] = arguments
        {
            let equal =
                runtime_name(source, left)?.eq_ignore_ascii_case(&runtime_name(source, right)?);
            return Ok(Value::Bool(equal == (index == 0xfe)));
        }
        if index == SET_TIMER
            && let [Value::Float(rate), rest @ ..] = arguments
        {
            if !rate.is_finite() {
                return Err("SetTimer rate is not finite".to_owned());
            }
            if *rate <= 0.0 {
                self.timers.remove(&actor);
                return Ok(Value::None);
            }
            let looping = match rest.first() {
                Some(Value::Bool(looping)) => *looping,
                Some(Value::None) | None => false,
                Some(value) => return Err(format!("SetTimer loop flag is {}", value.kind())),
            };
            self.timers.insert(
                actor,
                ActorTimer {
                    remaining: *rate,
                    rate: *rate,
                    looping,
                },
            );
            return Ok(Value::None);
        }
        if index == SET_COLLISION {
            for (name, value) in ["bCollideActors", "bBlockActors", "bBlockPlayers"]
                .into_iter()
                .zip(collision_updates(arguments)?)
            {
                let Some(value) = value else {
                    continue;
                };
                let field = self
                    .find_property(actor_class, name, 0)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("SetCollision property {name} is missing"))?;
                instance.insert(field, StoredValue::Value(Value::Bool(value)));
            }
            // ponytail: these flags become collision behavior when BSP movement exists.
            return Ok(Value::None);
        }
        if index == SET_LOCATION {
            let [Value::Vector(location)] = arguments else {
                return Err(format!(
                    "SetLocation expects one vector, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !location.iter().all(|value| value.is_finite()) {
                return Err("SetLocation coordinates are not finite".to_owned());
            }
            let field = self
                .find_property(actor_class, "Location", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SetLocation property Location is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Vector(*location)));
            actions.push(ActorAction::SetLocation {
                actor,
                location: *location,
            });
            // ponytail: accept finite locations until UE1 BSP collision rejection exists.
            return Ok(Value::Bool(true));
        }
        scalar_native(index, arguments)
    }

    fn load_class_defaults(
        &mut self,
        class: &ResolvedObject,
        depth: usize,
    ) -> DispatchResult<InstanceState> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let id = object_id(&class.package, class.export_index);
        if let Some(defaults) = self.class_defaults.get(&id) {
            return Ok(defaults.clone());
        }
        let (metadata, mut reader) = class_defaults_reader(&class.package, class.export_index)?;
        let mut defaults = match self.packages.resolve(&class.package, metadata.base_field)? {
            Some(base) => self.load_class_defaults(&base, depth + 1)?,
            None => InstanceState::new(),
        };
        self.apply_properties(class, &class.package, &mut reader, &mut defaults)?;
        self.class_defaults.insert(id, defaults.clone());
        Ok(defaults)
    }

    fn apply_properties(
        &mut self,
        class: &ResolvedObject,
        source: &Arc<Package>,
        reader: &mut ObjectReader<'_>,
        instance: &mut InstanceState,
    ) -> DispatchResult<()> {
        while let Some(property) = reader.next_property()? {
            if property.array_index.is_some() {
                continue;
            }
            let name = reader.summary().name(property.name).to_owned();
            let Some(value) = self.read_property(source, reader, &property)? else {
                continue;
            };
            let Some(field) = self.find_property(class, &name, 0)? else {
                continue;
            };
            instance.insert(field, value);
        }
        Ok(())
    }

    fn read_property(
        &mut self,
        source: &Arc<Package>,
        reader: &ObjectReader<'_>,
        property: &openhp1_package::PropertyTag,
    ) -> DispatchResult<Option<StoredValue>> {
        let mut value = reader.property_reader(property);
        Ok(Some(match property.kind {
            PropertyKind::Byte => StoredValue::Value(Value::Byte(value.read_u8()?)),
            PropertyKind::Int => StoredValue::Value(Value::Int(value.read_i32()?)),
            PropertyKind::Bool => {
                StoredValue::Value(Value::Bool(property.bool_value.unwrap_or(false)))
            }
            PropertyKind::Float => StoredValue::Value(Value::Float(value.read_f32()?)),
            PropertyKind::Object | PropertyKind::Class => {
                let reference = value.read_object_reference()?;
                match self.packages.resolve(source, reference) {
                    Ok(object) => StoredValue::Object(
                        object.map(|object| object_id(&object.package, object.export_index)),
                    ),
                    Err(error) => StoredValue::UnresolvedObject(error.to_string()),
                }
            }
            PropertyKind::Name => {
                let name = value.read_name_index("runtime name property")?;
                StoredValue::Name(value.summary().name(name).to_owned())
            }
            PropertyKind::String | PropertyKind::Str => {
                StoredValue::Value(Value::String(value.read_string()?))
            }
            PropertyKind::Vector => StoredValue::Value(Value::Vector([
                value.read_f32()?,
                value.read_f32()?,
                value.read_f32()?,
            ])),
            PropertyKind::Rotator => StoredValue::Value(Value::Rotator([
                value.read_i32()?,
                value.read_i32()?,
                value.read_i32()?,
            ])),
            PropertyKind::Struct
                if property
                    .struct_name
                    .is_some_and(|name| value.summary().name(name) == "Vector") =>
            {
                StoredValue::Value(Value::Vector([
                    value.read_f32()?,
                    value.read_f32()?,
                    value.read_f32()?,
                ]))
            }
            PropertyKind::Struct
                if property
                    .struct_name
                    .is_some_and(|name| value.summary().name(name) == "Rotator") =>
            {
                StoredValue::Value(Value::Rotator([
                    value.read_i32()?,
                    value.read_i32()?,
                    value.read_i32()?,
                ]))
            }
            _ => return Ok(None),
        }))
    }

    fn find_property(
        &mut self,
        class: &ResolvedObject,
        name: &str,
        mut depth: usize,
    ) -> DispatchResult<Option<ObjectId>> {
        let key = (
            object_id(&class.package, class.export_index),
            name.to_ascii_lowercase(),
        );
        if let Some(field) = self.fields.get(&key) {
            return Ok(field.clone());
        }
        let mut current = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        let field = loop {
            if depth >= MAX_CALL_DEPTH {
                return Err(DispatchError::CallDepth);
            }
            if let Some(export_index) =
                current.package.summary().exports.iter().position(|export| {
                    export.outer == ObjectReference::Export(current.export_index)
                        && current
                            .package
                            .summary()
                            .class_name(export)
                            .is_some_and(|class| class.ends_with("Property"))
                        && current
                            .package
                            .summary()
                            .name(export.object_name)
                            .eq_ignore_ascii_case(name)
                })
            {
                break Some(object_id(&current.package, export_index));
            }
            let metadata = ScriptExport::decode(&current.package, current.export_index)?;
            let Some(base) = self
                .packages
                .resolve(&current.package, metadata.base_field)?
            else {
                break None;
            };
            current = base;
            depth += 1;
        };
        self.fields.insert(key, field.clone());
        Ok(field)
    }

    fn load_frame_instance(
        &mut self,
        source: &Arc<Package>,
        bytecode: &Bytecode,
        instance: &InstanceState,
        frame: &mut HashMap<i32, Value>,
    ) -> DispatchResult<()> {
        frame.clear();
        for field in instance_fields(bytecode) {
            let Some(resolved) = self.packages.resolve(source, object_reference(field))? else {
                continue;
            };
            let id = object_id(&resolved.package, resolved.export_index);
            let value = match instance.get(&id) {
                Some(value) => Some(self.frame_value(value)?),
                None => self.zero_field_value(&resolved)?,
            };
            if let Some(value) = value {
                frame.insert(field, value);
            }
        }
        Ok(())
    }

    fn store_frame_instance(
        &mut self,
        source: &Arc<Package>,
        frame: &HashMap<i32, Value>,
        instance: &mut InstanceState,
    ) -> DispatchResult<()> {
        for (&field, value) in frame {
            let Some(id) = self.resolve_field(source, field)? else {
                continue;
            };
            instance.insert(id, self.stored_value(source, value)?);
        }
        Ok(())
    }

    fn frame_value(&mut self, value: &StoredValue) -> DispatchResult<Value> {
        Ok(match value {
            StoredValue::Value(value) => value.clone(),
            StoredValue::Name(name) => Value::NameText(name.clone()),
            StoredValue::Object(None) => Value::Object(0),
            StoredValue::Object(Some(object)) => Value::Object(self.object_handle(object.clone())?),
            StoredValue::UnresolvedObject(message) => {
                return Err(DispatchError::UnresolvedObject {
                    message: message.clone(),
                });
            }
            StoredValue::SelfObject => Value::Object(-1),
        })
    }

    fn stored_value(&self, source: &Arc<Package>, value: &Value) -> DispatchResult<StoredValue> {
        Ok(match value {
            Value::Name(name) => {
                let name = usize::try_from(*name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .ok_or_else(|| DispatchError::MissingName {
                        package: Arc::clone(&source.summary().source),
                        name: format!("#{name}"),
                    })?;
                StoredValue::Name(source.summary().name(name).to_owned())
            }
            Value::NameText(name) => StoredValue::Name(name.clone()),
            Value::Object(0) => StoredValue::Object(None),
            Value::Object(-1) => StoredValue::SelfObject,
            Value::Object(handle) => {
                let index = usize::try_from(*handle - 1)
                    .ok()
                    .filter(|index| *index < self.handle_objects.len())
                    .ok_or(DispatchError::InvalidObjectHandle { handle: *handle })?;
                StoredValue::Object(Some(self.handle_objects[index].clone()))
            }
            value => StoredValue::Value(value.clone()),
        })
    }

    fn zero_field_value(&mut self, field: &ResolvedObject) -> DispatchResult<Option<Value>> {
        let class = field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index])
            .unwrap_or("<unknown>");
        let value = match class {
            "ByteProperty" => Value::Byte(0),
            "IntProperty" => Value::Int(0),
            "BoolProperty" => Value::Bool(false),
            "FloatProperty" => Value::Float(0.0),
            "ObjectProperty" | "ClassProperty" => Value::Object(0),
            "NameProperty" => Value::NameText("None".to_owned()),
            "StrProperty" | "StringProperty" => Value::String(String::new()),
            "StructProperty" => {
                let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
                let Some(struct_type) = metadata.struct_type else {
                    return Ok(None);
                };
                let Some(struct_type) = self.packages.resolve(&field.package, struct_type)? else {
                    return Ok(None);
                };
                let name = struct_type.package.summary().name(
                    struct_type.package.summary().exports[struct_type.export_index].object_name,
                );
                match name {
                    "Vector" => Value::Vector([0.0; 3]),
                    "Rotator" => Value::Rotator([0; 3]),
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn object_handle(&mut self, object: ObjectId) -> DispatchResult<i32> {
        if let Some(handle) = self.object_handles.get(&object) {
            return Ok(*handle);
        }
        let handle =
            i32::try_from(self.handle_objects.len() + 1).map_err(|_| DispatchError::ObjectLimit)?;
        self.handle_objects.push(object.clone());
        self.object_handles.insert(object, handle);
        Ok(handle)
    }

    fn resolve_field(
        &mut self,
        source: &Arc<Package>,
        field: i32,
    ) -> DispatchResult<Option<ObjectId>> {
        Ok(self
            .packages
            .resolve(source, object_reference(field))?
            .map(|field| ObjectId {
                package: Arc::clone(&field.package.summary().source),
                export_index: field.export_index,
            }))
    }

    fn bind_struct_members(
        &mut self,
        source: &Arc<Package>,
        bytecode: &Bytecode,
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        for field in fields(bytecode, 0x36) {
            let Some(resolved) = self.packages.resolve(source, object_reference(field))? else {
                continue;
            };
            let summary = resolved.package.summary();
            let export = &summary.exports[resolved.export_index];
            let Some(owner) = summary.object_name(export.outer) else {
                continue;
            };
            let name = summary.name(export.object_name);
            let member = match (owner, name) {
                ("Vector", "X") => StructMember::X,
                ("Vector", "Y") => StructMember::Y,
                ("Vector", "Z") => StructMember::Z,
                ("Rotator", "Pitch") => StructMember::Pitch,
                ("Rotator", "Yaw") => StructMember::Yaw,
                ("Rotator", "Roll") => StructMember::Roll,
                _ => continue,
            };
            frame.set_struct_member(field, member);
        }
        Ok(())
    }

    fn bind_frame_arguments(
        &mut self,
        source: &Arc<Package>,
        function: &ScriptExport,
        arguments: &[Value],
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        let parameters = self.function_parameters(source, function.children)?;
        let arguments = parameters.iter().zip(arguments).collect::<HashMap<_, _>>();
        for field in local_fields(&function.bytecode) {
            let Some(id) = self.resolve_field(source, field)? else {
                continue;
            };
            if let Some(value) = arguments.get(&id) {
                frame.set_local(field, (*value).clone());
            }
        }
        Ok(())
    }

    fn function_parameters(
        &mut self,
        source: &Arc<Package>,
        mut field: ObjectReference,
    ) -> DispatchResult<Vec<ObjectId>> {
        let mut parameters = Vec::new();
        let mut field_source = Arc::clone(source);
        for _ in 0..MAX_CALL_DEPTH {
            let Some(resolved) = self.packages.resolve(&field_source, field)? else {
                return Ok(parameters);
            };
            let metadata = PropertyMetadata::decode(&resolved.package, resolved.export_index)?;
            if metadata.flags & PROPERTY_PARAMETER != 0 && metadata.flags & PROPERTY_RETURN == 0 {
                parameters.push(ObjectId {
                    package: Arc::clone(&resolved.package.summary().source),
                    export_index: resolved.export_index,
                });
            }
            field = metadata.next_field;
            field_source = resolved.package;
            if field == ObjectReference::None {
                return Ok(parameters);
            }
        }
        Err(DispatchError::CallDepth)
    }
}

fn local_fields(bytecode: &Bytecode) -> impl Iterator<Item = i32> + '_ {
    fields(bytecode, 0x00)
}

fn advance_timer(timer: &mut ActorTimer, delta_time: f32) -> bool {
    timer.remaining -= delta_time;
    if timer.remaining > 0.0 {
        return false;
    }
    if timer.looping {
        // ponytail: one callback per rendered frame; add catch-up callbacks
        // if sub-frame timer fidelity becomes observable.
        timer.remaining = timer.rate - (-timer.remaining).rem_euclid(timer.rate);
    }
    true
}

fn instance_fields(bytecode: &Bytecode) -> impl Iterator<Item = i32> + '_ {
    fields(bytecode, 0x01)
}

fn fields(bytecode: &Bytecode, opcode: u8) -> impl Iterator<Item = i32> + '_ {
    bytecode
        .tokens
        .iter()
        .filter(move |token| token.opcode == opcode)
        .filter_map(|token| {
            bytecode
                .bytes
                .get(token.offset + 1..token.offset + 5)
                .and_then(|bytes| bytes.try_into().ok())
                .map(i32::from_le_bytes)
        })
}

fn runtime_name(source: &Package, value: &Value) -> std::result::Result<String, String> {
    match value {
        Value::Name(index) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < source.summary().names.len())
            .map(|index| source.summary().name(index).to_owned())
            .ok_or_else(|| format!("invalid name index {index}")),
        Value::NameText(name) => Ok(name.clone()),
        value => Err(format!("expected name, found {}", value.kind())),
    }
}

fn probe_event_index(event: &str) -> Option<usize> {
    PROBE_EVENTS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(event))
}

fn event_key(actor: usize, state: Option<&str>) -> (usize, String) {
    (actor, state.unwrap_or_default().to_ascii_lowercase())
}

fn set_event_disabled(
    disabled_events: &mut HashMap<(usize, String), HashSet<String>>,
    actor: usize,
    state: Option<&str>,
    event: &str,
    disabled: bool,
) {
    let events = disabled_events.entry(event_key(actor, state)).or_default();
    let event = event.to_ascii_lowercase();
    if disabled {
        events.insert(event);
    } else {
        events.remove(&event);
    }
}

fn event_disabled(
    disabled_events: &HashMap<(usize, String), HashSet<String>>,
    actor: usize,
    state: Option<&str>,
    event: &str,
) -> bool {
    disabled_events
        .get(&event_key(actor, state))
        .is_some_and(|events| events.contains(&event.to_ascii_lowercase()))
}

fn collision_updates(arguments: &[Value]) -> std::result::Result<[Option<bool>; 3], String> {
    if arguments.len() > 3 {
        return Err(format!(
            "SetCollision expects at most 3 arguments, found {}",
            arguments.len()
        ));
    }
    let mut updates = [None; 3];
    for (update, argument) in updates.iter_mut().zip(arguments) {
        match argument {
            Value::Bool(value) => *update = Some(*value),
            Value::None => {}
            value => return Err(format!("SetCollision flag is {}", value.kind())),
        }
    }
    Ok(updates)
}

fn scalar_native(index: u16, arguments: &[Value]) -> std::result::Result<Value, String> {
    if index == 0xf5 {
        let [Value::Float(left), Value::Float(right)] = arguments else {
            return Err(format!(
                "FMax expects two floats, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        return Ok(Value::Float(if left < right { *right } else { *left }));
    }
    if matches!(index, 0x72 | 0x77)
        && let [left, right] = arguments
        && let (Some(left), Some(right)) = (object_value(left), object_value(right))
    {
        return Ok(Value::Bool((left == right) == (index == 0x72)));
    }
    if index == 0x91
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return left
            .checked_div(*right)
            .map(Value::Int)
            .ok_or_else(|| "integer division by zero or overflow".to_owned());
    }
    Ok(match (index, arguments) {
        (0x81, [value]) => Value::Bool(!value.truthy().map_err(|error| error.to_string())?),
        (0x82, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                && right.truthy().map_err(|error| error.to_string())?,
        ),
        (0x84, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                || right.truthy().map_err(|error| error.to_string())?,
        ),
        (0x90, [Value::Int(left), Value::Int(right)]) => Value::Int(left * right),
        (0x92, [Value::Int(left), Value::Int(right)]) => Value::Int(left + right),
        (0x93, [Value::Int(left), Value::Int(right)]) => Value::Int(left - right),
        (0x96, [Value::Int(left), Value::Int(right)]) => Value::Bool(left < right),
        (0x97, [Value::Int(left), Value::Int(right)]) => Value::Bool(left > right),
        (0x9a, [Value::Int(left), Value::Int(right)]) => Value::Bool(left == right),
        (0x9b, [Value::Int(left), Value::Int(right)]) => Value::Bool(left != right),
        (0x7a, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (0x7b, [Value::String(left), Value::String(right)]) => {
            Value::Bool(!left.eq_ignore_ascii_case(right))
        }
        (0xa9, [Value::Float(value)]) => Value::Float(-value),
        (0xab, [Value::Float(left), Value::Float(right)]) => Value::Float(left * right),
        (0xac, [Value::Float(left), Value::Float(right)]) => Value::Float(left / right),
        (0xae, [Value::Float(left), Value::Float(right)]) => Value::Float(left + right),
        (0xaf, [Value::Float(left), Value::Float(right)]) => Value::Float(left - right),
        (0xb0, [Value::Float(left), Value::Float(right)]) => Value::Bool(left < right),
        (0xb1, [Value::Float(left), Value::Float(right)]) => Value::Bool(left > right),
        (0xd3, [Value::Vector(value)]) => Value::Vector([-value[0], -value[1], -value[2]]),
        (0xd4, [Value::Vector(value), Value::Float(scale)])
        | (0xd5, [Value::Float(scale), Value::Vector(value)]) => {
            Value::Vector([value[0] * scale, value[1] * scale, value[2] * scale])
        }
        (0xd6, [Value::Vector(value), Value::Float(divisor)]) => {
            Value::Vector([value[0] / divisor, value[1] / divisor, value[2] / divisor])
        }
        (0xd7, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] + right[0], left[1] + right[1], left[2] + right[2]])
        }
        (0xd8, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] - right[0], left[1] - right[1], left[2] - right[2]])
        }
        (0xe1, [Value::Vector(value)]) => {
            Value::Float((value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt())
        }
        _ => return Err(format!("native {index:#05x} is not implemented")),
    })
}

fn object_value(value: &Value) -> Option<i32> {
    match value {
        Value::None => Some(0),
        Value::Object(value) => Some(*value),
        _ => None,
    }
}

fn object_id(package: &Arc<Package>, export_index: usize) -> ObjectId {
    ObjectId {
        package: Arc::clone(&package.summary().source),
        export_index,
    }
}

fn object_reference(index: i32) -> ObjectReference {
    if index == 0 {
        ObjectReference::None
    } else if index > 0 {
        ObjectReference::Export(index as usize - 1)
    } else {
        ObjectReference::Import(index.unsigned_abs() as usize - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looping_timer_keeps_fractional_overshoot() {
        let mut timer = ActorTimer {
            remaining: 0.1,
            rate: 0.1,
            looping: true,
        };
        assert!(advance_timer(&mut timer, 0.15));
        assert!((timer.remaining - 0.05).abs() < 1.0e-6);
        assert!(!advance_timer(&mut timer, 0.04));
        assert!(advance_timer(&mut timer, 0.01));
        assert!((timer.remaining - 0.1).abs() < 1.0e-6);
    }

    #[test]
    fn integer_division_is_checked() {
        assert_eq!(
            scalar_native(0x91, &[Value::Int(7), Value::Int(2)]),
            Ok(Value::Int(3))
        );
        assert!(scalar_native(0x91, &[Value::Int(1), Value::Int(0)]).is_err());
    }

    #[test]
    fn collision_updates_preserve_omitted_flags() {
        assert_eq!(
            collision_updates(&[Value::Bool(true), Value::None]),
            Ok([Some(true), None, None])
        );
        assert!(collision_updates(&[Value::Float(1.0)]).is_err());
    }

    #[test]
    fn disabled_events_are_case_insensitive_and_scoped_to_actor_state() {
        let mut disabled = HashMap::new();
        set_event_disabled(&mut disabled, 2, Some("Beano"), "Tick", true);

        assert!(event_disabled(&disabled, 2, Some("beano"), "TICK"));
        assert!(!event_disabled(&disabled, 2, Some("KillBean"), "Tick"));
        assert!(!event_disabled(&disabled, 3, Some("Beano"), "Tick"));

        set_event_disabled(&mut disabled, 2, Some("BEANO"), "tick", false);
        assert!(!event_disabled(&disabled, 2, Some("Beano"), "Tick"));
        assert_eq!(probe_event_index("tick"), Some(36));
    }

    #[test]
    fn fmax_matches_unreal_native_ordering() {
        assert_eq!(
            scalar_native(0xf5, &[Value::Float(2.0), Value::Float(3.0)]),
            Ok(Value::Float(3.0))
        );
        assert_eq!(
            scalar_native(0xf5, &[Value::Float(2.0), Value::Float(f32::NAN)]),
            Ok(Value::Float(2.0))
        );
    }

    #[test]
    fn basic_vector_arithmetic_matches_unreal_natives() {
        assert_eq!(
            scalar_native(
                0xd7,
                &[
                    Value::Vector([1.0, 2.0, 3.0]),
                    Value::Vector([4.0, 5.0, 6.0])
                ]
            ),
            Ok(Value::Vector([5.0, 7.0, 9.0]))
        );
        assert_eq!(
            scalar_native(0xd4, &[Value::Vector([1.0, 2.0, 3.0]), Value::Float(2.0)]),
            Ok(Value::Vector([2.0, 4.0, 6.0]))
        );
    }
}
