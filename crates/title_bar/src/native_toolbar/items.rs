use gpui::{
    App, NativeToolbarButton, NativeToolbarClickEvent, NativeToolbarItem, NativeToolbarMenuButton,
    NativeToolbarMenuButtonSelectEvent, NativeToolbarMenuItem, Window,
};
use workspace_modes::ModeId;

use crate::TitleBar;

impl TitleBar {
    pub(crate) fn has_restricted_worktrees(&self, cx: &App) -> bool {
        project::trusted_worktrees::TrustedWorktrees::try_get_global(cx)
            .map(|trusted_worktrees| {
                trusted_worktrees
                    .read(cx)
                    .has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx)
            })
            .unwrap_or(false)
    }

    pub(super) fn build_simple_action_button(
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

    pub(crate) fn build_mode_switcher_item(&self, active_mode: ModeId) -> NativeToolbarItem {
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
            .on_select(
                move |event: &NativeToolbarMenuButtonSelectEvent, window, cx| {
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
                },
            ),
        )
    }
}
