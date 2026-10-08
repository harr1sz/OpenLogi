use gpui::{TestAppContext, size};
use openlogi_core::binding::default_binding;
use openlogi_core::config::Config;

use super::*;
use crate::services::assets::AssetResolver;
use crate::services::i18n::LOCALE_LOCK;
use crate::state::Sources;

fn install_app_state(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let resolver = AssetResolver::new();
        let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let state =
            cx.new(|_| AppState::new(Sources::in_memory(Config::ephemeral(), &resolver, commands)));
        AppState::set_global(state, cx);
    });
}

#[gpui::test]
fn long_bindings_stay_inside_their_label_card(cx: &mut TestAppContext) {
    // #1401: the card's Button base centres its children on the cross axis,
    // so without `items_stretch` the value row keeps its natural width and
    // overflows both edges once the binding name is wider than the card. The
    // English defaults are enough to trip it under the test text system
    // ("Forward (Button 5)" measures a 206px row over a 156px card). Pinned
    // to English under the lock: another test in this binary leaves the
    // process locale at zh-CN, whose labels are short enough to fit and
    // would have made this a false pass.
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    cx.update(gpui_component::init);
    install_app_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    // Wide enough for labels on both sides (`model_layout` hides them under 960).
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));

    // Selectors are `label-card-{MouseControlId:?}` / `label-value-row-{MouseControlId:?}`.
    for (card_selector, row_selector) in [
        (
            "label-card-Button(Forward)",
            "label-value-row-Button(Forward)",
        ),
        (
            "label-card-Button(MiddleClick)",
            "label-value-row-Button(MiddleClick)",
        ),
    ] {
        let card = cx
            .debug_bounds(card_selector)
            .expect("the synthetic model renders a label card for this control");
        let row = cx
            .debug_bounds(row_selector)
            .expect("the label card renders its value row");
        assert!(
            card.contains(&row.origin) && card.contains(&row.bottom_right()),
            "{row_selector}: value row {row:?} must sit inside its card {card:?}"
        );
    }

    drop(view);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

#[test]
fn narrow_model_keeps_every_callout_separate_and_inside_the_canvas() {
    for width in [960., 980., 1200.] {
        let layout = model_layout(None, width, 600., true);
        assert_eq!(layout.labels.len(), default_hotspots(true).len());
        for side in [
            super::super::leader_lines::Side::Left,
            super::super::leader_lines::Side::Right,
        ] {
            let mut cards: Vec<_> = layout
                .labels
                .iter()
                .filter(|label| label.side == side)
                .collect();
            cards.sort_by(|a, b| a.y.total_cmp(&b.y));
            for pair in cards.windows(2) {
                assert!(
                    pair[1].y - pair[0].y >= super::super::geometry::LABEL_H,
                    "{width}px: {:?} and {:?} overlap",
                    pair[0].id,
                    pair[1].id
                );
            }
            for card in cards {
                assert!(card.y + super::super::geometry::LABEL_H / 2. <= layout.canvas_h);
            }
        }
    }
}

