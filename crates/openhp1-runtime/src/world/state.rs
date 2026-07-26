use super::*;

impl ScriptRuntime {
    pub(super) fn find_function(
        &mut self,
        mut class: ResolvedObject,
        name: &str,
        mut depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        let lookup = FunctionLookup::new(
            object_id(&class.package, class.export_index),
            None,
            name,
            depth,
        );
        if let Some(function) = self.function_lookups.get(&lookup).cloned() {
            return function
                .as_ref()
                .map(|function| self.resolved_object(function))
                .transpose();
        }
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
                let function = ResolvedObject {
                    package: class.package,
                    export_index,
                };
                self.function_lookups.insert(
                    lookup,
                    Some(object_id(&function.package, function.export_index)),
                );
                return Ok(Some(function));
            }

            let Some(base) = self.base_class(&class)? else {
                self.function_lookups.insert(lookup, None);
                return Ok(None);
            };
            class = base;
            depth += 1;
        }
    }

    pub(super) fn base_class(
        &mut self,
        class: &ResolvedObject,
    ) -> DispatchResult<Option<ResolvedObject>> {
        let metadata = self.script(class)?;
        if !matches!(metadata.metadata, ScriptMetadata::Class(_)) {
            return Err(DispatchError::InvalidClass {
                export_index: class.export_index,
            });
        }
        Ok(self.packages.resolve(&class.package, metadata.base_field)?)
    }

    pub(super) fn find_actor_function(
        &mut self,
        actor: usize,
        class: ResolvedObject,
        name: &str,
        depth: usize,
    ) -> DispatchResult<Option<ResolvedObject>> {
        let state = self.actor_states.get(&actor).and_then(Clone::clone);
        let lookup = FunctionLookup::new(
            object_id(&class.package, class.export_index),
            state.as_deref(),
            name,
            depth,
        );
        if let Some(function) = self.function_lookups.get(&lookup).cloned() {
            return function
                .as_ref()
                .map(|function| self.resolved_object(function))
                .transpose();
        }
        let function = if let Some(state) = &state
            && let Some(function) = self.find_state_function(
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                state,
                name,
                depth,
            )? {
            Some(function)
        } else {
            self.find_function(class, name, depth)?
        };
        self.function_lookups.insert(
            lookup,
            function
                .as_ref()
                .map(|function| object_id(&function.package, function.export_index)),
        );
        Ok(function)
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

    pub(super) fn find_state(
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

    pub(super) fn refresh_tick_actor(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
    ) -> DispatchResult<()> {
        let function = if self.state_ignores_event(actor, class, "Tick")? {
            None
        } else {
            self.find_actor_function(
                actor,
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                "Tick",
                0,
            )?
        };
        if let Some(function) = function {
            self.tick_functions.insert(actor, function);
        } else {
            self.tick_functions.remove(&actor);
        }
        Ok(())
    }

    pub(super) fn state_ignores_event(
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
}

pub(super) fn probe_event_index(event: &str) -> Option<usize> {
    PROBE_EVENTS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(event))
}

fn event_key(actor: usize, state: Option<&str>) -> (usize, String) {
    (actor, state.unwrap_or_default().to_ascii_lowercase())
}

pub(super) fn set_event_disabled(
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

pub(super) fn event_disabled(
    disabled_events: &HashMap<(usize, String), HashSet<String>>,
    actor: usize,
    state: Option<&str>,
    event: &str,
) -> bool {
    disabled_events
        .get(&event_key(actor, state))
        .is_some_and(|events| events.contains(&event.to_ascii_lowercase()))
}
