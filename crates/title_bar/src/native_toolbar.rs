use crate::{TitleBar, show_menus};
use browser::{self, BrowserView, history::HistoryMatch};
use client::{Status as ClientStatus, User, zed_urls};
use editor::{Editor, EditorEvent};
use encoding_selector::Toggle as ToggleEncoding;
use gpui::{
    Action, AnyElement, App, Context, Corner, DismissEvent, Entity, Focusable, InteractiveElement,
    IntoElement, NativePopover, NativePopoverAnchor, NativePopoverBehavior,
    NativePopoverClickableRow, NativePopoverContentItem, NativeSearchFieldTarget,
    NativeSearchSuggestionMenu, NativeToolbar, NativeToolbarButton, NativeToolbarClickEvent,
    NativeToolbarDisplayMode, NativeToolbarItem, NativeToolbarLabel, NativeToolbarMenuButton,
    NativeToolbarMenuButtonSelectEvent, NativeToolbarMenuItem, NativeToolbarSearchEvent,
    NativeToolbarSearchField, NativeToolbarSizeMode, ParentElement, SharedString, Subscription,
    Window, anchored, deferred, div, point, px,
};
use image_viewer::ImageView;
use language::LineEnding;
use project::{image_store::{ImageFormat, ImageMetadata}, trusted_worktrees::TrustedWorktrees};
use settings::Settings;
use std::collections::HashSet;
use std::sync::Arc;
use ui::ContextMenu;
use workspace::{ToggleWorktreeSecurity, WorkspaceId, notifications::NotifyResultExt};
use workspace_modes::ModeId;
use zed_actions::OpenRemote;
use crate::title_bar_settings::TitleBarSettings;

const MAX_PROJECT_NAME_LENGTH: usize = 40;
const MAX_BRANCH_NAME_LENGTH: usize = 40;
const MAX_SHORT_SHA_LENGTH: usize = 8;
#[derive(Default)]
pub(crate) struct NativeToolbarState {
    pub(crate) omnibox_text: String,
    pub(crate) omnibox_focused: bool,
    pub(crate) omnibox_panel_dirty: bool,
    pub(crate) omnibox_suggestions: Vec<HistoryMatch>,
    pub(crate) omnibox_selected_index: Option<usize>,
    pub(crate) last_toolbar_key: String,
    pub(crate) status_encoding: Option<String>,
    pub(crate) status_line_ending: Option<String>,
    pub(crate) status_toolchain: Option<String>,
    pub(crate) status_image_info: Option<String>,
    pub(crate) active_editor_subscription: Option<Subscription>,
    pub(crate) active_image_subscription: Option<Subscription>,
    pub(crate) toolbar_overlay_menu: Option<Entity<ContextMenu>>,
    pub(crate) toolbar_overlay_anchor: Option<gpui::Point<gpui::Pixels>>,
    pub(crate) toolbar_overlay_item_id: Option<SharedString>,
    pub(crate) toolbar_overlay_subscription: Option<Subscription>,
}

impl TitleBar {
    pub(crate) fn render_macos_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if show_menus(cx) {
            window.set_native_toolbar(None);
            return self.render_gpui_title_bar(window, cx).into_any_element();
        }

        self.update_native_toolbar(window, cx);
        let button_layout = TitleBarSettings::get_global(cx).button_layout;
        let overlay = self.render_toolbar_overlay();

        self.platform_titlebar.update(cx, |titlebar, _| {
            titlebar.set_button_layout(button_layout);
            titlebar.set_children(overlay.into_iter());
        });