#[gpui::test]
fn a_selected_gesture_can_render_in_the_binding_inspector(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    install_app_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    cx.run_until_parked();

    view.update(cx, |view, cx| {
        view.set_gesture_selected_dir(Some(GestureDirection::Up));
        let gesture_maps = BTreeMap::from([(
            ButtonId::MiddleClick,
            BTreeMap::from([(
                GestureDirection::Click,
                default_binding(ButtonId::MiddleClick),
            )]),
        )]);
        let bindings = BTreeMap::new();
        let entity = cx.entity();
        let target = view.editor_target(cx);

        binding_inspector(
            BindingInspectorData {
                target: &target,
                selected: Some(MouseControlId::Button(ButtonId::MiddleClick)),
                gesture_direction: Some(GestureDirection::Up),
                action_picker_open: false,
                button_press: view.button_press,
                shortcut_mode: view.shortcut_mode,
                custom_inputs: &view.custom_inputs,
                bindings: &bindings,
                gesture_maps: &gesture_maps,
                dpi_gestures: false,
                editing_app: None,
                overridden: None,
            },
            &view.action_search,
            &entity,
            cx,
        );
    });
    cx.run_until_parked();
    drop(view);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

#[gpui::test]
fn selected_button_and_its_custom_picker_render_in_the_owner_view(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    install_app_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    view.update(cx, |view, cx| {
        view.select(MouseControlId::Button(ButtonId::Back));
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.toggle_action_picker(window, cx);
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("custom-shortcut-add").is_some());
    assert!(cx.debug_bounds("custom-application-add").is_some());
    assert!(cx.debug_bounds("custom-shortcut-hold").is_some());
    let row = cx.debug_bounds("custom-shortcut-add").unwrap();
    cx.simulate_click(
        gpui::point(row.left() + px(20.), row.center().y),
        gpui::Modifiers::default(),
    );
    cx.simulate_input("not-a-key");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(!view.read_with(cx, |view, _| view.action_picker_open));
    assert!(cx.debug_bounds("custom-shortcut-add").is_none());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.toggle_action_picker(window, cx);
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("custom-shortcut-error").is_none());
}

#[gpui::test]
fn selecting_another_control_closes_the_action_picker(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    install_app_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    cx.run_until_parked();

    view.update(cx, |view, _| {
        view.selected = Some(MouseControlId::Button(ButtonId::Back));
        view.action_picker_open = true;

        view.select(MouseControlId::Button(ButtonId::Forward));

        assert!(!view.action_picker_open);
    });
    drop(view);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

#[test]
fn active_thumbwheel_directions_highlight_the_paired_control() {
    assert_eq!(
        MouseControlId::from_active_button(ButtonId::ThumbwheelScrollUp),
        MouseControlId::ThumbwheelRotation
    );
    assert_eq!(
        MouseControlId::from_active_button(ButtonId::ThumbwheelScrollDown),
        MouseControlId::ThumbwheelRotation
    );
}

#[test]
fn fallback_model_only_adds_thumbwheel_when_capability_is_measured() {
    let (_, _, without, _) = scaled_model(None, 560., 420., false, LabelDistribution::LeftOnly);
    let (_, _, with, _) = scaled_model(None, 560., 420., true, LabelDistribution::LeftOnly);
    assert_eq!(
        without
            .iter()
            .filter(|hotspot| hotspot.id == MouseControlId::ThumbwheelRotation)
            .count(),
        0
    );
    assert_eq!(
        with.iter()
            .filter(|hotspot| hotspot.id == MouseControlId::ThumbwheelRotation)
            .count(),
        1
    );
}

fn mouse_state(
    cx: &mut TestAppContext,
) -> (
    Entity<AppState>,
    tokio::sync::mpsc::UnboundedReceiver<crate::services::ipc::Command>,
) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        let profile: openlogi_fixture::DeviceProfile =
            serde_json::from_str(openlogi_fixture::CANONICAL_DEVICE_PROFILE_JSON).unwrap();
        let resolver = AssetResolver::new();
        let (commands, received) = tokio::sync::mpsc::unbounded_channel();
        let state = cx.new(|_| {
            let mut state = AppState::new(Sources {
                inventories: &profile.inventories,
                ..Sources::in_memory(Config::ephemeral(), &resolver, commands)
            });
            let index = state
                .devices()
                .iter()
                .position(|record| {
                    record.kind == openlogi_core::device::DeviceKind::Mouse
                        && record.route.is_some()
                })
                .unwrap();
            let _ = state.select_device(index);
            state
        });
        AppState::set_global(state.clone(), cx);
        (state, received)
    })
}

