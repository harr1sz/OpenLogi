//! Eight-slot Actions Ring editor for the active device.

mod action_icons;
mod editor;
#[cfg(test)]
mod tests;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, StatefulInteractiveElement as _, Styled, Subscription, Window, div,
    prelude::FluentBuilder as _, px, rgb, svg,
};
use gpui_base::Button as BaseButton;
use gpui_component::{
    Icon, IconName, Selectable as _, button::Button, h_flex, tooltip::Tooltip, v_flex,
};
use openlogi_core::binding::{
    ActionRingConfig, ActionRingEntry, ActionRingIcon, ActionRingLayout, ActionRingSlot,
};
use openlogi_ui::action_icons::RING_CANCEL_ICON;

use self::action_icons::action_icon_path;
use self::editor::action_library;
use crate::features::binding_editor::custom::CustomActionInputs;
use crate::state::{
    AppState, BindingEditorKind, BindingEditorScope, DeviceKey, DeviceRecord, StateEvent,
    StateEvents,
};
use crate::ui::action::localized_action_label;
use crate::ui::theme::{self, Palette, Typography as _};

/// Stateful Actions Ring editor. Ring configuration itself lives in
/// [`AppState`]; this entity owns selection and editor input state.
pub struct ActionRingPanel {
    focus_handle: FocusHandle,
    selected_slot: ActionRingSlot,
    custom_inputs: Option<CustomActionInputs>,
    current_device_key: Option<DeviceKey>,
    editing_scope: Option<String>,
    library_scroll: ScrollHandle,
    #[expect(dead_code, reason = "held to keep the AppState subscription alive")]
    state_obs: Subscription,
}

/// The scope and slot that owned one frame's ring editor commands.
#[derive(Clone)]
struct RingEditorTarget {
    scope: Option<BindingEditorScope>,
    slot: ActionRingSlot,
    view: Entity<ActionRingPanel>,
}

impl RingEditorTarget {
    fn is_current(&self, cx: &App) -> bool {
        self.view.read(cx).selected_slot == self.slot
            && self.scope.as_ref().is_some_and(|scope| {
                AppState::try_read(cx).is_some_and(|state| scope.is_current(state))
            })
    }
}

impl ActionRingPanel {
    /// Create the editor and repaint it after any config/device change.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let state_obs =
            AppState::repaint_on(cx, |event| matches!(event, StateEvent::BindingsChanged(_)));
        Self {
            focus_handle: cx.focus_handle(),
            selected_slot: ActionRingSlot::Top,
            custom_inputs: None,
            current_device_key: None,
            editing_scope: None,
            library_scroll: ScrollHandle::new(),
            state_obs,
        }
    }

    fn sync_custom_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CustomActionInputs {
        let inputs = self
            .custom_inputs
            .get_or_insert_with(|| CustomActionInputs::new(window, cx))
            .clone();
        let device_key = AppState::try_read(cx)
            .and_then(|state| state.current_record().map(DeviceRecord::device_key));
        let scope = AppState::try_read(cx)
            .and_then(|state| state.editing_action_ring_app().map(str::to_owned));
        if self.current_device_key != device_key || self.editing_scope != scope {
            inputs.clear(window, cx);
            self.current_device_key = device_key;
            self.editing_scope = scope;
        }
        inputs.localize(window, cx);
        inputs
    }
}

impl Focusable for ActionRingPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ActionRingPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette(cx);
        let (ring, layout) = action_ring_editor_state(cx);
        let haptics_supported = current_device_supports_haptics(cx);
        let inputs = self.sync_custom_inputs(window, cx);
        let view = cx.entity();
        let target = RingEditorTarget {
            scope: AppState::try_read(cx)
                .and_then(|state| state.binding_editor_scope(BindingEditorKind::ActionRing)),
            slot: self.selected_slot,
            view: view.clone(),
        };

        v_flex()
            .w_full()
            .gap_4()
            .tab_group()
            .track_focus(&self.focus_handle)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_subheading()
                            .child(tr!("action_ring.actions_ring")),
                    )
                    .child(
                        div()
                            .text_caption()
                            .text_color(pal.text_muted)
                            .child(tr!("action_ring.action_ring_description")),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_center()
                    .gap_4()
                    .child(ring_preview(&layout, self.selected_slot, &view, pal))
                    .child(action_library(
                        &target,
                        layout.slots.get(&self.selected_slot),
                        &inputs,
                        &self.library_scroll,
                        pal,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .child(div().text_body().child(tr!("action_ring.actions_ring")))
                            .child(
                                div()
                                    .text_caption()
                                    .text_color(pal.text_muted)
                                    .child(tr!("action_ring.open_at_the_current_cursor_position")),
                            ),
                    )
                    .child(toggle_button(
                        "ring-enabled",
                        ring.enabled,
                        target.scope.as_ref(),
                        |state| state.current_action_ring().enabled,
                        AppState::commit_action_ring_enabled,
                    )),
            )
            .when(haptics_supported, |panel| {
                panel.child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            v_flex()
                                .child(div().text_body().child(tr!("action_ring.haptic_feedback")))
                                .child(
                                    div()
                                        .text_caption()
                                        .text_color(pal.text_muted)
                                        .child(tr!("action_ring.action_ring_haptic_description")),
                                ),
                        )
                        .child(toggle_button(
                            "ring-haptics",
                            ring.haptics,
                            target.scope.as_ref(),
                            |state| state.current_action_ring().haptics,
                            AppState::commit_action_ring_haptics,
                        )),
                )
            })
    }
}

