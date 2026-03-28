use gpui::{Action, App, NativeToolbarButton, NativeToolbarItem};
use workspace::ToggleWorktreeSecurity;
use zed_actions::OpenRemote;

use crate::{MAX_BRANCH_NAME_LENGTH, MAX_PROJECT_NAME_LENGTH, MAX_SHORT_SHA_LENGTH, TitleBar};

impl TitleBar {
    pub(crate) fn build_restricted_mode_item(&self, cx: &App) -> Option<NativeToolbarItem> {
        self.has_restricted_worktrees(cx).then(|| {
            NativeToolbarItem::Button(
                NativeToolbarButton::new("glass.restricted_mode", "Restricted Mode")
                    .tool_tip("Manage Worktree Trust")
                    .icon("exclamationmark.shield")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(ToggleWorktreeSecurity.boxed_clone(), cx);
                    }),
            )
        })
    }

    pub(crate) fn build_project_button_item(&self, cx: &App) -> Option<NativeToolbarItem> {
        let display_name = self
            .effective_active_worktree(cx)
            .map(|worktree| {
                util::truncate_and_trailoff(
                    worktree.read(cx).root_name().as_unix_str(),
                    MAX_PROJECT_NAME_LENGTH,
                )
            })
            .unwrap_or_else(|| "Open Recent Project".to_string());
        let workspace = self.workspace.clone();

        Some(NativeToolbarItem::Button(
            NativeToolbarButton::new("glass.project_name", display_name)
                .icon("folder")
                .tool_tip("Recent Projects")
                .on_click(move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.show_recent_projects_overlay(window, cx);
                        });
                    }
                }),
        ))
    }

    pub(crate) fn build_branch_button_item(&self, cx: &App) -> Option<NativeToolbarItem> {
        let effective_worktree = self.effective_active_worktree(cx)?;
        let repository = self.get_repository_for_worktree(&effective_worktree, cx)?;

        let branch_name = {
            let repository = repository.read(cx);
            repository
                .branch
                .as_ref()
                .map(|branch| branch.name())
                .map(|name| util::truncate_and_trailoff(name, MAX_BRANCH_NAME_LENGTH))
                .or_else(|| {
                    repository.head_commit.as_ref().map(|commit| {
                        commit
                            .sha
                            .chars()
                            .take(MAX_SHORT_SHA_LENGTH)
                            .collect::<String>()
                    })
                })?
        };

        let workspace = self.workspace.clone();
        Some(NativeToolbarItem::Button(
            NativeToolbarButton::new("glass.branch_name", branch_name)
                .tool_tip("Git Branches")
                .icon("arrow.triangle.branch")
                .on_click(move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.show_branch_overlay(window, cx);
                        });
                    }
                }),
        ))
    }

    pub(crate) fn build_project_host_item(&self, cx: &App) -> Option<NativeToolbarItem> {
        if self.project.read(cx).is_via_remote_server() {
            let options = self.project.read(cx).remote_connection_options(cx)?;
            return Some(NativeToolbarItem::Button(
                NativeToolbarButton::new("glass.project_host", options.display_name())
                    .tool_tip("Remote Project")
                    .icon("server.rack")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            OpenRemote {
                                from_existing_connection: false,
                                create_new_window: false,
                            }
                            .boxed_clone(),
                            cx,
                        );
                    }),
            ));
        }

        if self.project.read(cx).is_disconnected(cx) {
            return Some(NativeToolbarItem::Button(
                NativeToolbarButton::new("glass.project_host", "Disconnected")
                    .tool_tip("Disconnected Remote Project")
                    .icon("bolt.horizontal.circle"),
            ));
        }

        let host = self.project.read(cx).host()?;
        let host_user = self.user_store.read(cx).get_cached_user(host.user_id)?;
        let workspace = self.workspace.clone();
        let peer_id = host.peer_id;
        let mut button =
            NativeToolbarButton::new("glass.project_host", host_user.github_login.clone())
                .tool_tip("Follow Project Host")
                .on_click(move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.follow(peer_id, window, cx);
                        });
                    }
                });
        let avatar_url = host_user.avatar_uri.to_string();
        if !avatar_url.is_empty() {
            button = button.image_url(avatar_url).image_circular(true);
        }
        Some(NativeToolbarItem::Button(button))
    }
}
