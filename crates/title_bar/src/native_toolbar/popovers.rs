use crate::TitleBar;
use gpui::{
    DismissEvent, Focusable, NativePopover, NativePopoverAnchor, NativePopoverBehavior,
    SharedString, Window,
};
use std::collections::HashSet;
use ui::ContextMenu;
use workspace::WorkspaceId;

impl TitleBar {
    fn show_toolbar_context_menu_popover(
        &mut self,
        menu: gpui::Entity<ContextMenu>,
        item_id: &'static str,
        width: f64,
        height: f64,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let item_id: SharedString = item_id.into();
        if self
            .native_toolbar_state
            .open_toolbar_popover_item_id
            .as_ref()
            == Some(&item_id)
        {
            self.native_toolbar_state.open_toolbar_popover_item_id = None;
            window.dismiss_native_popover();
            cx.notify();
            return;
        }

        let weak_title_bar = cx.entity().downgrade();
        let dismiss_title_bar = weak_title_bar.clone();
        window
            .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                window.dismiss_native_popover();
                dismiss_title_bar
                    .update(cx, |title_bar, cx| {
                        title_bar.native_toolbar_state.open_toolbar_popover_item_id = None;
                        cx.notify();
                    })
                    .ok();
            })
            .detach();

        self.native_toolbar_state.open_toolbar_popover_item_id = Some(item_id.clone());
        window.dismiss_native_popover();
        window.show_native_popover(
            NativePopover::new(width, height)
                .behavior(NativePopoverBehavior::Transient)
                .on_close(move |_, _, cx| {
                    weak_title_bar
                        .update(cx, |title_bar, cx| {
                            title_bar.native_toolbar_state.open_toolbar_popover_item_id = None;
                            cx.notify();
                        })
                        .ok();
                })
                .content_view(menu),
            NativePopoverAnchor::ToolbarItem(item_id),
        );
        cx.notify();
    }

    pub(super) fn show_recent_projects_popover(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let workspace = self.workspace.clone();
        let focus_handle = workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).focus_handle(cx))
            .unwrap_or_else(|| cx.focus_handle());
        let sibling_workspace_ids: HashSet<WorkspaceId> = self
            .multi_workspace
            .as_ref()
            .and_then(|multi_workspace| multi_workspace.upgrade())
            .map(|multi_workspace| {
                multi_workspace
                    .read(cx)
                    .workspaces()
                    .iter()
                    .filter_map(|workspace| workspace.read(cx).database_id())
                    .collect()
            })
            .unwrap_or_default();

        let Some(popover) = workspace.upgrade().map(|_| {
            recent_projects::RecentProjects::popover(
                workspace.clone(),
                sibling_workspace_ids,
                false,
                focus_handle,
                window,
                cx,
            )
        }) else {
            return;
        };

        window.dismiss_native_popover();
        window.show_native_popover(
            NativePopover::new(360.0, 420.0)
                .behavior(NativePopoverBehavior::Transient)
                .content_view(popover),
            NativePopoverAnchor::ToolbarItem("glass.project_name".into()),
        );
    }

    pub(super) fn show_branch_popover(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let effective_repository = self
            .effective_active_worktree(cx)
            .and_then(|worktree| self.get_repository_for_worktree(&worktree, cx));
        let popover = git_ui::git_picker::popover(
            workspace.downgrade(),
            effective_repository,
            git_ui::git_picker::GitPickerTab::Branches,
            gpui::rems(34.0),
            window,
            cx,
        );

        window.dismiss_native_popover();
        window.show_native_popover(
            NativePopover::new(380.0, 480.0)
                .behavior(NativePopoverBehavior::Transient)
                .content_view(popover),
            NativePopoverAnchor::ToolbarItem("glass.branch_name".into()),
        );
    }

    pub(crate) fn show_lsp_menu(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(lsp_button) = self.right_item_view::<language_tools::lsp_button::LspButton>()
        else {
            return;
        };
        let menu = lsp_button.update(cx, |lsp_button, cx| {
            lsp_button.ensure_toolbar_menu(window, cx)
        });
        let Some(menu) = menu else {
            return;
        };

        self.show_toolbar_context_menu_popover(
            menu,
            "glass.status.language_servers",
            320.0,
            420.0,
            window,
            cx,
        );
    }
}
