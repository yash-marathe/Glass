use app_runtime::{
    DetectedProject, ExecutionRequest, RuntimeAction, RuntimeCatalog, RuntimeDevice, RuntimeTarget,
    SystemCommandRunner,
};
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, ParentElement, Render,
    SharedString, Styled, Task, WeakEntity, Window,
};
use menu::Cancel;
use task::{HideStrategy, RevealStrategy, SaveStrategy, Shell, SpawnInTerminal, TaskId};
use ui::{
    Button, ButtonSize, Color, ContextMenu, Divider, DropdownMenu, DropdownStyle, KeyBinding,
    Label, LabelSize, Tooltip, prelude::*,
};
use uuid::Uuid;
use workspace::{ModalView, Workspace};

use crate::OpenRuntimeActions;

pub struct RuntimeActionsModal {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    catalog: Option<RuntimeCatalog>,
    selected_project_index: usize,
    selected_target_index: usize,
    selected_device_index: Option<usize>,
    selected_action: RuntimeAction,
    status_message: Option<SharedString>,
    _load_task: Task<()>,
}

impl RuntimeActionsModal {
    pub fn toggle(
        workspace: &mut Workspace,
        _: &OpenRuntimeActions,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let workspace_paths = workspace
            .project()
            .read(cx)
            .visible_worktrees(cx)
            .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
            .collect::<Vec<_>>();
        let workspace_handle = workspace.weak_handle();

        workspace.toggle_modal(window, cx, |window, cx| {
            Self::new(workspace_handle.clone(), workspace_paths.clone(), window, cx)
        });
    }

    fn new(
        workspace: WeakEntity<Workspace>,
        workspace_paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let load_task = cx.spawn_in(window, async move |this, cx| {
            let catalog = cx
                .background_spawn(async move {
                    let runner = SystemCommandRunner;
                    RuntimeCatalog::discover(&workspace_paths, &runner)
                })
                .await;

            this.update(cx, |this, cx| {
                this.catalog = Some(catalog);
                this.reset_selection();
                cx.notify();
            })
            .ok();
        });

        Self {
            workspace,
            focus_handle: cx.focus_handle(),
            catalog: None,
            selected_project_index: 0,
            selected_target_index: 0,
            selected_device_index: None,
            selected_action: RuntimeAction::Run,
            status_message: None,
            _load_task: load_task,
        }
    }

    fn selected_project(&self) -> Option<&DetectedProject> {
        self.catalog
            .as_ref()?
            .projects
            .get(self.selected_project_index)
    }

    fn selected_target(&self) -> Option<&RuntimeTarget> {
        self.selected_project()?
            .targets
            .get(self.selected_target_index)
    }

    fn selected_device(&self) -> Option<&RuntimeDevice> {
        let device_index = self.selected_device_index?;
        self.selected_project()?.devices.get(device_index)
    }

    fn reset_selection(&mut self) {
        let Some(catalog) = self.catalog.as_ref() else {
            self.selected_project_index = 0;
            self.selected_target_index = 0;
            self.selected_device_index = None;
            self.status_message = None;
            return;
        };

        let Some(project) = catalog.projects.get(self.selected_project_index) else {
            self.selected_project_index = 0;
            self.selected_target_index = 0;
            self.selected_device_index = None;
            self.status_message = Some("No runnable Apple project was detected.".into());
            return;
        };

        if self.selected_target_index >= project.targets.len() {
            self.selected_target_index = 0;
        }

        if project.devices.is_empty() {
            self.selected_device_index = None;
        } else if self.selected_device_index.is_none() {
            self.selected_device_index = Some(0);
        } else if self.selected_device_index.unwrap_or_default() >= project.devices.len() {
            self.selected_device_index = Some(0);
        }

        self.status_message = None;
    }

    fn choose_project(&mut self, project_index: usize, cx: &mut Context<Self>) {
        self.selected_project_index = project_index;
        self.selected_target_index = 0;
        self.selected_device_index = None;
        self.reset_selection();
        cx.notify();
    }

