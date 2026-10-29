use super::*;

impl ClientShellState {
    pub(super) fn active_endpoint_workspace_at(&self, point: (u16, u16)) -> Option<String> {
        self.hits
            .workspaces
            .iter()
            .find(|hit| {
                hit.endpoint_id == self.active_endpoint_id && super::contains(hit.rect, point)
            })
            .map(|hit| hit.workspace_id.clone())
    }

    pub(super) fn endpoint_workspace_is_draggable(&self, press: &ClientWorkspacePress) -> bool {
        press.endpoint_id == self.active_endpoint_id
            && self
                .snapshot
                .as_deref()
                .and_then(|snapshot| {
                    snapshot
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.workspace_id == press.workspace_id)
                })
                .is_some_and(|workspace| {
                    !workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.is_linked_worktree)
                })
    }

    pub(super) fn finish_endpoint_workspace_press(
        &mut self,
        press: ClientWorkspacePress,
        outcome: &mut ClientShellInput,
    ) {
        self.focus_or_activate(
            press.endpoint_id,
            ClientEndpointFocusTarget::Workspace(press.workspace_id),
            outcome,
        );
    }

    pub(super) fn toggle_machine_collapse(
        &mut self,
        endpoint_id: ClientEndpointId,
        outcome: &mut ClientShellInput,
    ) {
        if !self.collapsed_endpoints.remove(&endpoint_id) {
            self.collapsed_endpoints.insert(endpoint_id);
        }
        self.persist_chrome_preferences(outcome);
        outcome.repaint = true;
    }

    pub(super) fn collapse_other_machines(
        &mut self,
        endpoint_id: &ClientEndpointId,
        outcome: &mut ClientShellInput,
    ) {
        let others = self
            .endpoints
            .iter()
            .map(|endpoint| &endpoint.endpoint_id)
            .filter(|other| *other != endpoint_id);
        // Once every other machine is folded, the same key unfolds all of them.
        self.collapsed_endpoints = if others
            .clone()
            .all(|other| self.collapsed_endpoints.contains(other))
        {
            HashSet::new()
        } else {
            others.cloned().collect()
        };
        self.persist_chrome_preferences(outcome);
        outcome.repaint = true;
    }

    /// The snapshot a fold key acts on: the active endpoint's, or a foreign one in navigate mode.
    fn endpoint_fold_snapshot(
        &self,
        endpoint_id: &ClientEndpointId,
    ) -> Option<&ClientShellSnapshot> {
        if *endpoint_id == self.active_endpoint_id {
            if let Some(snapshot) = self.snapshot.as_deref() {
                return Some(snapshot);
            }
        }
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| endpoint.snapshot.as_deref())
    }

    /// Repo keys of the space groups the sidebar can fold: a main checkout with linked worktrees.
    fn foldable_space_groups(snapshot: &ClientShellSnapshot) -> Vec<String> {
        let mut keys = Vec::new();
        for workspace in &snapshot.workspaces {
            let Some(worktree) = workspace.worktree.as_ref() else {
                continue;
            };
            if worktree.is_linked_worktree || keys.contains(&worktree.key) {
                continue;
            }
            let has_child = snapshot.workspaces.iter().any(|candidate| {
                candidate.worktree.as_ref().is_some_and(|candidate| {
                    candidate.key == worktree.key && candidate.is_linked_worktree
                })
            });
            if has_child {
                keys.push(worktree.key.clone());
            }
        }
        keys
    }

    /// The fold-key target: the navigate selection, else the active endpoint's focused workspace.
    fn space_fold_target(&self) -> (ClientEndpointId, Option<String>) {
        match self.navigate_workspace_id.as_ref() {
            Some(target) => (
                target.endpoint_id.clone(),
                Some(target.workspace_id.clone()),
            ),
            None => (
                self.active_endpoint_id.clone(),
                self.snapshot
                    .as_deref()
                    .and_then(|snapshot| snapshot.focused_workspace_id.clone()),
            ),
        }
    }

    /// Folds or unfolds the space group of the target workspace, the same state as the group
    /// arrow and `herdr space fold`.
    pub(super) fn toggle_space_collapse(&mut self, outcome: &mut ClientShellInput) {
        let (endpoint_id, workspace_id) = self.space_fold_target();
        let key = self
            .endpoint_fold_snapshot(&endpoint_id)
            .and_then(|snapshot| {
                let worktree = snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| Some(&workspace.workspace_id) == workspace_id.as_ref())?
                    .worktree
                    .as_ref()?;
                Self::foldable_space_groups(snapshot)
                    .contains(&worktree.key)
                    .then(|| worktree.key.clone())
            });
        let Some(key) = key else {
            return;
        };
        self.toggle_collapsed_group(&endpoint_id, key);
        self.persist_chrome_preferences(outcome);
        outcome.repaint = true;
    }

    /// Folds every other space group of that machine and unfolds the target's own; once every
    /// other group is folded, unfolds all instead (`herdr space fold-others`).
    pub(super) fn collapse_other_spaces(&mut self, outcome: &mut ClientShellInput) {
        let (endpoint_id, workspace_id) = self.space_fold_target();
        let Some((all, own)) = self.endpoint_fold_snapshot(&endpoint_id).map(|snapshot| {
            let own = snapshot
                .workspaces
                .iter()
                .find(|workspace| Some(&workspace.workspace_id) == workspace_id.as_ref())
                .and_then(|workspace| workspace.worktree.as_ref())
                .map(|worktree| worktree.key.clone());
            (Self::foldable_space_groups(snapshot), own)
        }) else {
            return;
        };
        let others = all
            .into_iter()
            .filter(|key| Some(key) != own.as_ref())
            .collect::<HashSet<_>>();
        if others.is_empty() {
            return;
        }
        let folded = self
            .collapsed_groups_for_endpoint(&endpoint_id)
            .cloned()
            .unwrap_or_default();
        let next = if others.iter().all(|key| folded.contains(key)) {
            HashSet::new()
        } else {
            others
        };
        if endpoint_id.is_local() {
            self.collapsed_groups = next;
        } else {
            self.remote_collapsed_groups.insert(endpoint_id, next);
        }
        self.persist_chrome_preferences(outcome);
        outcome.repaint = true;
    }

    pub(super) fn handle_endpoint_machine_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(hit) = self
            .hits
            .machines
            .iter()
            .find(|hit| super::contains(hit.rect, point))
        else {
            return false;
        };
        let endpoint_id = hit.endpoint_id.clone();
        let collapse_toggle = super::contains(hit.collapse_toggle, point);
        if collapse_toggle || endpoint_id == self.active_endpoint_id {
            self.toggle_machine_collapse(endpoint_id.clone(), outcome);
            if !collapse_toggle && endpoint_id.is_local() {
                self.activate_endpoint(endpoint_id, outcome);
            }
        } else if endpoint_id.is_local() || self.endpoint_is_online(&endpoint_id) {
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id,
                target: None,
            });
        } else {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
        }
        true
    }

    pub(super) fn handle_endpoint_agent_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some((endpoint_id, pane_id)) = self
            .hits
            .endpoint_agents
            .iter()
            .find(|(rect, _, _)| super::contains(*rect, point))
            .map(|(_, endpoint_id, pane_id)| (endpoint_id.clone(), pane_id.clone()))
        else {
            return false;
        };
        self.focus_or_activate(
            endpoint_id,
            ClientEndpointFocusTarget::Pane(pane_id),
            outcome,
        );
        true
    }

    pub(super) fn handle_endpoint_navigation(
        &mut self,
        action: crate::input::KeybindAction,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crate::input::KeybindAction;
        if !self.multi_endpoint_active() {
            return false;
        }
        if matches!(
            action,
            KeybindAction::PreviousWorkspace | KeybindAction::NextWorkspace
        ) {
            let workspaces = self
                .endpoints
                .iter()
                .filter(|endpoint| endpoint.status == ClientEndpointStatus::Online)
                .flat_map(|endpoint| {
                    endpoint
                        .snapshot
                        .as_deref()
                        .map_or_else(Vec::new, |snapshot| {
                            render::workspace_entries(snapshot, &HashSet::new())
                                .into_iter()
                                .filter_map(|entry| {
                                    snapshot.workspaces.get(entry.index).map(|workspace| {
                                        (
                                            endpoint.endpoint_id.clone(),
                                            workspace.workspace_id.clone(),
                                        )
                                    })
                                })
                                .collect()
                        })
                })
                .collect::<Vec<_>>();
            if workspaces.is_empty() {
                return true;
            }
            let focused = self
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.focused_workspace_id.as_deref());
            let current = workspaces.iter().position(|(endpoint_id, workspace_id)| {
                endpoint_id == &self.active_endpoint_id && Some(workspace_id.as_str()) == focused
            });
            let next = match (current, action) {
                (Some(index), KeybindAction::PreviousWorkspace) => {
                    (index + workspaces.len() - 1) % workspaces.len()
                }
                (Some(index), KeybindAction::NextWorkspace) => (index + 1) % workspaces.len(),
                (None, KeybindAction::PreviousWorkspace) => workspaces.len() - 1,
                (None, KeybindAction::NextWorkspace) => 0,
                _ => unreachable!("endpoint workspace navigation"),
            };
            let (endpoint_id, workspace_id) = workspaces[next].clone();
            self.focus_or_activate(
                endpoint_id,
                ClientEndpointFocusTarget::Workspace(workspace_id),
                outcome,
            );
            return true;
        }
        if matches!(
            action,
            KeybindAction::PreviousAgent | KeybindAction::NextAgent | KeybindAction::FocusAgent(_)
        ) {
            let agents = super::aggregate_navigation::online_agent_targets(
                &self.endpoints,
                &self.active_endpoint_id,
                self.config.agent_panel_sort,
            );
            if agents.is_empty() {
                return true;
            }
            let next = match action {
                KeybindAction::FocusAgent(index) => {
                    if index >= agents.len() {
                        return true;
                    }
                    index
                }
                KeybindAction::PreviousAgent | KeybindAction::NextAgent => {
                    let focused = self
                        .snapshot
                        .as_deref()
                        .and_then(|snapshot| snapshot.focused_pane_id.as_deref());
                    let current = agents.iter().position(|target| {
                        target.endpoint_id == self.active_endpoint_id
                            && Some(target.pane_id.as_str()) == focused
                    });
                    match (current, action) {
                        (Some(index), KeybindAction::PreviousAgent) => {
                            (index + agents.len() - 1) % agents.len()
                        }
                        (Some(index), KeybindAction::NextAgent) => (index + 1) % agents.len(),
                        (None, KeybindAction::PreviousAgent) => agents.len() - 1,
                        _ => 0,
                    }
                }
                _ => unreachable!("endpoint agent navigation"),
            };
            let target = &agents[next];
            if self.focus_or_activate(
                target.endpoint_id.clone(),
                ClientEndpointFocusTarget::Pane(target.pane_id.clone()),
                outcome,
            ) {
                if target.endpoint_id == self.active_endpoint_id {
                    self.reveal_endpoint_agent(
                        &target.endpoint_id,
                        &target.pane_id,
                        self.hits.agent_body.height,
                    );
                } else {
                    self.pending_agent_reveal =
                        Some((target.endpoint_id.clone(), target.pane_id.clone()));
                }
                outcome.repaint = true;
            }
            return true;
        }
        false
    }

    pub(super) fn activate_endpoint(
        &mut self,
        endpoint_id: ClientEndpointId,
        outcome: &mut ClientShellInput,
    ) -> bool {
        self.pending_workspace_highlight = None;
        self.pending_agent_reveal = None;
        let online = self.endpoint_is_online(&endpoint_id);
        if !online && !endpoint_id.is_local() {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
            return false;
        }
        if (endpoint_id.is_local() && (self.multi_endpoint_active() || !online))
            || endpoint_id != self.active_endpoint_id
        {
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id,
                target: None,
            });
        }
        true
    }

    pub(super) fn focus_or_activate(
        &mut self,
        endpoint_id: ClientEndpointId,
        target: ClientEndpointFocusTarget,
        outcome: &mut ClientShellInput,
    ) -> bool {
        #[cfg(windows)]
        if !self.notification_target_is_current(&endpoint_id, &target) {
            return false;
        }
        self.pending_workspace_highlight = None;
        self.pending_agent_reveal = None;
        let online = self.endpoint_is_online(&endpoint_id);
        if !online && !endpoint_id.is_local() {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
            return false;
        }
        // Local can still be displayed while a remote activation is pending.
        // Route explicit selections through the runtime so they can cancel that handoff.
        if endpoint_id == self.active_endpoint_id
            && !(endpoint_id.is_local() && (self.multi_endpoint_active() || !online))
        {
            let method = match target {
                ClientEndpointFocusTarget::Workspace(workspace_id) => {
                    crate::api::schema::Method::WorkspaceFocus(
                        crate::api::schema::WorkspaceTarget { workspace_id },
                    )
                }
                ClientEndpointFocusTarget::Tab(tab_id) => {
                    crate::api::schema::Method::TabFocus(crate::api::schema::TabTarget { tab_id })
                }
                ClientEndpointFocusTarget::Pane(pane_id) => {
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id,
                    })
                }
                #[cfg(windows)]
                ClientEndpointFocusTarget::Notification { pane_id, .. } => {
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id,
                    })
                }
            };
            self.push_endpoint_method(method, outcome);
        } else {
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id,
                target: Some(target),
            });
        }
        true
    }
}