fn action_ring_editor_state(cx: &Context<ActionRingPanel>) -> (ActionRingConfig, ActionRingLayout) {
    AppState::try_read(cx).map_or_else(
        || {
            let ring = ActionRingConfig::default();
            let layout = ring.default.clone();
            (ring, layout)
        },
        |state| {
            let ring = state.current_action_ring();
            let layout = state.current_action_ring_layout();
            (ring, layout)
        },
    )
}

fn current_device_supports_haptics(cx: &Context<ActionRingPanel>) -> bool {
    AppState::try_read(cx).is_some_and(|state| {
        state.current_record().is_some_and(|record| {
            record
                .capabilities
                .unwrap_or_else(|| {
                    openlogi_core::device::Capabilities::presumed_from_kind(record.kind)
                })
                .haptic_feedback
        })
    })
}

fn toggle_button(
    id: &'static str,
    enabled: bool,
    scope: Option<&BindingEditorScope>,
    read: impl Fn(&AppState) -> bool + 'static,
    commit: impl Fn(&mut AppState, bool) -> StateEvents + 'static,
) -> impl IntoElement {
    let scope = scope.cloned();
    div()
        .debug_selector(move || id.to_string())
        .flex_none()
        .child(
            Button::new(id)
                .compact()
                .label(if enabled {
                    tr!("common.on")
                } else {
                    tr!("common.off")
                })
                .selected(enabled)
                .on_click(move |_, _, cx| {
                    AppState::apply(cx, |state| {
                        if !scope.as_ref().is_some_and(|scope| scope.is_current(state)) {
                            return StateEvents::none();
                        }
                        let enabled = read(state);
                        commit(state, !enabled)
                    });
                }),
        )
}

const PREVIEW_SIZE: f32 = 320.0;
const PREVIEW_RADIUS: f32 = 106.0;
const PREVIEW_SLOT_SIZE: f32 = 50.0;

fn ring_preview(
    layout: &ActionRingLayout,
    selected_slot: ActionRingSlot,
    view: &Entity<ActionRingPanel>,
    pal: Palette,
) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(PREVIEW_SIZE))
        .child(
            div()
                .absolute()
                .left(px(24.0))
                .top(px(24.0))
                .size(px(PREVIEW_SIZE - 48.0))
                .rounded_full()
                .border_1()
                .border_color(pal.border)
                .bg(pal.panel),
        )
        .child(
            div()
                .absolute()
                .left(px(PREVIEW_SIZE / 2.0 - 24.0))
                .top(px(PREVIEW_SIZE / 2.0 - 24.0))
                .size(px(48.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(pal.muted)
                .text_color(pal.text_muted)
                .child(svg().path(RING_CANCEL_ICON).size(px(20.0)).flex_none()),
        )
        .children(ActionRingSlot::ALL.into_iter().map(|slot| {
            slot_button(
                slot,
                layout.slots.get(&slot),
                selected_slot == slot,
                view,
                pal,
            )
        }))
}

fn slot_button(
    slot: ActionRingSlot,
    entry: Option<&ActionRingEntry>,
    selected: bool,
    view: &Entity<ActionRingPanel>,
    pal: Palette,
) -> impl IntoElement {
    let index = slot.index();
    let (left, top) = slot.placement(PREVIEW_SIZE, PREVIEW_RADIUS, PREVIEW_SLOT_SIZE);
    let label = entry.map_or_else(
        || tr!("action_ring.empty_slot").to_string(),
        |entry| localized_action_label(entry.action()).to_string(),
    );
    let icon_path = entry.map(|entry| {
        entry.custom_icon().map_or_else(
            || action_icon_path(entry.action()),
            ActionRingIcon::asset_path,
        )
    });
    let accessible_label = label.clone();
    let selected_view = view.clone();

    BaseButton::new(("action-ring-slot", index))
        .debug_selector(move || format!("action-ring-slot-{index}"))
        .selected(selected)
        .absolute()
        .left(px(left))
        .top(px(top))
        .size(px(PREVIEW_SLOT_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(if selected {
            rgb(theme::ACCENT_BLUE).into()
        } else {
            pal.border
        })
        .bg(if selected {
            theme::accent_tint()
        } else {
            pal.control
        })
        .text_color(if selected {
            pal.text_primary
        } else {
            pal.text_muted
        })
        .cursor_pointer()
        .accessibility_label(accessible_label)
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .when_some(icon_path, |button, path| {
            button.child(svg().path(path).size(px(20.0)).text_color(if selected {
                pal.text_primary
            } else {
                pal.text_muted
            }))
        })
        .when(icon_path.is_none(), |button| {
            button.child(Icon::new(IconName::Plus).size_4())
        })
        .hover(move |button| {
            button.bg(if selected {
                theme::accent_tint_hover()
            } else {
                pal.control_hover
            })
        })
        .focus_visible(move |button| {
            button
                .border_color(rgb(theme::ACCENT_BLUE))
                .bg(if selected {
                    theme::accent_tint_hover()
                } else {
                    pal.control_hover
                })
        })
        .on_click(move |_, window, cx| {
            selected_view.update(cx, |panel, cx| {
                if panel.selected_slot != slot
                    && let Some(inputs) = &panel.custom_inputs
                {
                    inputs.clear(window, cx);
                }
                panel.selected_slot = slot;
                cx.notify();
            });
        })
}