    fn choose_target(&mut self, target_index: usize, cx: &mut Context<Self>) {
        self.selected_target_index = target_index;
        self.status_message = None;
        cx.notify();
    }

    fn choose_device(&mut self, device_index: usize, cx: &mut Context<Self>) {
        self.selected_device_index = Some(device_index);
        self.status_message = None;
        cx.notify();
    }

    fn choose_action(&mut self, action: RuntimeAction, cx: &mut Context<Self>) {
        self.selected_action = action;
        self.status_message = None;
        cx.notify();
    }

    fn dismiss(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.as_ref() else {
            self.status_message = Some("Runtime detection is still loading.".into());
            cx.notify();
            return;
        };
        let Some(project) = self.selected_project() else {
            self.status_message = Some("No runnable Apple project was detected.".into());
            cx.notify();
            return;
        };
        let Some(target) = self.selected_target() else {
            self.status_message = Some("No shared Xcode scheme was detected.".into());
            cx.notify();
            return;
        };

        let request = ExecutionRequest {
            project_id: project.id.clone(),
            target_id: target.id.clone(),
            device_id: self.selected_device().map(|device| device.id.clone()),
            action: self.selected_action,
        };

        let plan = match catalog.build_execution_plan(&request) {
            Ok(plan) => plan,
            Err(error) => {
                self.status_message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };

        let task = SpawnInTerminal {
            id: TaskId(format!("app-runtime-{}", Uuid::new_v4())),
            full_label: plan.label.clone(),
            label: plan.label,
            command: Some(plan.command),
            args: plan.args,
            command_label: plan.command_label,
            cwd: Some(plan.cwd),
            env: Default::default(),
            use_new_terminal: true,
            allow_concurrent_runs: true,
            reveal: RevealStrategy::Always,
            reveal_target: task::RevealTarget::Dock,
            hide: HideStrategy::Never,
            shell: Shell::System,
            show_summary: true,
            show_command: true,
            show_rerun: true,
            save: SaveStrategy::All,
        };

        let spawned = self
            .workspace
            .update(cx, |workspace, cx| workspace.spawn_in_terminal(task, window, cx))
            .is_ok();

        if spawned {
            cx.emit(DismissEvent);
        } else {
            self.status_message = Some("Glass could not start the runtime command.".into());
            cx.notify();
        }
    }

    fn project_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ContextMenu> {
        let entries = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .projects
                    .iter()
                    .enumerate()
                    .map(|(index, project)| (index, project.label.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let modal = cx.weak_entity();

        ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (index, label) in entries.clone() {
                let modal = modal.clone();
                menu = menu.entry(label, None, move |_, cx| {
                    if let Some(modal) = modal.upgrade() {
                        modal.update(cx, |this, cx| this.choose_project(index, cx));
                    }
                });
            }
            menu
        })
    }

    fn target_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ContextMenu> {
        let entries = self
            .selected_project()
            .map(|project| {
                project
                    .targets
                    .iter()
                    .enumerate()
                    .map(|(index, target)| (index, target.label.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let modal = cx.weak_entity();

        ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (index, label) in entries.clone() {
                let modal = modal.clone();
                menu = menu.entry(label, None, move |_, cx| {
                    if let Some(modal) = modal.upgrade() {
                        modal.update(cx, |this, cx| this.choose_target(index, cx));
                    }
                });
            }
            menu
        })
    }

    fn device_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ContextMenu> {
        let entries = self
            .selected_project()
            .map(|project| {
                project
                    .devices
                    .iter()
                    .enumerate()
                    .map(|(index, device)| {
                        let label = match &device.os_version {
                            Some(os_version) => format!("{} ({os_version})", device.name),
                            None => device.name.clone(),
                        };
                        (index, label)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let modal = cx.weak_entity();

        ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (index, label) in entries.clone() {
                let modal = modal.clone();
                menu = menu.entry(label, None, move |_, cx| {
                    if let Some(modal) = modal.upgrade() {
                        modal.update(cx, |this, cx| this.choose_device(index, cx));
                    }
                });
            }
            menu
        })
    }

    fn action_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ContextMenu> {
        let modal = cx.weak_entity();
        ContextMenu::build(window, cx, move |mut menu, _, _| {
            for action in [RuntimeAction::Run, RuntimeAction::Build] {
                let modal = modal.clone();
                menu = menu.entry(action.label(), None, move |_, cx| {
                    if let Some(modal) = modal.upgrade() {
                        modal.update(cx, |this, cx| this.choose_action(action, cx));
                    }
                });
            }
            menu
        })
    }
}

impl EventEmitter<DismissEvent> for RuntimeActionsModal {}

impl Focusable for RuntimeActionsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for RuntimeActionsModal {}

impl Render for RuntimeActionsModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.catalog.is_none();
        let no_projects = self
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.projects.is_empty());

        let project_label = self
            .selected_project()
            .map(|project| project.label.clone())
            .unwrap_or_else(|| "No project".to_string());
        let target_label = self
            .selected_target()
            .map(|target| target.label.clone())
            .unwrap_or_else(|| "No target".to_string());
        let device_label = self
            .selected_device()
            .map(|device| match &device.os_version {
                Some(os_version) => format!("{} ({os_version})", device.name),
                None => device.name.clone(),
            })
            .unwrap_or_else(|| "No device".to_string());
        let execute_disabled = loading
            || no_projects
            || self.selected_target().is_none()
            || (matches!(self.selected_action, RuntimeAction::Run) && self.selected_device().is_none());

        v_flex()
            .key_context("RuntimeActions")
            .on_action(cx.listener(Self::dismiss))
            .w(rems(34.))
            .gap_3()
            .p_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("Runtime Actions").size(LabelSize::Large))
                    .child(
                        Label::new("Run or build the detected Apple app from this workspace.")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .gap_2()
                    .child(Label::new("Project").size(LabelSize::Small).color(Color::Muted))
                    .child(
                        DropdownMenu::new("runtime-project", project_label, self.project_menu(window, cx))
                            .style(DropdownStyle::Outlined)
                            .trigger_size(ButtonSize::Compact)
                            .disabled(loading || no_projects),
                    )
                    .child(Label::new("Target").size(LabelSize::Small).color(Color::Muted))
                    .child(
                        DropdownMenu::new("runtime-target", target_label, self.target_menu(window, cx))
                            .style(DropdownStyle::Outlined)
                            .trigger_size(ButtonSize::Compact)
                            .disabled(loading || no_projects),
                    )
                    .child(Label::new("Action").size(LabelSize::Small).color(Color::Muted))
                    .child(
                        DropdownMenu::new("runtime-action", self.selected_action.label(), self.action_menu(window, cx))
                            .style(DropdownStyle::Outlined)
                            .trigger_size(ButtonSize::Compact)
                            .disabled(loading || no_projects),
                    )
                    .child(Label::new("Device").size(LabelSize::Small).color(Color::Muted))
                    .child(
                        DropdownMenu::new("runtime-device", device_label, self.device_menu(window, cx))
                            .style(DropdownStyle::Outlined)
                            .trigger_size(ButtonSize::Compact)
                            .disabled(
                                loading
                                    || no_projects
                                    || matches!(self.selected_action, RuntimeAction::Build)
                                    || self.selected_project().is_none_or(|project| project.devices.is_empty()),
                            ),
                    ),
            )
            .when(loading, |this| {
                this.child(
                    Label::new("Detecting runtime capabilities...")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when_some(self.status_message.clone(), |this, message| {
                this.child(
                    Label::new(message)
                        .size(LabelSize::Small)
                        .color(Color::Error),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-runtime-actions", "Cancel")
                            .key_binding(KeyBinding::for_action(&Cancel, cx))
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(&Cancel, window, cx))),
                    )
                    .child(
                        Button::new("execute-runtime-action", self.selected_action.label())
                            .disabled(execute_disabled)
                            .tooltip(move |_window, cx| {
                                Tooltip::for_action(
                                    "Open Runtime Actions",
                                    &OpenRuntimeActions,
                                    cx,
                                )
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.execute(window, cx))),
                    ),
            )
    }
}
