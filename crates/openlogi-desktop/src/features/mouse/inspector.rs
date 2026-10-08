//! Fixed binding inspector for the Buttons workspace.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Role,
    StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px, rgb, svg,
};
use gpui_base::Button as BaseButton;
use gpui_component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _, button::Button, h_flex,
    input::InputState, scroll::ScrollableElement as _, v_flex,
};
use openlogi_core::binding::{Action, ButtonId, GestureDirection, default_binding};

use super::hotspots::MouseControlId;
use super::thumbwheel::ThumbwheelPreset;
use super::view::{ButtonPress, MouseEditorTarget, MouseModelView};
use crate::features::binding_editor::custom::{CustomActionInputs, ShortcutMode, ShortcutModes};
use crate::features::binding_editor::{
    GESTURE_BUTTON_ICON, PickFn, action_icon_path, action_rows_matching, editor_section,
    gesture_direction_icon,
};
use crate::state::{AppState, StateEvents};
use crate::ui::action::localized_action_label;
use crate::ui::components::{MenuRow, control_button, control_input};
use crate::ui::theme::{self, ACCENT_BLUE, Palette, Typography as _};

pub(super) const INSPECTOR_W: f32 = 328.;

#[derive(Clone, Copy)]
pub(super) struct BindingInspectorData<'a> {
    pub target: &'a MouseEditorTarget,
    pub selected: Option<MouseControlId>,
    pub gesture_direction: Option<GestureDirection>,
    pub action_picker_open: bool,
    pub button_press: ButtonPress,
    pub shortcut_mode: ShortcutMode,
    pub custom_inputs: &'a CustomActionInputs,
    pub bindings: &'a BTreeMap<ButtonId, Action>,
    pub gesture_maps: &'a BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>>,
    pub dpi_gestures: bool,
    pub editing_app: Option<&'a str>,
    pub overridden: Option<&'a BTreeMap<ButtonId, Action>>,
}

#[derive(Clone, Copy)]
struct ActionPickerContext<'a> {
    target: &'a MouseEditorTarget,
    open: bool,
    search: &'a Entity<InputState>,
    view: &'a Entity<MouseModelView>,
    mode: ShortcutMode,
    hold_available: bool,
    inputs: &'a CustomActionInputs,
}

