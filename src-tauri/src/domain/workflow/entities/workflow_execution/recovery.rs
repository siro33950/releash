use super::*;

impl WorkflowExecution {
    pub fn restore_definition_resolution(
        &mut self,
        resolution: crate::domain::workflow::DefinitionResolution,
    ) {
        self.definition_resolution = resolution;
    }

    pub fn record_recovery_block(&mut self, node_execution_id: &str, reason: String) {
        if let Some(node) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|node| node.id == node_execution_id)
        {
            node.recovery_reason = Some(reason);
            if node.status.is_active() {
                node.status = RuntimeNodeExecutionStatus::Unresolved;
            }
        }
    }

    pub(super) fn definition_error(&self, node_name: &str) -> Option<String> {
        self.definition_resolution
            .node_error(&self.runtime.workflow, node_name)
    }

    pub(super) fn has_unavailable_definitions(&self) -> bool {
        self.definition_resolution.definition_error.is_some()
            || !self.definition_resolution.node_errors.is_empty()
            || !self.definition_resolution.schema_errors.is_empty()
    }

    pub(super) fn start_unavailable_reason(
        &self,
        parent_scope_id: Option<&str>,
        node_name: &str,
        item: Option<&serde_json::Value>,
    ) -> Option<String> {
        if let Some(reason) = self.definition_error(node_name) {
            return Some(reason);
        }
        if !self.has_unavailable_definitions() {
            return None;
        }
        let node = self.runtime.workflow.node_by_name(node_name)?;
        if let Some(spec) = node.fanout() {
            if let Err(error) = self.resolve_fanout_items_for_parent(parent_scope_id, spec) {
                return Some(error.to_string());
            }
        }
        let bindings = self.resolve_child_bindings(parent_scope_id, node, item);
        node.input
            .iter()
            .filter(|input| !bindings.iter().any(|(name, _)| name == &input.name))
            .find_map(|input| self.input_recovery_reason(parent_scope_id, node, &input.name))
    }

    fn input_recovery_reason(
        &self,
        parent_scope_id: Option<&str>,
        node: &NodeDefinition,
        parameter: &str,
    ) -> Option<String> {
        let scope_id = parent_scope_id?;
        let scope = self.scope(scope_id)?;
        let Some(parent) = self.runtime.workflow.node_by_name(&scope.node_name) else {
            return self.definition_error(&scope.node_name);
        };
        let entry = match &scope.kind {
            ScopeRuntimeKind::Sequence(_) => parent.sequence()?.child_entry(&node.name),
            ScopeRuntimeKind::Fanout(_) => parent
                .fanout()?
                .children
                .iter()
                .find(|child| child.name == node.name),
        };
        let source =
            entry.and_then(|entry| entry.inputs.iter().find(|(name, _)| name == parameter));
        let Some((_, source)) = source else {
            return scope
                .fanout()
                .and_then(|_| self.node_execution(scope_id))
                .and_then(|node| node.recovery_reason.clone());
        };
        let reason = if source.root() == workflow_reference::ITEMS_SOURCE {
            self.node_execution(scope_id)
                .and_then(|node| node.recovery_reason.clone())
        } else if parent.input.iter().any(|input| input.name == source.root()) {
            self.input_recovery_reason(scope.parent_scope_id.as_deref(), parent, source.root())
        } else if source.root() == workflow_reference::REQUEST_ARTIFACT {
            None
        } else {
            self.runtime
                .node_executions
                .iter()
                .filter(|execution| {
                    execution.node_name == source.root()
                        && execution
                            .parent
                            .as_ref()
                            .is_some_and(|parent| parent.parent_id == scope_id)
                })
                .max_by_key(|node| node.attempt)
                .and_then(|node| node.recovery_reason.clone())
                .or_else(|| self.definition_error(source.root()))
        };
        reason.map(|reason| {
            format!(
                "Input '{}.{}' is unavailable: {reason}",
                node.name, parameter
            )
        })
    }

    pub(super) fn unavailable_child_artifact(&self, scope_id: &str) -> Option<String> {
        let scope = self.scope(scope_id)?;
        self.runtime
            .node_executions
            .iter()
            .filter(|node| {
                node.parent
                    .as_ref()
                    .is_some_and(|parent| parent.parent_id == scope_id)
            })
            .filter(|node| match &scope.kind {
                ScopeRuntimeKind::Sequence(sequence) => {
                    sequence.child_counts.get(&node.node_name) == Some(&node.attempt)
                }
                ScopeRuntimeKind::Fanout(fanout) => fanout
                    .children
                    .iter()
                    .any(|slot| slot.node_execution_id == node.id),
            })
            .find_map(|node| {
                (matches!(scope.kind, ScopeRuntimeKind::Fanout(_)) || node.artifact.is_none())
                    .then(|| node.recovery_reason.clone())
                    .flatten()
            })
    }

    pub fn resolve_recovery_dependencies(&mut self) {
        if !self.has_unavailable_definitions()
            && self
                .runtime
                .node_executions
                .iter()
                .all(|node| node.recovery_reason.is_none())
        {
            return;
        }
        for advance in self.derive_pending_advances() {
            let (scope_id, reason) = match &advance {
                PendingAdvance::AfterChild {
                    scope_id,
                    child_name,
                } => {
                    let reason = self.scope(scope_id).and_then(|scope| {
                        let sequence = scope.sequence()?;
                        let spec = self
                            .runtime
                            .workflow
                            .node_by_name(&scope.node_name)?
                            .sequence()?;
                        let artifact = sequence
                            .artifacts
                            .get(child_name)
                            .and_then(|value| value.artifact.as_ref());
                        match workflow_routing::route_in_scope(
                            &self.runtime.workflow,
                            spec,
                            child_name,
                            artifact,
                            &sequence.child_counts,
                        ) {
                            Ok(workflow_routing::RouteDecision::TransitionTo(next)) => {
                                self.start_unavailable_reason(Some(scope_id), &next, None)
                            }
                            Ok(workflow_routing::RouteDecision::Completed) => {
                                self.unavailable_child_artifact(scope_id)
                            }
                            Err(error) if self.has_unavailable_definitions() => {
                                Some(error.to_string())
                            }
                            Err(_) => None,
                        }
                    });
                    (scope_id, reason)
                }
                PendingAdvance::StartEntry { scope_id } => {
                    let reason = self
                        .scope(scope_id)
                        .and_then(|scope| self.runtime.workflow.node_by_name(&scope.node_name))
                        .and_then(|node| node.sequence())
                        .and_then(|spec| spec.entry_child_name())
                        .and_then(|name| self.start_unavailable_reason(Some(scope_id), name, None));
                    (scope_id, reason)
                }
                PendingAdvance::ExpandFanout { scope_id } => {
                    (scope_id, self.fanout_recovery_reason(scope_id))
                }
            };
            if let Some(reason) = reason {
                self.record_recovery_block(scope_id, reason);
            }
        }
    }

    fn fanout_recovery_reason(&self, scope_id: &str) -> Option<String> {
        let scope = self.scope(scope_id)?;
        let fanout = scope.fanout()?;
        let spec = self
            .runtime
            .workflow
            .node_by_name(&scope.node_name)?
            .fanout()?;
        let items = fanout
            .items
            .as_ref()
            .map(|items| items.iter().map(Some).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![None]);
        let mut blocked = None;
        for (item_index, item) in items.into_iter().enumerate() {
            for (child_index, entry) in spec.children.iter().enumerate() {
                let occupied = self.runtime.node_executions.iter().any(|node| {
                    node.parent.as_ref().is_some_and(|parent| {
                        parent.parent_id == scope_id
                            && parent.fanout_slot.is_some_and(|slot| {
                                slot.child_index == child_index
                                    && slot.item_index == fanout.items.as_ref().map(|_| item_index)
                            })
                    })
                });
                if occupied {
                    continue;
                }
                let reason = self.start_unavailable_reason(Some(scope_id), &entry.name, item)?;
                blocked = Some(reason);
            }
        }
        blocked
    }
}
