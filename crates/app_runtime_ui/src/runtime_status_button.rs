use app_runtime::{RuntimeCatalog, SystemCommandRunner};
use gpui::{AppContext, Context, IntoElement, ParentElement, Render, Task, WeakEntity, Window, div};
use ui::{Button, ButtonCommon, Clickable, LabelSize, Tooltip};
use workspace::{TitleBarItemView, Workspace, item::ItemHandle};

use crate::OpenRuntimeActions;

pub struct RuntimeStatusButton {
    workspace: WeakEntity<Workspace>,
    has_runnable_project: bool,
    _detect_task: Task<()>,
}

impl RuntimeStatusButton {
    pub fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace_paths = workspace
            .project()
            .read(cx)
            .visible_worktrees(cx)
            .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
            .collect::<Vec<_>>();

        let detect_task = cx.spawn_in(window, async move |this, cx| {
            let catalog = cx
                .background_spawn(async move {
                    let runner = SystemCommandRunner;
                    RuntimeCatalog::discover(&workspace_paths, &runner)
                })
                .await;

            this.update(cx, |this, cx| {
                this.has_runnable_project = !catalog.projects.is_empty();
                cx.notify();
            })
            .ok();
        });

        Self {
            workspace: workspace.weak_handle(),
            has_runnable_project: false,
            _detect_task: detect_task,
        }
    }

    pub fn should_render(&self) -> bool {
        self.has_runnable_project
    }
}

impl Render for RuntimeStatusButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.has_runnable_project {
            div().child(
                Button::new("runtime-actions-titlebar", "Run App")
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(workspace) = this.workspace.upgrade() {
                            workspace.update(cx, |workspace, cx| {
                                RuntimeStatusButton::open_runtime_actions(workspace, window, cx);
                            });
                        }
                    }))
                    .tooltip(|_window, cx| {
                        Tooltip::for_action("Runtime Actions", &OpenRuntimeActions, cx)
                    }),
            )
        } else {
            div()
        }
    }
}

impl TitleBarItemView for RuntimeStatusButton {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }
}

impl RuntimeStatusButton {
    fn open_runtime_actions(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut gpui::Context<Workspace>,
    ) {
        crate::runtime_actions_modal::RuntimeActionsModal::toggle(
            workspace,
            &OpenRuntimeActions,
            window,
            cx,
        );
    }
}