        self.platform_titlebar.clone().into_any_element()
    }

    pub(crate) fn invalidate_native_toolbar(&mut self, cx: &mut Context<Self>) {
        self.native_toolbar_state.last_toolbar_key.clear();
        cx.notify();
    }

    fn close_toolbar_overlay(&mut self, cx: &mut Context<Self>) {
        self.native_toolbar_state.toolbar_overlay_menu = None;
        self.native_toolbar_state.toolbar_overlay_anchor = None;
        self.native_toolbar_state.toolbar_overlay_item_id = None;
        self.native_toolbar_state.toolbar_overlay_subscription = None;
        cx.notify();
    }

    fn render_toolbar_overlay(&self) -> Option<AnyElement> {
        let menu = self.native_toolbar_state.toolbar_overlay_menu.clone()?;
        let anchor = self.native_toolbar_state.toolbar_overlay_anchor?;

        Some(
            deferred(
                anchored()
                    .position(anchor)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.0))
                    .child(div().id("native-toolbar-overlay-root").occlude().child(menu)),
            )
            .priority(1)
            .into_any_element(),
        )
    }

    fn update_native_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active_mode = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).active_mode_id())
            .unwrap_or(ModeId::BROWSER);
        let is_browser_mode = active_mode == ModeId::BROWSER;
        let is_terminal_mode = active_mode == ModeId::TERMINAL;
        let is_new_tab_page = self.active_tab_is_new_tab_page(cx);
        let title_bar_settings = *TitleBarSettings::get_global(cx);

        self.sync_omnibox_url(cx);
        self.refresh_status_data(window, cx);

        if self.native_toolbar_state.omnibox_panel_dirty {
            self.native_toolbar_state.omnibox_panel_dirty = false;
            if self.native_toolbar_state.omnibox_focused
                && !self.native_toolbar_state.omnibox_text.is_empty()
            {
                self.show_search_suggestion_menu(window);
            }
        }

        if self.native_toolbar_state.omnibox_focused {
            return;
        }

        let project_name = self
            .effective_active_worktree(cx)
            .map(|worktree| worktree.read(cx).root_name().as_unix_str().to_string())
            .unwrap_or_default();
        let branch_name = self
            .effective_active_worktree(cx)
            .and_then(|worktree| self.get_repository_for_worktree(&worktree, cx))
            .and_then(|repository| {
                let repository = repository.read(cx);
                repository.branch.as_ref().map(|branch| branch.name().to_string())
            })
            .unwrap_or_default();
        let has_restricted_worktrees = TrustedWorktrees::try_get_global(cx)
            .map(|trusted_worktrees| {
                trusted_worktrees
                    .read(cx)
                    .has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx)
            })
            .unwrap_or(false);
        let is_remote = self.project.read(cx).is_via_remote_server();
        let user = self.user_store.read(cx).current_user();
        let signed_in = user.is_some();
        let user_login = user
            .as_ref()
            .map(|user| user.github_login.to_string())
            .unwrap_or_default();
        let show_update = self.update_version.read(cx).show_update_in_menu_bar();
        let connection_status_key = match &*self.client.status().borrow() {
            ClientStatus::ConnectionError => "connection_error",
            ClientStatus::ConnectionLost => "connection_lost",
            ClientStatus::Reauthenticating => "reauthenticating",
            ClientStatus::Reconnecting => "reconnecting",
            ClientStatus::ReconnectionError { .. } => "reconnection_error",
            ClientStatus::UpgradeRequired => "upgrade_required",
            _ => "ok",
        };
        let diagnostics = self.project.read(cx).diagnostic_summary(false, cx);

        let toolbar_key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{:?}:{:?}:{:?}:{:?}",
            active_mode.0,
            project_name,
            branch_name,
            self.native_toolbar_state.omnibox_text,
            is_new_tab_page,
            has_restricted_worktrees,
            is_remote,
            title_bar_settings.show_project_items,
            title_bar_settings.show_branch_name,
            user_login,
            connection_status_key,
            show_update,
            self.native_toolbar_state.status_encoding,
            self.native_toolbar_state.status_line_ending,
            self.native_toolbar_state.status_toolchain,
            self.native_toolbar_state.status_image_info,
        );

        if self.native_toolbar_state.last_toolbar_key == toolbar_key {
            return;
        }
        self.native_toolbar_state.last_toolbar_key = toolbar_key;

        let mut toolbar = NativeToolbar::new("glass.main.toolbar")
            .title("Glass")
            .display_mode(NativeToolbarDisplayMode::IconOnly)
            .size_mode(NativeToolbarSizeMode::Regular)
            .shows_baseline_separator(false)
            .item(NativeToolbarItem::SidebarToggle)
            .item(NativeToolbarItem::SidebarTrackingSeparator)
            .item(self.build_mode_switcher_item(active_mode));

        if let Some(item) = self.build_restricted_mode_item(cx) {
            toolbar = toolbar.item(item);
        }

        if !is_browser_mode {
            if title_bar_settings.show_project_items {
                if let Some(item) = self.build_project_host_item(cx) {
                    toolbar = toolbar.item(item);
                }
                if let Some(item) = self.build_project_button_item(cx) {
                    toolbar = toolbar.item(item);
                }
            }

            if title_bar_settings.show_branch_name {
                if let Some(item) = self.build_branch_button_item(cx) {
                    toolbar = toolbar.item(item);
                }
            }
        }

        toolbar = toolbar.item(NativeToolbarItem::FlexibleSpace);

        if is_browser_mode && !is_new_tab_page {
            toolbar = toolbar
                .item(self.build_back_item())
                .item(self.build_forward_item())
                .item(self.build_reload_item())
                .item(self.build_omnibox_item())
                .item(NativeToolbarItem::FlexibleSpace)
                .item(self.build_downloads_item());
        }

        if !is_browser_mode && !is_terminal_mode {
            if let Some(toolchain) = self.native_toolbar_state.status_toolchain.clone() {
                toolbar = toolbar.item(NativeToolbarItem::Button(
                    NativeToolbarButton::new("glass.status.toolchain", toolchain)
                        .tool_tip("Select Toolchain")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(toolchain_selector::Select.boxed_clone(), cx);
                        }),
                ));
            }
            if let Some(encoding) = self.native_toolbar_state.status_encoding.clone() {
                toolbar = toolbar.item(NativeToolbarItem::Button(
                    NativeToolbarButton::new("glass.status.encoding", encoding)
                        .tool_tip("Reopen with Encoding")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(ToggleEncoding.boxed_clone(), cx);
                        }),
                ));
            }
            if let Some(line_ending) = self.native_toolbar_state.status_line_ending.clone() {
                toolbar = toolbar.item(NativeToolbarItem::Button(
                    NativeToolbarButton::new("glass.status.line_ending", line_ending)
                        .tool_tip("Select Line Ending")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(line_ending_selector::Toggle.boxed_clone(), cx);
                        }),
                ));
            }
            if let Some(image_info) = self.native_toolbar_state.status_image_info.clone() {
                toolbar = toolbar.item(NativeToolbarItem::Label(NativeToolbarLabel::new(
                    "glass.status.image_info",
                    image_info,
                )));
            }

            if active_mode == ModeId::EDITOR {
                toolbar = toolbar
                    .item(self.build_simple_action_button(
                        "glass.nav.agent",
                        "sparkles",
                        "Toggle Agent Panel",
                        |window, cx| {
                            window.dispatch_action(zed_actions::assistant::Toggle.boxed_clone(), cx);
                        },
                    ))
                    .item(self.build_simple_action_button(
                        "glass.nav.search",
                        "magnifyingglass",
                        "Project Search",
                        |window, cx| {
                            window.dispatch_action(
                                workspace::ToggleProjectSearch.boxed_clone(),
                                cx,
                            );
                        },
                    ))
                    .item(self.build_simple_action_button(
                        "glass.nav.runtime",
                        "play.fill",
                        "Runtime Actions",
                        |window, cx| {
                            window.dispatch_action(
                                app_runtime_ui::OpenRuntimeActions.boxed_clone(),
                                cx,
                            );
                        },
                    ))
                    .item(self.build_simple_action_button(
                        "glass.nav.diagnostics",
                        if diagnostics.error_count > 0 {
                            "xmark.circle"
                        } else if diagnostics.warning_count > 0 {
                            "exclamationmark.triangle"
                        } else {
                            "checkmark.circle"
                        },
                        "Project Diagnostics",
                        |window, cx| {
                            window.dispatch_action(
                                workspace::ToggleProjectDiagnostics.boxed_clone(),
                                cx,
                            );
                        },
                    ))
                    .item(self.build_simple_action_button(
                        "glass.nav.debugger",
                        "ladybug",
                        "Toggle Debug Panel",
                        |window, cx| {
                            window.dispatch_action(
                                zed_actions::debug_panel::Toggle.boxed_clone(),
                                cx,
                            );
                        },
                    ));

                if let Some(item) = self.build_lsp_button_item(cx) {
                    toolbar = toolbar.item(item);
                }
            }
        }

        if let Some(item) = self.build_connection_status_item(cx) {
            toolbar = toolbar.item(item);
        }

        if show_update {
            toolbar = toolbar.item(self.build_simple_action_button(
                "glass.update",
                "arrow.down.circle",
                "Restart to Update",
                |_window, cx| {
                    workspace::reload(cx);
                },
            ));
        }

        if !signed_in && title_bar_settings.show_sign_in {
            toolbar = toolbar.item(self.build_sign_in_item());
        }

        toolbar = toolbar
            .item(NativeToolbarItem::Space)
            .item(self.build_user_menu_item(&user, cx));

        window.set_native_toolbar(Some(toolbar));
    }

    fn build_simple_action_button(
        &self,
        id: &'static str,
        icon: &'static str,
        tool_tip: &'static str,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> NativeToolbarItem {
        NativeToolbarItem::Button(
            NativeToolbarButton::new(id, "")
                .tool_tip(tool_tip)
                .icon(icon)
                .on_click(move |_: &NativeToolbarClickEvent, window, cx| on_click(window, cx)),
        )
    }

    fn build_mode_switcher_item(&self, active_mode: ModeId) -> NativeToolbarItem {
        let (label, icon) = match active_mode {
            ModeId::BROWSER => ("Browser", "globe"),
            ModeId::EDITOR => ("Editor", "doc.text"),
            ModeId::TERMINAL => ("Terminal", "terminal"),
            _ => ("Browser", "globe"),
        };

        let workspace = self.workspace.clone();
        NativeToolbarItem::MenuButton(
            NativeToolbarMenuButton::new(
                "glass.mode_switcher",
                label,
                vec![
                    NativeToolbarMenuItem::action("Browser").icon("globe"),
                    NativeToolbarMenuItem::action("Editor").icon("doc.text"),
                    NativeToolbarMenuItem::action("Terminal").icon("terminal"),
                ],
            )
            .tool_tip("Switch Mode")
            .icon(icon)
            .shows_indicator(true)
            .on_select(move |event: &NativeToolbarMenuButtonSelectEvent, window, cx| {
                let mode = match event.index {
                    0 => Some(ModeId::BROWSER),
                    1 => Some(ModeId::EDITOR),
                    2 => Some(ModeId::TERMINAL),
                    _ => None,
                };
                if let Some(mode) = mode
                    && let Some(workspace) = workspace.upgrade()
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.switch_to_mode(mode, window, cx);
                    });
                }
            }),
        )
    }

    fn build_project_button_item(&self, cx: &App) -> Option<NativeToolbarItem> {
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
                            title_bar.show_recent_projects_popover(window, cx);
                        });
                    }
                }),
        ))
    }

    fn build_branch_button_item(&self, cx: &App) -> Option<NativeToolbarItem> {
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
                            title_bar.show_branch_popover(window, cx);
                        });
                    }
                }),
        ))
    }

    fn build_lsp_button_item(&self, _cx: &Context<Self>) -> Option<NativeToolbarItem> {
        self.right_item_view::<language_tools::lsp_button::LspButton>()?;
        let workspace = self.workspace.clone();
        Some(NativeToolbarItem::Button(
            NativeToolbarButton::new("glass.status.language_servers", "")
                .tool_tip("Language Servers")
                .icon("bolt")
                .on_click(move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.show_lsp_menu(window, cx);
                        });
                    }
                }),
        ))
    }

    fn build_omnibox_item(&self) -> NativeToolbarItem {
        let workspace_for_change = self.workspace.clone();
        let workspace_for_submit = self.workspace.clone();
        let workspace_for_move_up = self.workspace.clone();
        let workspace_for_move_down = self.workspace.clone();
        let workspace_for_cancel = self.workspace.clone();
        let workspace_for_begin = self.workspace.clone();
        let workspace_for_end = self.workspace.clone();

        NativeToolbarItem::SearchField(
            NativeToolbarSearchField::new("glass.omnibox")
                .placeholder("Search or enter URL")
                .text(SharedString::from(
                    self.native_toolbar_state.omnibox_text.clone(),
                ))
                .min_width(px(300.0))
                .max_width(px(600.0))
                .on_change(move |event: &NativeToolbarSearchEvent, window, cx| {
                    if let Some(workspace) = workspace_for_change.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        let text = event.text.clone();
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.native_toolbar_state.omnibox_text = text.clone();
                            title_bar.native_toolbar_state.omnibox_selected_index = None;
                            title_bar.native_toolbar_state.omnibox_suggestions.clear();
                            title_bar.show_search_suggestion_menu(window);
                            title_bar.search_history(text, cx);
                            cx.notify();
                        });
                    }
                })
                .on_submit(move |event: &NativeToolbarSearchEvent, window, cx| {
                    window.dismiss_native_search_suggestion_menu();
                    if let Some(workspace) = workspace_for_submit.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        let text = event.text.clone();
                        title_bar.update(cx, |title_bar, cx| {
                            if let Some(selected) =
                                title_bar.native_toolbar_state.omnibox_selected_index
                                && let Some(url) = title_bar.url_for_selected_row(selected)
                            {
                                title_bar.native_toolbar_state.omnibox_selected_index = None;
                                title_bar.navigate_omnibox(&url, cx);
                                return;
                            }
                            title_bar.native_toolbar_state.omnibox_selected_index = None;
                            title_bar.navigate_omnibox(&text, cx);
                        });
                    }
                })
                .on_move_down(move |_event, window, cx| {
                    if let Some(workspace) = workspace_for_move_down.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            let total = title_bar.omnibox_row_count();
                            if total == 0 {
                                return;
                            }
                            title_bar.native_toolbar_state.omnibox_selected_index =
                                Some(match title_bar.native_toolbar_state.omnibox_selected_index {
                                    Some(index) => (index + 1) % total,
                                    None => 0,
                                });
                            title_bar.show_search_suggestion_menu(window);
                            cx.notify();
                        });
                    }
                })
                .on_move_up(move |_event, window, cx| {
                    if let Some(workspace) = workspace_for_move_up.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            let total = title_bar.omnibox_row_count();
                            if total == 0 {
                                return;
                            }
                            title_bar.native_toolbar_state.omnibox_selected_index =
                                Some(match title_bar.native_toolbar_state.omnibox_selected_index {
                                    Some(0) | None => total.saturating_sub(1),
                                    Some(index) => index - 1,
                                });
                            title_bar.show_search_suggestion_menu(window);
                            cx.notify();
                        });
                    }
                })
                .on_cancel(move |_event, window, cx| {
                    window.dismiss_native_search_suggestion_menu();
                    window.blur_native_field_editor();
                    if let Some(workspace) = workspace_for_cancel.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.native_toolbar_state.omnibox_focused = false;
                            title_bar.native_toolbar_state.omnibox_selected_index = None;
                            cx.notify();
                        });
                    }
                })
                .on_begin_editing(move |_event, _window, cx| {
                    if let Some(workspace) = workspace_for_begin.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, _cx| {
                            title_bar.native_toolbar_state.omnibox_focused = true;
                        });
                    }
                })
                .on_end_editing(move |_event, window, cx| {
                    window.dismiss_native_search_suggestion_menu();
                    if let Some(workspace) = workspace_for_end.upgrade()
                        && let Some(title_bar) = workspace
                            .read(cx)
                            .titlebar_item()
                            .and_then(|item| item.downcast::<TitleBar>().ok())
                    {
                        title_bar.update(cx, |title_bar, cx| {
                            title_bar.native_toolbar_state.omnibox_focused = false;
                            title_bar.native_toolbar_state.omnibox_selected_index = None;
                            cx.notify();
                        });
                    }
                }),
        )
    }

    fn build_back_item(&self) -> NativeToolbarItem {
        let workspace = self.workspace.clone();
        self.build_simple_action_button(
            "glass.browser.back",
            "chevron.left",
            "Go Back",
            move |_window, cx| {
                if let Some(workspace) = workspace.upgrade()
                    && let Some(browser_view) = workspace
                        .read(cx)
                        .get_mode_view(ModeId::BROWSER)
                        .and_then(|view| view.downcast::<BrowserView>().ok())
                {
                    browser_view.update(cx, |browser_view, cx| {
                        if let Some(tab) = browser_view.active_tab() {
                            tab.update(cx, |tab, _| tab.go_back());
                        }
                    });
                }
            },
        )
    }

    fn build_forward_item(&self) -> NativeToolbarItem {
        let workspace = self.workspace.clone();
        self.build_simple_action_button(
            "glass.browser.forward",
            "chevron.right",
            "Go Forward",
            move |_window, cx| {
                if let Some(workspace) = workspace.upgrade()
                    && let Some(browser_view) = workspace
                        .read(cx)
                        .get_mode_view(ModeId::BROWSER)
                        .and_then(|view| view.downcast::<BrowserView>().ok())
                {
                    browser_view.update(cx, |browser_view, cx| {
                        if let Some(tab) = browser_view.active_tab() {
                            tab.update(cx, |tab, _| tab.go_forward());
                        }
                    });
                }
            },
        )
    }

    fn build_reload_item(&self) -> NativeToolbarItem {
        let workspace = self.workspace.clone();
        self.build_simple_action_button(
            "glass.browser.reload",
            "arrow.clockwise",
            "Reload",
            move |_window, cx| {
                if let Some(workspace) = workspace.upgrade()
                    && let Some(browser_view) = workspace
                        .read(cx)
                        .get_mode_view(ModeId::BROWSER)
                        .and_then(|view| view.downcast::<BrowserView>().ok())
                {
                    browser_view.update(cx, |browser_view, cx| {
                        if let Some(tab) = browser_view.active_tab() {
                            tab.update(cx, |tab, _| tab.reload());
                        }
                    });
                }
            },
        )
    }

    fn build_downloads_item(&self) -> NativeToolbarItem {
        let workspace = self.workspace.clone();
        self.build_simple_action_button(
            "glass.browser.downloads",
            "arrow.down.circle",
            "Downloads",
            move |window, cx| {
                if let Some(workspace) = workspace.upgrade()
                    && let Some(title_bar) = workspace
                        .read(cx)
                        .titlebar_item()
                        .and_then(|item| item.downcast::<TitleBar>().ok())
                {
                    title_bar.update(cx, |title_bar, cx| {
                        title_bar.show_downloads_panel(window, cx);
                    });
                }
            },
        )
    }

    fn build_restricted_mode_item(&self, cx: &App) -> Option<NativeToolbarItem> {
        let has_restricted_worktrees = TrustedWorktrees::try_get_global(cx)
            .map(|trusted_worktrees| {
                trusted_worktrees
                    .read(cx)
                    .has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx)
            })
            .unwrap_or(false);
        has_restricted_worktrees.then(|| {
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

    fn build_project_host_item(&self, cx: &App) -> Option<NativeToolbarItem> {
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

    fn build_connection_status_item(&self, _cx: &App) -> Option<NativeToolbarItem> {
        match &*self.client.status().borrow() {
            ClientStatus::ConnectionError
            | ClientStatus::ConnectionLost
            | ClientStatus::Reauthenticating
            | ClientStatus::Reconnecting
            | ClientStatus::ReconnectionError { .. } => Some(NativeToolbarItem::Button(
                NativeToolbarButton::new("glass.connection_status", "Disconnected")
                    .tool_tip("Disconnected")
                    .icon("wifi.exclamationmark"),
            )),
            ClientStatus::UpgradeRequired => Some(NativeToolbarItem::Button(
                NativeToolbarButton::new("glass.connection_status", "Update Required")
                    .tool_tip("Please Update to Collaborate")
                    .icon("exclamationmark.arrow.circlepath")
                    .on_click(|_, window, cx| {
                        auto_update::check(&Default::default(), window, cx);
                    }),
            )),
            _ => None,
        }
    }

    fn build_sign_in_item(&self) -> NativeToolbarItem {
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        NativeToolbarItem::Button(
            NativeToolbarButton::new("glass.sign_in", "Sign In")
                .tool_tip("Sign In")
                .icon("person.crop.circle.badge.plus")
                .on_click(move |_, window, cx| {
                    let client = client.clone();
                    let workspace = workspace.clone();
                    window
                        .spawn(cx, async move |mut cx| {
                            client
                                .sign_in_with_optional_connect(true, cx)
                                .await
                                .notify_workspace_async_err(workspace, &mut cx);
                        })
                        .detach();
                }),
        )
    }

    fn build_user_menu_item(&self, user: &Option<Arc<User>>, cx: &App) -> NativeToolbarItem {
        let show_update = self.update_version.read(cx).show_update_in_menu_bar();
        let signed_in = user.is_some();
        let user_login = user
            .as_ref()
            .map(|user| user.github_login.to_string())
            .unwrap_or_else(|| "Account".to_string());

        let mut menu_items = Vec::new();
        if signed_in {
            menu_items.push(NativeToolbarMenuItem::action(&user_login).enabled(false));
            menu_items.push(NativeToolbarMenuItem::separator());
        }
        if show_update {
            menu_items.push(
                NativeToolbarMenuItem::action("Restart to Update").icon("arrow.down.circle"),
            );
            menu_items.push(NativeToolbarMenuItem::separator());
        }
        menu_items.push(NativeToolbarMenuItem::action("Settings").icon("gearshape"));
        menu_items.push(NativeToolbarMenuItem::action("Keymap").icon("keyboard"));
        menu_items.push(NativeToolbarMenuItem::action("Themes…").icon("paintbrush"));
        menu_items.push(NativeToolbarMenuItem::action("Icon Themes…").icon("photo"));
        menu_items.push(NativeToolbarMenuItem::action("Extensions").icon("puzzlepiece.extension"));
        if signed_in {
            menu_items.push(NativeToolbarMenuItem::separator());
            menu_items.push(
                NativeToolbarMenuItem::action("Sign Out")
                    .icon("rectangle.portrait.and.arrow.right"),
            );
        }

        let mut menu_button = NativeToolbarMenuButton::new("glass.user_menu", "Account", menu_items)
            .tool_tip("User Menu")
            .shows_indicator(false);
        if TitleBarSettings::get_global(cx).show_user_picture
            && let Some(user) = user
        {
            let avatar_url = user.avatar_uri.to_string();
            if !avatar_url.is_empty() {
                menu_button = menu_button.image_url(avatar_url).image_circular(true);
            } else {
                menu_button = menu_button.icon("person.crop.circle");
            }
        } else {
            menu_button = menu_button.icon("person.crop.circle");
        }

        let client = self.client.clone();
        NativeToolbarItem::MenuButton(
            menu_button.on_select(move |event, window, cx| {
                let show_update_offset = usize::from(show_update);
                let signed_in_offset = usize::from(signed_in);
                let base = show_update_offset + signed_in_offset;

                if signed_in && event.index == 0 {
                    cx.open_url(&zed_urls::account_url(cx));
                    return;
                }

                if show_update && event.index == signed_in_offset {
                    workspace::reload(cx);
                    return;
                }

                match event.index.saturating_sub(base) {
                    0 => window.dispatch_action(zed_actions::OpenSettings.boxed_clone(), cx),
                    1 => window.dispatch_action(zed_actions::OpenKeymap.boxed_clone(), cx),
                    2 => window.dispatch_action(
                        zed_actions::theme_selector::Toggle::default().boxed_clone(),
                        cx,
                    ),
                    3 => window.dispatch_action(
                        zed_actions::icon_theme_selector::Toggle::default().boxed_clone(),
                        cx,
                    ),
                    4 => window.dispatch_action(zed_actions::Extensions::default().boxed_clone(), cx),
                    5 if signed_in => {
                        let client = client.clone();
                        window.spawn(cx, async move |mut cx| {
                            client.sign_out(&mut cx).await;
                        }).detach();
                    }
                    _ => {}
                }
            }),
        )
    }

    fn browser_view(&self, cx: &App) -> Option<Entity<BrowserView>> {
        let workspace = self.workspace.upgrade()?;
        let view = workspace.read(cx).get_mode_view(ModeId::BROWSER)?;
        view.downcast::<BrowserView>().ok()
    }

    fn active_tab_is_new_tab_page(&self, cx: &App) -> bool {
        self.browser_view(cx)
            .and_then(|browser_view| {
                let browser_view = browser_view.read(cx);
                browser_view
                    .active_tab()
                    .map(|tab| tab.read(cx).is_new_tab_page())
            })
            .unwrap_or(false)
    }

    fn sync_omnibox_url(&mut self, cx: &mut App) {
        if self.native_toolbar_state.omnibox_focused {
            return;
        }

        if let Some(url) = self.browser_view(cx).and_then(|browser_view| {
            let browser_view = browser_view.read(cx);
            browser_view
                .active_tab()
                .map(|tab| tab.read(cx).url().to_string())
        }) {
            let omnibox_text = display_omnibox_text(&url);
            if self.native_toolbar_state.omnibox_text != omnibox_text {
                self.native_toolbar_state.omnibox_text = omnibox_text;
            }
        }
    }

    fn navigate_omnibox(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let url = text_to_url(text);
        self.native_toolbar_state.omnibox_text = url.clone();
        self.native_toolbar_state.omnibox_focused = false;
        self.native_toolbar_state.omnibox_suggestions.clear();

        if let Some(browser_view) = self.browser_view(cx) {
            browser_view.update(cx, |browser_view, cx| {
                if let Some(tab) = browser_view.active_tab() {
                    tab.update(cx, |tab, cx| {
                        tab.navigate(&url, cx);
                    });
                }
            });
        }

        cx.notify();
    }

    pub(crate) fn refresh_status_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active_item = self
            .active_pane
            .as_ref()
            .and_then(|pane| pane.read(cx).active_item());

        self.native_toolbar_state.status_encoding = None;
        self.native_toolbar_state.status_line_ending = None;
        self.native_toolbar_state.status_toolchain = None;
        self.native_toolbar_state.status_image_info = None;
        self.native_toolbar_state.active_editor_subscription = None;
        self.native_toolbar_state.active_image_subscription = None;

        if let Some(item) = active_item {
            if let Some(editor) = item.act_as::<Editor>(cx) {
                self.native_toolbar_state.active_editor_subscription = Some(cx.subscribe_in(
                    &editor,
                    window,
                    |_this, _editor, event, _window, cx| {
                        if matches!(
                            event,
                            EditorEvent::SelectionsChanged { .. } | EditorEvent::BufferEdited
                        ) {
                            cx.notify();
                        }
                    },
                ));

                let (encoding, line_ending) = editor.update(cx, |editor, cx| {
                    let mut encoding = None;
                    let mut line_ending = None;

                    if let Some((_, buffer, _)) = editor.active_excerpt(cx) {
                        let buffer = buffer.read(cx);
                        let active_encoding = buffer.encoding();
                        if active_encoding != encoding_rs::UTF_8 || buffer.has_bom() {
                            let mut text = active_encoding.name().to_string();
                            if buffer.has_bom() {
                                text.push_str(" (BOM)");
                            }
                            encoding = Some(text);
                        }

                        let current_line_ending = buffer.line_ending();
                        if current_line_ending != LineEnding::Unix {
                            line_ending = Some(current_line_ending.label().to_string());
                        }
                    }

                    (encoding, line_ending)
                });

                self.native_toolbar_state.status_encoding = encoding;
                self.native_toolbar_state.status_line_ending = line_ending;
            }

            if let Some(toolchain) = self.right_item_view::<toolchain_selector::ActiveToolchain>() {
                self.native_toolbar_state.status_toolchain = toolchain
                    .read(cx)
                    .active_toolchain_name()
                    .map(ToOwned::to_owned);
            }

            if let Some(image_view) = item.act_as::<ImageView>(cx) {
                if let Some(metadata) = image_view.read(cx).image_metadata(cx) {
                    self.native_toolbar_state.status_image_info =
                        Some(Self::format_image_metadata(&metadata, cx));
                } else {
                    self.native_toolbar_state.active_image_subscription =
                        Some(cx.observe(&image_view, |title_bar, image_view, cx| {
                            if let Some(metadata) = image_view.read(cx).image_metadata(cx) {
                                title_bar.native_toolbar_state.status_image_info =
                                    Some(Self::format_image_metadata(&metadata, cx));
                                cx.notify();
                            }
                        }));
                }
            }
        }
    }

    fn format_image_metadata(metadata: &ImageMetadata, cx: &App) -> String {
        let settings = image_viewer::ImageViewerSettings::get_global(cx);
        let mut components = Vec::new();
        components.push(format!("{}x{}", metadata.width, metadata.height));
        components.push(util::size::format_file_size(
            metadata.file_size,
            matches!(settings.unit, image_viewer::ImageFileSizeUnit::Decimal),
        ));
        components.push(
            match metadata.format {
                ImageFormat::Png => "PNG",
                ImageFormat::Jpeg => "JPEG",
                ImageFormat::Gif => "GIF",
                ImageFormat::WebP => "WebP",
                ImageFormat::Tiff => "TIFF",
                ImageFormat::Bmp => "BMP",
                ImageFormat::Ico => "ICO",
                ImageFormat::Avif => "Avif",
                _ => "Unknown",
            }
            .to_string(),
        );
        components.join(" • ")
    }

    fn omnibox_row_count(&self) -> usize {
        let mut count = self.native_toolbar_state.omnibox_suggestions.len();
        if !self.native_toolbar_state.omnibox_text.is_empty() {
            count += 1;
        }
        count
    }

    fn url_for_selected_row(&self, index: usize) -> Option<String> {
        let mut current = 0;
        if !self.native_toolbar_state.omnibox_text.is_empty() {
            if current == index {
                return Some(text_to_url(&self.native_toolbar_state.omnibox_text));
            }
            current += 1;
        }

        for suggestion in &self.native_toolbar_state.omnibox_suggestions {
            if current == index {
                return Some(suggestion.url.clone());
            }
            current += 1;
        }

        None
    }

    fn show_search_suggestion_menu(&self, window: &mut Window) {
        if self.omnibox_row_count() == 0 {
            window.dismiss_native_search_suggestion_menu();
            return;
        }

        let workspace = self.workspace.clone();
        let selected = self.native_toolbar_state.omnibox_selected_index;
        let mut items: Vec<NativePopoverContentItem> = Vec::new();
        let mut row_count = 0usize;
        let mut row_index = 0usize;
        let has_search_row = !self.native_toolbar_state.omnibox_text.is_empty();

        if has_search_row {
            let query = self.native_toolbar_state.omnibox_text.clone();
            let search_workspace = workspace.clone();
            items.push(
                NativePopoverClickableRow::new(format!("Search \"{}\"", query))
                    .icon("magnifyingglass")
                    .detail("Google")
                    .selected(selected == Some(row_index))
                    .on_click(move |window, cx| {
                        window.dismiss_native_search_suggestion_menu();
                        let url = text_to_url(&query);
                        if let Some(workspace) = search_workspace.upgrade()
                            && let Some(title_bar) = workspace
                                .read(cx)
                                .titlebar_item()
                                .and_then(|item| item.downcast::<TitleBar>().ok())
                        {
                            title_bar.update(cx, |title_bar, cx| {
                                title_bar.navigate_omnibox(&url, cx);
                            });
                        }
                    })
                    .into(),
            );
            row_count += 1;
            row_index += 1;
            items.push(NativePopoverContentItem::separator());
        }

        items.push(NativePopoverContentItem::heading("History"));
        for suggestion in &self.native_toolbar_state.omnibox_suggestions {
            let url = suggestion.url.clone();
            let navigate_workspace = workspace.clone();
            let title = if suggestion.title.is_empty() {
                suggestion.url.clone()
            } else {
                suggestion.title.clone()
            };
            let detail = extract_domain(&suggestion.url);
            items.push(
                NativePopoverClickableRow::new(title)
                    .icon("clock")
                    .detail(detail)
                    .selected(selected == Some(row_index))
                    .on_click(move |window, cx| {
                        window.dismiss_native_search_suggestion_menu();
                        if let Some(workspace) = navigate_workspace.upgrade()
                            && let Some(title_bar) = workspace
                                .read(cx)
                                .titlebar_item()
                                .and_then(|item| item.downcast::<TitleBar>().ok())
                        {
                            title_bar.update(cx, |title_bar, cx| {
                                title_bar.navigate_omnibox(&url, cx);
                            });
                        }
                    })
                    .into(),
            );
            row_count += 1;
            row_index += 1;
        }

        let content_height = (row_count as f64 * 28.0)
            + 28.0
            + if has_search_row { 12.0 } else { 0.0 }
            + 32.0;
        let menu = NativeSearchSuggestionMenu::new(450.0, content_height.min(400.0))
            .items(items);
        window.update_native_search_suggestion_menu(
            menu,
            NativeSearchFieldTarget::ToolbarItem("glass.omnibox".into()),
        );
    }

    fn show_downloads_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser_view) = self.browser_view(cx) else {
            return;
        };

        let (downloads, is_incognito_window) =
            browser_view.read_with(cx, |browser_view, _| {
                (
                    browser_view.download_items(),
                    browser_view.is_incognito_window(),
                )
            });

        let mut items = vec![NativePopoverContentItem::heading("Downloads")];
        let mut row_count = 1usize;

        if is_incognito_window {
            items.push(NativePopoverContentItem::small_label(
                "Incognito downloads stay in memory for this window only.".to_string(),
            ));
            row_count += 1;
            if !downloads.is_empty() {
                items.push(NativePopoverContentItem::separator());
                row_count += 1;
            }
        }

        if downloads.is_empty() {
            items.push(NativePopoverContentItem::small_label(
                "No downloads yet.".to_string(),
            ));
            row_count += 1;
        } else {
            for download in downloads {
                let browser_view = browser_view.clone();
                let detail = if download.is_incognito {
                    format!("{} • Incognito", download.status_text)
                } else {
                    download.status_text.clone()
                };

                if download.is_complete && download.full_path.is_some() {
                    let id = download.id;
                    items.push(
                        NativePopoverClickableRow::new(download.display_name)
                            .icon("arrow.down.doc")
                            .detail(detail)
                            .on_click(move |window, cx| {
                                window.dismiss_native_popover();
                                browser_view.update(cx, |browser_view, cx| {
                                    browser_view.open_download_with_system(id, cx);
                                });
                            })
                            .into(),
                    );
                } else {
                    items.push(NativePopoverContentItem::small_label(format!(
                        "{} — {}",
                        download.display_name, detail
                    )));
                }
                row_count += 1;
            }
        }

        window.dismiss_native_popover();
        window.show_native_popover(
            NativePopover::new(360.0, (row_count as f64 * 28.0 + 32.0).clamp(120.0, 360.0))
                .behavior(NativePopoverBehavior::Transient)
                .items(items),
            NativePopoverAnchor::ToolbarItem("glass.browser.downloads".into()),
        );
    }

    fn search_history(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(entries) = self
            .browser_view(cx)
            .map(|browser_view| browser_view.read(cx).history().read(cx).entries().to_vec())
        else {
            return;
        };

        let executor = cx.background_executor().clone();
        let requested_query = query.clone();
        cx.spawn(async move |this, cx| {
            let matches = browser::history::BrowserHistory::search(entries, query, 8, executor).await;
            let _ = cx.update(|cx| {
                let _ = this.update(cx, |title_bar, cx| {
                    if title_bar.native_toolbar_state.omnibox_text != requested_query {
                        return;
                    }
                    title_bar.native_toolbar_state.omnibox_suggestions = matches;
                    title_bar.native_toolbar_state.omnibox_panel_dirty = true;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn show_recent_projects_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn show_branch_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn show_lsp_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lsp_button) = self.right_item_view::<language_tools::lsp_button::LspButton>() else {
            return;
        };
        let menu = lsp_button.update(cx, |lsp_button, cx| {
            lsp_button.ensure_toolbar_menu(window, cx)
        });
        let Some(menu) = menu else {
            return;
        };
        self.open_toolbar_context_menu(
            menu,
            "glass.status.language_servers",
            window,
            cx,
        );
    }

    fn open_toolbar_context_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        item_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item_id: SharedString = item_id.into();

        if self.native_toolbar_state.toolbar_overlay_item_id.as_ref() == Some(&item_id)
            && self
                .native_toolbar_state
                .toolbar_overlay_menu
                .as_ref()
                .is_some_and(|open| open == &menu)
        {
            self.close_toolbar_overlay(cx);
            return;
        }

        let Some(bounds) = window.native_toolbar_item_bounds(item_id.as_ref()) else {
            return;
        };

        let anchor = point(bounds.origin.x, bounds.origin.y + bounds.size.height + px(4.0));
        let subscription = cx.subscribe(&menu, |title_bar, _, _: &DismissEvent, cx| {
            title_bar.close_toolbar_overlay(cx);
        });

        self.native_toolbar_state.toolbar_overlay_menu = Some(menu.clone());
        self.native_toolbar_state.toolbar_overlay_anchor = Some(anchor);
        self.native_toolbar_state.toolbar_overlay_item_id = Some(item_id);
        self.native_toolbar_state.toolbar_overlay_subscription = Some(subscription);

        let focus_handle = menu.focus_handle(cx);
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| {
                window.focus(&focus_handle, cx);
            });
        });
        cx.notify();
    }

}

fn text_to_url(text: &str) -> String {
    if text.starts_with("http://") || text.starts_with("https://") {
        text.to_string()
    } else if text.contains('.') && !text.contains(' ') {
        format!("https://{text}")
    } else {
        let encoded: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
        format!("https://www.google.com/search?q={encoded}")
    }
}

fn display_omnibox_text(url: &str) -> String {
    if url == "glass://newtab" {
        String::new()
    } else {
        url.to_string()
    }
}

fn extract_domain(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}