pub(super) fn binding_inspector(
    data: BindingInspectorData<'_>,
    action_search: &Entity<InputState>,
    view: &Entity<MouseModelView>,
    cx: &Context<MouseModelView>,
) -> gpui::Div {
    let pal = theme::palette(cx);
    let picker = ActionPickerContext {
        target: data.target,
        open: data.action_picker_open,
        search: action_search,
        view,
        mode: data.shortcut_mode,
        hold_available: true,
        inputs: data.custom_inputs,
    };
    let body = match data.selected {
        None => empty_inspector(
            data.editing_app,
            data.overridden.map_or(0, BTreeMap::len),
            pal,
        ),
        Some(MouseControlId::ThumbwheelRotation) => thumbwheel_inspector(
            data.bindings,
            data.editing_app,
            data.overridden,
            picker,
            pal,
        ),
        Some(MouseControlId::Button(button)) => button_inspector(button, &data, picker, pal, cx),
    };

    v_flex()
        .debug_selector(|| "button-inspector".into())
        .w(px(INSPECTOR_W))
        .h_full()
        .min_h_0()
        .flex_shrink_0()
        .border_l_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .child(
            div()
                .id("button-inspector-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_4()
                .child(body),
        )
}

fn picker_action(
    picker: ActionPickerContext<'_>,
    mutate: impl Fn(&mut AppState, Action) -> StateEvents + 'static,
) -> PickFn {
    let observer = picker.view.clone();
    let target = picker.target.clone();
    let inputs = picker.inputs.clone();
    Rc::new(move |action, window, cx| {
        if !target.is_current(&observer, cx) {
            return;
        }
        inputs.clear(window, cx);
        AppState::apply(cx, |state| mutate(state, action));
        observer.update(cx, |view, cx| {
            view.close_action_picker();
            cx.notify();
        });
    })
}

fn empty_inspector(app: Option<&str>, override_count: usize, pal: Palette) -> gpui::Div {
    let summary = match (app, override_count) {
        (Some(app), 0) => tr!(
            "profiles.app_profile_no_overrides",
            app => app.to_string()
        ),
        (Some(app), 1) => tr!(
            "profiles.app_profile_single_override",
            app => app.to_string()
        ),
        (Some(app), count) => tr!(
            "profiles.app_profile_override_count",
            app => app.to_string(),
            count => count.to_string()
        ),
        (None, _) => tr!("profiles.select_device_button_description"),
    };
    v_flex()
        .gap_3()
        .child(inspector_heading(
            tr!("actions.button_inspector"),
            None,
            pal,
        ))
        .child(div().text_body().text_color(pal.text_muted).child(summary))
}

fn button_inspector(
    button: ButtonId,
    data: &BindingInspectorData<'_>,
    picker: ActionPickerContext<'_>,
    pal: Palette,
    cx: &Context<MouseModelView>,
) -> gpui::Div {
    let gesture_map = data.gesture_maps.get(&button);
    let overridden = data
        .overridden
        .is_some_and(|overrides| overrides.contains_key(&button));
    if data.editing_app.is_none()
        && let Some(gesture_map) = gesture_map
    {
        return gesture_inspector(button, gesture_map, data.gesture_direction, picker, pal, cx);
    }
    if let Some(app) = data.editing_app
        && !overridden
        && gesture_map.is_some()
    {
        return inherited_gesture_inspector(button, app, picker, pal, cx);
    }

    let press = data.button_press;
    let pair = AppState::try_read(cx).and_then(|state| state.long_press_binding(button));
    let has_long_press = pair.is_some();
    let can_delay =
        AppState::try_read(cx).is_some_and(|state| state.is_button_press_delayable(button));
    // The short half of a pair executes on release, so it cannot hold a key.
    let picker = ActionPickerContext {
        hold_available: !has_long_press || press == ButtonPress::Long,
        ..picker
    };
    let action = match press {
        ButtonPress::Short => data
            .bindings
            .get(&button)
            .cloned()
            .unwrap_or_else(|| default_binding(button)),
        ButtonPress::Long => pair.map_or(Action::None, |pair| pair.long().clone()),
    };
    let status = match (
        data.editing_app,
        overridden,
        action == default_binding(button),
    ) {
        (Some(app), true, _) => tr!("actions.overridden_in_app", app => app.to_string()),
        (Some(_), false, _) => tr!("profiles.inherited_from_default"),
        (None, _, true) => tr!("pointer.device_default"),
        (None, _, false) => tr!("profiles.customized"),
    };
    let on_pick = picker_action(picker, move |state, action| match press {
        ButtonPress::Short => state.commit_binding(button, action),
        ButtonPress::Long => state.commit_long_binding(button, action),
    });

    v_flex()
        .gap_3()
        .child(inspector_heading(
            tr!(button.translation_key()),
            Some(status),
            pal,
        ))
        .when(
            AppState::try_read(cx).is_some_and(AppState::is_long_press_editable),
            |panel| panel.child(press_selector(button, press, picker, pal, cx)),
        )
        .child(current_action_card(&action, picker, pal))
        .when(overridden, |panel| {
            let observer = picker.view.clone();
            let target = picker.target.clone();
            panel.child(
                control_button("inspector-use-default")
                    .w_full()
                    .icon(IconName::Undo)
                    .label(tr!("profiles.use_the_default_profile"))
                    .on_click(move |_, _, cx| {
                        if !target.is_current(&observer, cx) {
                            return;
                        }
                        AppState::apply(cx, |state| state.clear_app_binding(button));
                        observer.update(cx, |view, cx| {
                            view.close_action_picker();
                            cx.notify();
                        });
                    }),
            )
        })
        .when(can_enable_gestures(button, data.editing_app), |panel| {
            panel.child(gesture_mode_control(
                button,
                data.dpi_gestures,
                can_delay,
                picker,
                pal,
            ))
        })
        .when(picker.open, |panel| {
            panel.child(action_library(
                "inspector-action",
                Some(&action),
                picker,
                &on_pick,
                pal,
                cx,
            ))
        })
}

fn gesture_mode_control(
    button: ButtonId,
    dpi_gestures: bool,
    can_delay: bool,
    picker: ActionPickerContext<'_>,
    pal: Palette,
) -> gpui::Div {
    let observer = picker.view.clone();
    let target = picker.target.clone();
    let unavailable = button == ButtonId::DpiToggle && !dpi_gestures;
    v_flex()
        .gap_3()
        .child(
            control_button("inspector-use-gestures")
                .w_full()
                .icon(Icon::empty().path(GESTURE_BUTTON_ICON))
                .label(tr!("actions.use_gestures"))
                .disabled(unavailable || !can_delay)
                .on_click(move |_, _, cx| {
                    if !target.is_current(&observer, cx) {
                        return;
                    }
                    AppState::apply(cx, |state| state.commit_gesture_mode(button, true));
                    observer.update(cx, |view, cx| {
                        view.set_gesture_selected_dir(Some(GestureDirection::Click));
                        cx.notify();
                    });
                }),
        )
        .when(unavailable, |panel| {
            panel.child(
                div()
                    .text_body()
                    .text_color(pal.text_muted)
                    .child(tr!("actions.dpi_gestures_unavailable")),
            )
        })
}

fn press_selector(
    button: ButtonId,
    selected: ButtonPress,
    picker: ActionPickerContext<'_>,
    pal: Palette,
    cx: &Context<MouseModelView>,
) -> impl IntoElement {
    let state = AppState::try_read(cx);
    let has_long_press = state.is_some_and(|state| state.long_press_binding(button).is_some());
    let can_delay = state.is_some_and(|state| state.is_button_press_delayable(button));
    let view = picker.view.clone();
    let target = picker.target.clone();
    v_flex()
        .gap_2()
        .child(
            h_flex().flex_wrap().gap_2().children(
                [
                    (
                        ButtonPress::Short,
                        "button-short-press",
                        tr!("actions.short_press"),
                    ),
                    (
                        ButtonPress::Long,
                        "button-long-press",
                        tr!("actions.long_press", duration => openlogi_core::binding::LONG_PRESS_THRESHOLD.as_millis()),
                    ),
                ]
                .map(|(press, id, label)| {
                    let view = view.clone();
                    div().debug_selector(move || id.to_string()).child(control_button(id)
                        .label(label)
                        .selected(selected == press)
                        .disabled(press == ButtonPress::Long && !can_delay)
                        .on_click(move |_, window, cx| {
                            view.update(cx, |view, cx| {
                                view.button_press = press;
                                view.close_action_picker();
                                view.custom_inputs.clear(window, cx);
                                cx.notify();
                            });
                        }))
                }),
            ),
        )
        .when(!can_delay, |panel| {
            panel.child(
                div().debug_selector(|| "hold-conversion-hint".into())
                    .text_body().text_color(pal.text_muted)
                    .child(tr!("actions.hold_conversion_requires_tap")),
            )
        })
        .when(has_long_press, |panel| {
            panel.child(
                control_button("button-remove-long-press")
                    .label(tr!("actions.use_a_single_action"))
                    .on_click(move |_, _, cx| {
                        if !target.is_current(&view, cx) {
                            return;
                        }
                        AppState::apply(cx, |state| state.clear_long_binding(button));
                        view.update(cx, |view, cx| {
                            view.button_press = ButtonPress::Short;
                            view.close_action_picker();
                            cx.notify();
                        });
                    }),
            )
        })
}

fn inherited_gesture_inspector(
    button: ButtonId,
    app: &str,
    picker: ActionPickerContext<'_>,
    pal: Palette,
    cx: &Context<MouseModelView>,
) -> gpui::Div {
    let on_pick = picker_action(picker, move |state, action| {
        state.commit_binding(button, action)
    });
    let edit_default = picker.view.clone();
    let target = picker.target.clone();
    v_flex()
        .gap_3()
        .child(inspector_heading(
            tr!(button.translation_key()),
            Some(tr!("profiles.inherited_from_default")),
            pal,
        ))
        .child(gesture_summary_card(picker, pal))
        .child(div().text_caption().text_color(pal.text_muted).child(tr!(
            "actions.app_profile_action_replaces_gestures",
            app => app.to_string()
        )))
        .child(
            Button::new("inspector-edit-default-gestures")
                .small()
                .w_full()
                .label(tr!("actions.edit_default_gestures"))
                .on_click(move |_, _, cx| {
                    if !target.is_current(&edit_default, cx) {
                        return;
                    }
                    AppState::apply(cx, |state| state.set_editing_app(None));
                    edit_default.update(cx, |view, cx| {
                        view.set_gesture_selected_dir(Some(GestureDirection::Click));
                        cx.notify();
                    });
                }),
        )
        .when(picker.open, |panel| {
            panel.child(action_library(
                "inspector-gesture-override",
                None,
                picker,
                &on_pick,
                pal,
                cx,
            ))
        })
}

fn gesture_inspector(
    button: ButtonId,
    gesture_map: &BTreeMap<GestureDirection, Action>,
    selected_direction: Option<GestureDirection>,
    picker: ActionPickerContext<'_>,
    pal: Palette,
    cx: &Context<MouseModelView>,
) -> gpui::Div {
    let direction = selected_direction.unwrap_or(GestureDirection::Click);
    let current = gesture_action(gesture_map, button, direction);
    // A gesture click is resolved on release; swipes still have a live press.
    let picker = ActionPickerContext {
        hold_available: direction != GestureDirection::Click,
        ..picker
    };
    let on_pick = picker_action(picker, move |state, action| {
        state.commit_gesture_binding(button, direction, action)
    });
    let turn_off = picker.view.clone();
    let target = picker.target.clone();

    v_flex()
        .gap_3()
        .child(inspector_heading(
            tr!(button.translation_key()),
            Some(tr!("actions.five_directions")),
            pal,
        ))
        .child(gesture_directions(
            direction,
            gesture_map,
            button,
            picker.view,
            pal,
        ))
        .child(current_action_card(&current, picker, pal))
        .child(
            control_button("inspector-single-action")
                .w_full()
                .label(tr!("actions.use_a_single_action"))
                .on_click(move |_, _, cx| {
                    if !target.is_current(&turn_off, cx) {
                        return;
                    }
                    AppState::apply(cx, |state| state.commit_gesture_mode(button, false));
                    turn_off.update(cx, |view, cx| {
                        view.set_gesture_selected_dir(None);
                        cx.notify();
                    });
                }),
        )
        .when(picker.open, |panel| {
            panel.child(action_library(
                "inspector-gesture-action",
                Some(&current),
                picker,
                &on_pick,
                pal,
                cx,
            ))
        })
}

fn gesture_directions(
    active: GestureDirection,
    gesture_map: &BTreeMap<GestureDirection, Action>,
    button: ButtonId,
    view: &Entity<MouseModelView>,
    pal: Palette,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(editor_section(tr!("actions.direction"), pal))
        .children(
            GestureDirection::ALL
                .into_iter()
                .enumerate()
                .map(|(index, direction)| {
                    let selected = direction == active;
                    let action = gesture_action(gesture_map, button, direction);
                    let view = view.clone();
                    MenuRow::new(("inspector-direction", index))
                        .selected(selected)
                        .role(Role::Button)
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_2()
                                // `.size_4()` is not decoration: a bare `Icon`
                                // falls through to the current font size, which
                                // would leave these a step under the 16px leading
                                // column the action rows below use.
                                .child(gesture_direction_icon(direction).size_4())
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_body()
                                                .child(tr!(direction.translation_key())),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_caption()
                                                .text_color(pal.text_muted)
                                                .child(localized_action_label(&action)),
                                        ),
                                ),
                        )
                        .when(selected, |row| {
                            row.child(
                                Icon::new(IconName::Check)
                                    .size_3()
                                    .text_color(rgb(ACCENT_BLUE)),
                            )
                        })
                        .on_click(move |_, _, cx| {
                            view.update(cx, |view, cx| {
                                view.set_gesture_selected_dir(Some(direction));
                                cx.notify();
                            });
                        })
                }),
        )
}

