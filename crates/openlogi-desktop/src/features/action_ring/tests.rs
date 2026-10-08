//! Exercise the actual ring editor, including the shared inputs' owner changes.

use super::*;
use crate::{
    services::{assets::AssetResolver, i18n::LOCALE_LOCK},
    state::Sources,
};
use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext, point, size};
use openlogi_core::{binding::Action, config::Config, device::DeviceKind};

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn type_shortcut(text: &str, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds("custom-shortcut-add").unwrap();
    cx.simulate_click(
        point(bounds.left() + px(20.), bounds.center().y),
        Modifiers::default(),
    );
    cx.simulate_input(text);
    draw(cx);
}

fn add_shortcut(cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds("custom-shortcut-add").unwrap();
    cx.simulate_click(
        point(bounds.right() - px(16.), bounds.center().y),
        Modifiers::default(),
    );
    draw(cx);
}

fn click_without_frame(position: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    use gpui::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};

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

fn install(cx: &mut TestAppContext) -> Entity<AppState> {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        let profile: openlogi_fixture::DeviceProfile =
            serde_json::from_str(openlogi_fixture::CANONICAL_DEVICE_PROFILE_JSON).unwrap();
        let resolver = AssetResolver::new();
        let (commands, _received) = tokio::sync::mpsc::unbounded_channel();
        let state = cx.new(|_| {
            let mut state = AppState::new(Sources {
                inventories: &profile.inventories,
                ..Sources::in_memory(Config::ephemeral(), &resolver, commands)
            });
            let index = state
                .devices()
                .iter()
                .position(|record| {
                    record.kind == DeviceKind::Mouse && record.online && record.route.is_some()
                })
                .unwrap();
            let _ = state.select_device(index);
            let _ = state.commit_action_ring_slot(ActionRingSlot::Top, None);
            state
        });
        AppState::set_global(state.clone(), cx);
        state
    })
}

fn assert_draft_cleared(state: &Entity<AppState>, cx: &mut VisualTestContext) {
    assert!(cx.debug_bounds("custom-shortcut-error").is_none());
    let before = state.read_with(cx, |state, _| state.current_action_ring());
    add_shortcut(cx);
    assert_eq!(
        state.read_with(cx, |state, _| state.current_action_ring()),
        before,
        "an empty draft must not mutate the newly selected owner"
    );
}

#[gpui::test]
fn ring_custom_drafts_follow_slot_device_and_application(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let state = install(cx);
    let (panel, cx) = cx.add_window_view(|_, cx| ActionRingPanel::new(cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    draw(cx);
    assert!(cx.debug_bounds("custom-shortcut-hold").is_none());
    assert!(cx.debug_bounds("custom-shortcut-tap").is_none());
    type_shortcut("Ctrl+P", cx);
    add_shortcut(cx);
    let expected = Action::CustomShortcut("Ctrl+P".parse().unwrap());
    state.read_with(cx, |state, _| {
        assert_eq!(
            state.current_action_ring_layout().slots[&ActionRingSlot::Top].action(),
            &expected
        );
    });

    type_shortcut("not-a-key", cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    let bounds = cx.debug_bounds("action-ring-slot-2").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.selected_slot),
        ActionRingSlot::Right
    );
    assert_draft_cleared(&state, cx);

    type_shortcut("not-a-key", cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    cx.update(|_, cx| {
        AppState::apply(cx, |state| {
            state.set_editing_action_ring_app(Some("org.openlogi.review-app".into()))
        });
    });
    draw(cx);
    assert_draft_cleared(&state, cx);

    let (old, next) = state.read_with(cx, |state, _| {
        let old = state.selected_device_index().unwrap();
        let next = state
            .devices()
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != old
                    && record.kind == DeviceKind::Mouse
                    && record.online
                    && record.route.is_some()
            })
            .unwrap()
            .0;
        (old, next)
    });
    // Keep both devices in the same scope so this isolates the device guard.
    cx.update(|_, cx| AppState::apply(cx, |state| state.set_editing_action_ring_app(None)));
    draw(cx);
    assert!(state.read_with(cx, |state, _| state.editing_action_ring_app().is_none()));
    type_shortcut("not-a-key", cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    cx.update(|_, cx| AppState::apply(cx, |state| state.select_device(next)));
    assert!(state.read_with(cx, |state, _| state.editing_action_ring_app().is_none()));
    draw(cx);
    assert_draft_cleared(&state, cx);
    cx.update(|_, cx| AppState::apply(cx, |state| state.select_device(old)));
    cx.update(|_, cx| AppState::apply(cx, |state| state.set_editing_action_ring_app(None)));
    state.read_with(cx, |state, _| {
        assert_eq!(
            state.current_action_ring_layout().slots[&ActionRingSlot::Top].action(),
            &expected
        );
    });
}

#[gpui::test]
fn clearing_a_ring_slot_discards_custom_drafts(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let state = install(cx);
    let (_, cx) = cx.add_window_view(|_, cx| ActionRingPanel::new(cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    draw(cx);
    type_shortcut("Ctrl+P", cx);
    add_shortcut(cx);
    type_shortcut("not-a-key", cx);
    assert!(cx.debug_bounds("custom-shortcut-error").is_some());
    let clear = cx.debug_bounds("ring-clear-slot").unwrap();
    cx.simulate_click(clear.center(), Modifiers::default());
    draw(cx);
    assert!(!state.read_with(cx, |state, _| {
        state
            .current_action_ring_layout()
            .slots
            .contains_key(&ActionRingSlot::Top)
    }));
    assert_draft_cleared(&state, cx);
}

#[gpui::test]
fn ring_stale_frame_draft_cannot_cross_device_or_application(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let _state = install(cx);
    let (_, cx) = cx.add_window_view(|_, cx| ActionRingPanel::new(cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    for change_device in [false, true] {
        cx.update(|_, cx| {
            AppState::apply(cx, |state| state.set_editing_action_ring_app(None));
        });
        draw(cx);
        type_shortcut("Ctrl+P", cx);
        let row = cx.debug_bounds("custom-shortcut-add").unwrap();
        let submit = point(row.right() - px(16.), row.center().y);
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
                                && record.kind == DeviceKind::Mouse
                                && record.online
                                && record.route.is_some()
                        })
                        .unwrap()
                        .0;
                    state.select_device(next)
                } else {
                    state.set_editing_action_ring_app(Some("org.openlogi.stale-frame".into()))
                }
            });
            let before = AppState::try_read(cx).unwrap().current_action_ring();
            click_without_frame(submit, window, cx);
            assert_eq!(
                AppState::try_read(cx).unwrap().current_action_ring(),
                before
            );
        });
        draw(cx);
    }
}