fn open_back_picker(
    view: &Entity<MouseModelView>,
    press: ButtonPress,
    cx: &mut gpui::VisualTestContext,
) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.select(MouseControlId::Button(ButtonId::Back));
            view.button_press = press;
            view.toggle_action_picker(window, cx);
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn type_custom_shortcut(text: &str, cx: &mut gpui::VisualTestContext) {
    let row = cx.debug_bounds("custom-shortcut-add").unwrap();
    cx.simulate_click(
        gpui::point(row.left() + px(20.), row.center().y),
        gpui::Modifiers::default(),
    );
    cx.simulate_input(text);
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn submit_custom_shortcut(cx: &mut gpui::VisualTestContext) {
    let row = cx.debug_bounds("custom-shortcut-add").unwrap();
    cx.simulate_click(
        gpui::point(row.right() - px(16.), row.center().y),
        gpui::Modifiers::default(),
    );
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn click_without_frame(position: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};

    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

#[gpui::test]
fn mouse_stale_frame_draft_cannot_cross_device_or_application(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let (_state, _received) = mouse_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    for change_device in [false, true] {
        cx.update(|_, cx| AppState::apply(cx, |state| state.set_editing_app(None)));
        open_back_picker(&view, ButtonPress::Short, cx);
        type_custom_shortcut("Ctrl+P", cx);
        let row = cx.debug_bounds("custom-shortcut-add").unwrap();
        let submit = gpui::point(row.right() - px(16.), row.center().y);
        // Mutate the owner and use the previous frame's mouse listener in one
        // update; simulate_click would allow an intervening redraw.
        cx.update(|window, cx| {
            AppState::apply(cx, |state| {
                if change_device {
                    let current = state.selected_device_index().unwrap();
                    let next = state
                        .devices()
                        .iter()
                        .enumerate()
                        .find(|(index, record)| {
                            *index != current
                                && record.kind == openlogi_core::device::DeviceKind::Mouse
                                && record.online
                                && record.route.is_some()
                        })
                        .unwrap()
                        .0;
                    state.select_device(next)
                } else {
                    state.set_editing_app(Some("org.openlogi.stale-frame".into()))
                }
            });
            let before = AppState::try_read(cx).unwrap().button_bindings().clone();
            click_without_frame(submit, window, cx);
            assert_eq!(
                AppState::try_read(cx).unwrap().button_bindings(),
                &before,
                "the old draft must not be saved to the newly selected owner"
            );
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

#[gpui::test]
fn mouse_stale_frame_draft_cannot_commit_after_control_or_direction_switch(
    cx: &mut TestAppContext,
) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let (state, _received) = mouse_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    for change_direction in [false, true] {
        if change_direction {
            cx.update(|_, cx| {
                AppState::apply(cx, |state| state.commit_gesture_mode(ButtonId::Back, true));
            });
        }
        open_back_picker(&view, ButtonPress::Short, cx);
        type_custom_shortcut("Ctrl+P", cx);
        let row = cx.debug_bounds("custom-shortcut-add").unwrap();
        let submit = gpui::point(row.right() - px(16.), row.center().y);
        let before = state.read_with(cx, |state, _| {
            (
                state.button_bindings().clone(),
                state.gesture_bindings().clone(),
            )
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                if change_direction {
                    view.set_gesture_selected_dir(Some(GestureDirection::Up));
                } else {
                    view.select(MouseControlId::Button(ButtonId::Forward));
                }
                cx.notify();
            });
            click_without_frame(submit, window, cx);
        });
        assert_eq!(
            state.read_with(cx, |state, _| {
                (
                    state.button_bindings().clone(),
                    state.gesture_bindings().clone(),
                )
            }),
            before
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

#[gpui::test]
fn mouse_switching_app_device_and_offline_mouse_discards_drafts(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let (state, _received) = mouse_state(cx);
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    open_back_picker(&view, ButtonPress::Short, cx);
    type_custom_shortcut("not-a-key", cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    cx.update(|_, cx| {
        AppState::apply(cx, |state| {
            state.set_editing_app(Some("org.openlogi.review-app".into()))
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(!view.read_with(cx, |view, _| view.action_picker_open));
    open_back_picker(&view, ButtonPress::Short, cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_none());

    // Both another physical online mouse and an offline paired mouse must
    // reset selection before the old inspector can commit anything.
    for online in [true, false] {
        type_custom_shortcut("not-a-key", cx);
        cx.update(|_, cx| {
            AppState::apply(cx, |state| {
                let current = state.current_record().unwrap().device_key();
                let next = state
                    .devices()
                    .iter()
                    .position(|record| {
                        record.kind == openlogi_core::device::DeviceKind::Mouse
                            && record.online == online
                            && record.device_key() != current
                    })
                    .unwrap();
                state.select_device(next)
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(view.read_with(cx, |view, _| view.selected.is_none()));
        assert!(!view.read_with(cx, |view, _| view.action_picker_open));
        open_back_picker(&view, ButtonPress::Short, cx);
        assert!(cx.debug_bounds("custom-shortcut-error").is_none());
    }
    assert!(state.read_with(cx, |state, _| { !state.current_record().unwrap().online }));
}

#[gpui::test]
fn mouse_adding_long_press_does_not_silently_break_existing_hold(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let (state, _received) = mouse_state(cx);
    cx.update(|cx| {
        AppState::apply(cx, |state| {
            state.commit_binding(
                ButtonId::Back,
                Action::HoldShortcut("Ctrl+A".parse().unwrap()),
            )
        });
    });
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    open_back_picker(&view, ButtonPress::Long, cx);
    type_custom_shortcut("Ctrl+P", cx);
    submit_custom_shortcut(cx);
    assert!(
        !view.read_with(cx, |view, _| view.action_picker_open),
        "the production save callback ran"
    );
    state.read_with(cx, |state, _| {
        assert_eq!(state.button_bindings().get(&ButtonId::Back), Some(&Action::HoldShortcut("Ctrl+A".parse().unwrap())));
        assert!(
            state.long_press_binding(ButtonId::Back).is_none_or(|pair| pair.short().held_combo().is_none()),
            "a short action fires only at release, so converting Single(Hold) must not preserve an unusable short Hold"
        );
    });
}

#[gpui::test]
fn hold_binding_explains_why_long_press_is_unavailable(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let (state, _) = mouse_state(cx);
    cx.update(|cx| {
        AppState::apply(cx, |state| {
            state.commit_binding(
                ButtonId::Back,
                Action::HoldShortcut("Ctrl+A".parse().unwrap()),
            )
        });
    });
    let (view, cx) = cx.add_window_view(MouseModelView::new);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.select(MouseControlId::Button(ButtonId::Back));
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("hold-conversion-hint").is_some());
    let tab = cx.debug_bounds("button-long-press").unwrap();
    cx.simulate_click(tab.center(), gpui::Modifiers::default());
    assert!(view.read_with(cx, |view, _| view.button_press == ButtonPress::Short));
    assert!(state.read_with(cx, |state, _| {
        state.long_press_binding(ButtonId::Back).is_none()
    }));
}