/// Whether the default-profile inspector may promote `button` into gesture
/// mode. Per-app bindings are single-action overrides, so they cannot carry a
/// direction map.
fn can_enable_gestures(button: ButtonId, editing_app: Option<&str>) -> bool {
    editing_app.is_none() && button.supports_gesture_mode()
}

fn thumbwheel_inspector(
    bindings: &BTreeMap<ButtonId, Action>,
    editing_app: Option<&str>,
    overridden: Option<&BTreeMap<ButtonId, Action>>,
    picker: ActionPickerContext<'_>,
    pal: Palette,
) -> gpui::Div {
    let backward = bindings
        .get(&ButtonId::ThumbwheelScrollDown)
        .cloned()
        .unwrap_or_else(|| default_binding(ButtonId::ThumbwheelScrollDown));
    let forward = bindings
        .get(&ButtonId::ThumbwheelScrollUp)
        .cloned()
        .unwrap_or_else(|| default_binding(ButtonId::ThumbwheelScrollUp));
    let current = ThumbwheelPreset::recognize(&backward, &forward);
    let is_overridden = overridden.is_some_and(|overrides| {
        overrides.contains_key(&ButtonId::ThumbwheelScrollDown)
            || overrides.contains_key(&ButtonId::ThumbwheelScrollUp)
    });
    let status = match (editing_app, is_overridden) {
        (Some(app), true) => tr!("actions.overridden_in_app", app => app.to_string()),
        (Some(_), false) => tr!("profiles.inherited_from_default"),
        (None, _) => tr!("profiles.default_profile"),
    };
    let current_label = current.map_or_else(
        || tr!("common.custom"),
        |preset| tr!(preset.translation_key()),
    );
    let current_icon = current.map_or("action-icons/chevrons-right.svg", ThumbwheelPreset::icon);

    v_flex()
        .gap_3()
        .child(inspector_heading(
            tr!("pointer.thumb_wheel"),
            Some(status),
            pal,
        ))
        .child(selection_card(
            "inspector-current-thumbwheel-preset",
            tr!("common.preset"),
            current_icon,
            current_label,
            picker,
            pal,
        ))
        .when(picker.open, |panel| {
            panel.child(thumbwheel_preset_rows(current, picker, pal))
        })
        .when(is_overridden, |panel| {
            let observer = picker.view.clone();
            let target = picker.target.clone();
            panel.child(
                Button::new("inspector-thumbwheel-use-default")
                    .small()
                    .w_full()
                    .icon(IconName::Undo)
                    .label(tr!("profiles.use_the_default_profile"))
                    .on_click(move |_, _, cx| {
                        if !target.is_current(&observer, cx) {
                            return;
                        }
                        AppState::apply(cx, AppState::clear_app_thumbwheel);
                        observer.update(cx, |view, cx| {
                            view.close_action_picker();
                            cx.notify();
                        });
                    }),
            )
        })
}