#[gpui::test]
fn ring_stale_frame_clear_cannot_modify_the_previous_slot(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let state = install(cx);
    let (_, cx) = cx.add_window_view(|_, cx| ActionRingPanel::new(cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    draw(cx);
    type_shortcut("Ctrl+P", cx);
    add_shortcut(cx);
    let next = cx.debug_bounds("action-ring-slot-2").unwrap();
    let clear = cx.debug_bounds("ring-clear-slot").unwrap();
    let before = state.read_with(cx, |state, _| state.current_action_ring());
    cx.update(|window, cx| {
        click_without_frame(next.center(), window, cx);
        click_without_frame(clear.center(), window, cx);
    });
    assert_eq!(
        state.read_with(cx, |state, _| state.current_action_ring()),
        before
    );
}

#[gpui::test]
fn ring_stale_frame_toggles_use_current_state_and_device(cx: &mut TestAppContext) {
    let _locale = LOCALE_LOCK.lock().unwrap();
    rust_i18n::set_locale("en");
    let state = install(cx);
    let (_, cx) = cx.add_window_view(|_, cx| ActionRingPanel::new(cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    for id in ["ring-enabled", "ring-haptics"] {
        draw(cx);
        let toggle = cx.debug_bounds(id).unwrap();
        let before = state.read_with(cx, |state, _| state.current_action_ring());
        cx.update(|window, cx| {
            click_without_frame(toggle.center(), window, cx);
            click_without_frame(toggle.center(), window, cx);
        });
        assert_eq!(
            state.read_with(cx, |state, _| state.current_action_ring()),
            before
        );
        cx.update(|window, cx| {
            AppState::apply(cx, |state| {
                let current = state.selected_device_index().unwrap();
                let next = state
                    .devices()
                    .iter()
                    .enumerate()
                    .find(|(index, record)| {
                        *index != current
                            && record.kind == DeviceKind::Mouse
                            && record.online
                            && record.route.is_some()
                    })
                    .unwrap()
                    .0;
                state.select_device(next)
            });
            let before = AppState::try_read(cx).unwrap().current_action_ring();
            click_without_frame(toggle.center(), window, cx);
            assert_eq!(
                AppState::try_read(cx).unwrap().current_action_ring(),
                before
            );
        });
        cx.update(|_, cx| AppState::apply(cx, |state| state.select_device(0)));
    }
}
