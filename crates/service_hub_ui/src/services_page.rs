use std::collections::BTreeMap;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, SharedString, WeakEntity,
    Window,
};
use service_hub::{ServiceHub, ServiceProviderDescriptor};
use ui::{
    Button, ButtonSize, ButtonStyle, Color, ContextMenu, DropdownMenu, DropdownStyle, Icon,
    IconName, Label, LabelSize, prelude::*,
};
use workspace::Workspace;
use workspace::item::{Item, ItemBufferKind, ItemEvent};
use workspace_chrome::SidebarRow;

use crate::services_provider::{
    ServiceWorkspacePane, ServicesPageState, build_service_workspace_panes,
    collect_provider_descriptors, normalize_services_page_state,
};

pub struct ServicesPage {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    providers: Vec<ServiceProviderDescriptor>,
    panes: BTreeMap<String, ServiceWorkspacePane>,
    state: ServicesPageState,
}

impl ServicesPage {
    pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
        if let Some(existing) = workspace.item_of_type::<Self>(cx) {
            workspace.activate_item(&existing, true, true, window, cx);
            return;
        }

        let page = Self::new(workspace, None, window, cx);
        workspace.add_item_to_active_pane(Box::new(page), None, true, window, cx);
    }

    fn new(
        workspace: &mut Workspace,
        initial_state: Option<ServicesPageState>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let panes = build_service_workspace_panes(ServiceHub::default().providers(), window, cx);
        let providers = collect_provider_descriptors(&panes);
        let state = normalize_services_page_state(&providers, initial_state);
        let workspace_handle = workspace.weak_handle();

        let page = cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            workspace: workspace_handle,
            providers,
            panes,
            state,
        });

        page.update(cx, |page, cx| {
            page.normalize_active_provider_state();
            page.refresh_provider(window, cx);
        });
        page
    }

    pub(crate) fn workspace(&self) -> &WeakEntity<Workspace> {
        &self.workspace
    }

    pub(crate) fn with_provider_mut<R>(
        &mut self,
        provider_id: &str,
        callback: impl FnOnce(&mut ServiceWorkspacePane, &mut ServicesPageState) -> R,
    ) -> Option<R> {
        let pane = self.panes.get_mut(provider_id)?;
        Some(callback(pane, &mut self.state))
    }

    fn provider(&self) -> &ServiceProviderDescriptor {
        self.providers
            .iter()
            .find(|provider| provider.id == self.state.provider_id)
            .expect("selected provider should stay valid")
    }

    fn active_pane(&self) -> &ServiceWorkspacePane {
        self.panes
            .get(&self.state.provider_id)
            .expect("selected provider pane should stay valid")
    }

    fn normalize_active_provider_state(&mut self) {
        let provider_id = self.state.provider_id.clone();
        self.with_provider_mut(&provider_id, |pane, state| {
            pane.normalize_state(state);
        });
    }

    fn refresh_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider_id = self.state.provider_id.clone();
        self.with_provider_mut(&provider_id, |pane, state| {
            pane.refresh(state, window, cx);
        });
    }

    fn select_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.provider_id == provider_id {
            return;
        }

        self.state.provider_id = provider_id;
        self.state.navigation_id = self.provider().shell.default_navigation_item_id.clone();
        self.state.selected_resource_id = None;
        self.normalize_active_provider_state();
        cx.emit(ItemEvent::UpdateTab);
        self.refresh_provider(window, cx);
    }

    fn select_navigation(&mut self, navigation_id: String, cx: &mut Context<Self>) {
        if self.state.navigation_id == navigation_id {
            return;
        }

        self.state.navigation_id = navigation_id;
        self.normalize_active_provider_state();
        cx.notify();
    }

    fn select_resource(
        &mut self,
        resource_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider_id = self.state.provider_id.clone();
        self.with_provider_mut(&provider_id, |pane, state| {
            pane.select_resource(state, resource_id, window, cx);
        });
    }

    fn open_in_new_tab(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };

        let initial_state = self.state.clone();
        workspace.update(cx, |workspace, cx| {
            let page = Self::new(workspace, Some(initial_state.clone()), window, cx);
            workspace.add_item_to_active_pane(Box::new(page), None, true, window, cx);
        });
    }

    fn render_provider_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = cx.entity().downgrade();
        let menu = ContextMenu::build(window, cx, |mut menu, _, _| {
            for provider in &self.providers {
                let provider_id = provider.id.clone();
                let page = page.clone();
                menu = menu.entry(provider.label.clone(), None, move |window, cx| {
                    page.update(cx, |this, cx| {
                        this.select_provider(provider_id.clone(), window, cx);
                    })
                    .ok();
                });
            }

            menu
        });

        DropdownMenu::new(
            "services-provider-menu",
            self.provider().label.clone(),
            menu,
        )
        .style(DropdownStyle::Outlined)
        .trigger_size(ButtonSize::Default)
        .full_width(true)
    }

    fn render_resource_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let resource_menu = self.active_pane().resource_menu(&self.state)?;
        let page = cx.entity().downgrade();
        let menu = ContextMenu::build(window, cx, |mut menu, _, _| {
            for entry in &resource_menu.entries {
                let resource_id = entry.id.clone();
                let label = match &entry.detail {
                    Some(detail) => format!("{} ({detail})", entry.label),
                    None => entry.label.clone(),
                };
                let page = page.clone();
                menu = menu.entry(label, None, move |window, cx| {
                    page.update(cx, |this, cx| {
                        this.select_resource(resource_id.clone(), window, cx);
                    })
                    .ok();
                });
            }

            menu
        });

        Some(
            v_flex()
                .flex_1()
                .gap_1()
                .child(
                    Label::new(resource_menu.singular_label.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    DropdownMenu::new("services-resource-menu", resource_menu.current_label, menu)
                        .style(DropdownStyle::Outlined)
                        .trigger_size(ButtonSize::Default)
                        .full_width(true)
                        .disabled(resource_menu.disabled),
                ),
        )
    }

    fn render_shell_header(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().title_bar_background)
            .child(
                h_flex()
                    .justify_between()
                    .items_end()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(Label::new(self.provider().label.clone()).size(LabelSize::Large))
                            .child(
                                Label::new(
                                    "Switch providers, resources, and service areas without leaving the editor.",
                                )
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("services-open-new-tab", "Open in New Tab")
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_in_new_tab(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("services-refresh", "Refresh")
                                    .style(ButtonStyle::Filled)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_provider(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                Label::new("Provider")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(self.render_provider_menu(window, cx)),
                    )
                    .when_some(self.render_resource_menu(window, cx), |this, resource_menu| {
                        this.child(resource_menu)
                    }),
            )
    }

    fn render_navigation_sidebar(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_56()
            .min_w(rems(12.))
            .h_full()
            .p_3()
            .gap_3()
            .border_r_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().background)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Service Areas")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(self.provider().label.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Hidden),
                    ),
            )
            .children(
                self.provider()
                    .shell
                    .navigation_items
                    .iter()
                    .map(|navigation_item| {
                        SidebarRow::new(
                            format!("services-nav-{}", navigation_item.id),
                            navigation_item.label.clone(),
                            Self::navigation_icon(&navigation_item.id),
                        )
                        .selected(self.state.navigation_id == navigation_item.id)
                        .on_click({
                            let navigation_id = navigation_item.id.clone();
                            cx.listener(move |this, _, _window, cx| {
                                this.select_navigation(navigation_id.clone(), cx);
                            })
                        })
                    }),
            )
    }

    fn render_provider_content(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.active_pane().render_section(&self.state, window, cx)
    }

    fn navigation_icon(navigation_id: &str) -> IconName {
        match navigation_id {
            "overview" => IconName::Info,
            "builds" => IconName::BoltOutlined,
            _ => IconName::Globe,
        }
    }
}

impl EventEmitter<ItemEvent> for ServicesPage {}

impl Focusable for ServicesPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for ServicesPage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        self.provider().label.clone().into()
    }

    fn tab_tooltip_text(&self, _cx: &App) -> Option<SharedString> {
        Some("Services".into())
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Server))
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Services Page Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn buffer_kind(&self, _cx: &App) -> ItemBufferKind {
        ItemBufferKind::None
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event)
    }
}

impl Render for ServicesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(self.render_shell_header(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_navigation_sidebar(window, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .p_5()
                            .child(self.render_provider_content(window, cx)),
                    ),
            )
    }
}