fn thumbwheel_preset_rows(
    current: Option<ThumbwheelPreset>,
    picker: ActionPickerContext<'_>,
    pal: Palette,
) -> gpui::Div {
    let observer = picker.view.clone();
    let target = picker.target.clone();
    v_flex()
        .gap_1()
        .child(editor_section(tr!("common.preset"), pal))
        .children(
            ThumbwheelPreset::ALL
                .into_iter()
                .enumerate()
                .map(|(index, preset)| {
                    let selected = current == Some(preset);
                    let observer = observer.clone();
                    let target = target.clone();
                    MenuRow::new(("inspector-thumbwheel", index))
                        .selected(selected)
                        .role(Role::Button)
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    svg()
                                        .path(preset.icon())
                                        .size_4()
                                        .text_color(pal.text_muted),
                                )
                                .child(div().child(tr!(preset.translation_key()))),
                        )
                        .when(selected, |row| {
                            row.child(
                                Icon::new(IconName::Check)
                                    .size_3()
                                    .text_color(rgb(ACCENT_BLUE)),
                            )
                        })
                        .on_click(move |_, _, cx| {
                            if !target.is_current(&observer, cx) {
                                return;
                            }
                            AppState::apply(cx, |state| state.commit_thumbwheel_preset(preset));
                            observer.update(cx, |view, cx| {
                                view.close_action_picker();
                                cx.notify();
                            });
                        })
                }),
        )
}

fn inspector_heading(
    title: gpui::SharedString,
    status: Option<gpui::SharedString>,
    pal: Palette,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_heading().child(title))
        .children(status.map(|status| {
            div()
                .text_caption()
                .text_color(pal.text_muted)
                .child(status)
        }))
}

fn current_action_card(
    action: &Action,
    picker: ActionPickerContext<'_>,
    pal: Palette,
) -> impl IntoElement {
    selection_card(
        "inspector-current-action",
        tr!("actions.current_action"),
        action_icon_path(action),
        localized_action_label(action),
        picker,
        pal,
    )
}

fn gesture_summary_card(picker: ActionPickerContext<'_>, pal: Palette) -> impl IntoElement {
    selection_card(
        "inspector-current-gesture-summary",
        tr!("actions.current_action"),
        GESTURE_BUTTON_ICON,
        tr!("actions.five_directions"),
        picker,
        pal,
    )
}

fn selection_card(
    id: &'static str,
    caption: gpui::SharedString,
    icon: &'static str,
    value: gpui::SharedString,
    picker: ActionPickerContext<'_>,
    pal: Palette,
) -> impl IntoElement {
    let toggle = picker.view.clone();
    let search = picker.search.clone();
    let opening = !picker.open;
    let accessible_label = value.clone();
    BaseButton::new(id)
        .debug_selector(move || id.to_string())
        .accessibility_label(accessible_label)
        .aria_expanded(picker.open)
        .flex()
        .flex_col()
        .items_stretch()
        .gap_2()
        .rounded(pal.control_radius)
        .border_1()
        .border_color(pal.border)
        .bg(pal.control)
        .p_3()
        .cursor_pointer()
        .hover(move |card| card.bg(pal.control_hover))
        .focus_visible(move |card| card.bg(pal.control_hover).border_color(rgb(ACCENT_BLUE)))
        .child(
            div()
                .text_caption()
                .text_color(pal.text_muted)
                .child(caption),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    h_flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            svg()
                                .path(icon)
                                .size_4()
                                .flex_none()
                                .text_color(pal.text_muted),
                        )
                        .child(div().min_w_0().truncate().text_body().child(value)),
                )
                .child(
                    svg()
                        .path(if picker.open {
                            "action-icons/chevrons-up.svg"
                        } else {
                            "action-icons/chevrons-down.svg"
                        })
                        .size_3()
                        .flex_none()
                        .text_color(pal.text_muted),
                ),
        )
        .on_click(move |_, window, cx| {
            if opening {
                search.update(cx, |search, cx| search.set_value("", window, cx));
            }
            toggle.update(cx, |view, cx| {
                view.toggle_action_picker(window, cx);
                cx.notify();
            });
        })
}

fn action_library(
    id_prefix: &'static str,
    current: Option<&Action>,
    picker: ActionPickerContext<'_>,
    on_pick: &PickFn,
    pal: Palette,
    cx: &Context<MouseModelView>,
) -> impl IntoElement {
    let query = picker.search.read(cx).value();
    let rows = action_rows_matching(id_prefix, current, &query, on_pick, pal);
    let observer = picker.view.clone();
    let mode_reader = observer.clone();
    let modes = picker.hold_available.then(|| ShortcutModes {
        selected: picker.mode,
        read: Rc::new(move |cx| mode_reader.read(cx).shortcut_mode),
        on_change: Rc::new(move |mode, cx| {
            observer.update(cx, |view, cx| {
                view.shortcut_mode = mode;
                cx.notify();
            });
        }),
    });
    v_flex()
        .gap_2()
        .pt_1()
        .child(picker.inputs.render(modes, on_pick, pal, cx))
        .child(editor_section(tr!("actions.actions"), pal))
        .child(control_input(picker.search).cleanable(true))
        .child(
            v_flex()
                .gap_0p5()
                .when(rows.is_empty(), |list| {
                    list.child(
                        div()
                            .py_3()
                            .text_body()
                            .text_color(pal.text_muted)
                            .child(tr!("actions.no_actions_found")),
                    )
                })
                .children(rows),
        )
}

fn gesture_action(
    gesture_map: &BTreeMap<GestureDirection, Action>,
    button: ButtonId,
    direction: GestureDirection,
) -> Action {
    gesture_map.get(&direction).cloned().unwrap_or_else(|| {
        if direction == GestureDirection::Click {
            default_binding(button)
        } else {
            Action::None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_offers_gestures_for_every_supported_button() {
        let supported: Vec<_> = ButtonId::ALL
            .into_iter()
            .filter(|button| can_enable_gestures(*button, None))
            .collect();

        assert_eq!(
            supported,
            vec![
                ButtonId::Back,
                ButtonId::Forward,
                ButtonId::DpiToggle,
                ButtonId::GestureButton,
                ButtonId::HapticPanel,
            ]
        );
    }

    #[test]
    fn per_app_profile_does_not_offer_forward_gesture_mode() {
        assert!(!can_enable_gestures(
            ButtonId::Forward,
            Some("com.apple.Safari")
        ));
    }
}
