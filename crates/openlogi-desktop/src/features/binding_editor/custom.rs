//! Shared custom-action inputs; the enclosing picker owns the draft lifetime.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, ParentElement, Styled, Window,
    div, prelude::FluentBuilder as _,
};
use gpui_component::{
    Disableable as _, Selectable as _, h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use openlogi_core::binding::{Action, ApplicationTarget, KeyCombo};

use super::{PickFn, editor_section};
use crate::ui::components::{control_button, control_input, localize_placeholder};
use crate::ui::theme::{Palette, Typography as _};

/// Whether a shortcut releases immediately or follows the physical button.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ShortcutMode {
    #[default]
    Tap,
    Hold,
}

impl ShortcutMode {
    fn action(self, combo: KeyCombo) -> Action {
        match self {
            Self::Tap => Action::CustomShortcut(combo),
            Self::Hold => Action::HoldShortcut(combo),
        }
    }
}

type ModeChanged = Rc<dyn Fn(ShortcutMode, &mut App)>;

/// Optional mode control. Ring slots have no physical release and use Tap only.
pub(crate) struct ShortcutModes {
    pub selected: ShortcutMode,
    /// Read on submission, after the owner has handled any intervening clicks.
    pub read: Rc<dyn Fn(&App) -> ShortcutMode>,
    pub on_change: ModeChanged,
}

/// Retained text inputs shared by mouse and Actions Ring editors.
#[derive(Clone)]
pub(crate) struct CustomActionInputs {
    shortcut: Entity<InputState>,
    application: Entity<InputState>,
}

impl CustomActionInputs {
    /// Retain inputs and repaint the owner as either draft changes.
    pub(crate) fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Self {
        let shortcut = cx.new(|cx| InputState::new(window, cx));
        let application = cx.new(|cx| InputState::new(window, cx));
        for input in [&shortcut, &application] {
            cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        Self {
            shortcut,
            application,
        }
    }

    /// Discard drafts when the enclosing picker changes scope.
    pub(crate) fn clear(&self, window: &mut Window, cx: &mut App) {
        for input in [&self.shortcut, &self.application] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Refresh cached placeholders after a language change.
    pub(crate) fn localize(&self, window: &mut Window, cx: &mut App) {
        localize_placeholder(
            &self.shortcut,
            tr!("action_ring.shortcut_e_g_cmd_plus_shift_plus_p"),
            window,
            cx,
        );
        localize_placeholder(
            &self.application,
            tr!("action_ring.application_folder_path_or_url"),
            window,
            cx,
        );
    }

    /// Render validated custom actions for the current picker.
    pub(crate) fn render(
        &self,
        modes: Option<ShortcutModes>,
        on_pick: &PickFn,
        pal: Palette,
        cx: &App,
    ) -> gpui::Div {
        let selected_mode = modes
            .as_ref()
            .map_or(ShortcutMode::Tap, |modes| modes.selected);
        let read_mode = modes.as_ref().map(|modes| modes.read.clone());
        let shortcut = self.shortcut.read(cx).value();
        let application = self.application.read(cx).value();
        let valid_shortcut = shortcut.trim().parse::<KeyCombo>().is_ok();
        let valid_application = ApplicationTarget::new(application.trim(), "").is_ok();
        v_flex()
            .gap_2()
            .child(editor_section(tr!("action_ring.custom_shortcut"), pal))
            .when_some(modes, |panel, modes| {
                panel.child(
                    h_flex().flex_wrap().gap_2().children(
                        [
                            (
                                ShortcutMode::Tap,
                                "custom-shortcut-tap",
                                tr!("actions.shortcut_tap"),
                            ),
                            (
                                ShortcutMode::Hold,
                                "custom-shortcut-hold",
                                tr!("actions.shortcut_hold"),
                            ),
                        ]
                        .map(|(mode, id, label)| {
                            let on_change = modes.on_change.clone();
                            div().debug_selector(move || id.to_string()).child(
                                control_button(id)
                                    .label(label)
                                    .selected(selected_mode == mode)
                                    .on_click(move |_, _, cx| on_change(mode, cx)),
                            )
                        }),
                    ),
                )
            })
            .child(Self::input_row(
                "custom-shortcut-add",
                &self.shortcut,
                valid_shortcut,
                on_pick,
                move |text, cx| {
                    let mode = read_mode
                        .as_ref()
                        .map_or(ShortcutMode::Tap, |read| read(cx));
                    text.parse::<KeyCombo>()
                        .ok()
                        .map(|combo| mode.action(combo))
                },
            ))
            .when(!shortcut.trim().is_empty() && !valid_shortcut, |panel| {
                panel.child(
                    div()
                        .debug_selector(|| "custom-shortcut-error".into())
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(tr!("actions.invalid_shortcut")),
                )
            })
            .child(editor_section(
                tr!("action_ring.open_application_or_folder"),
                pal,
            ))
            .child(Self::input_row(
                "custom-application-add",
                &self.application,
                valid_application,
                on_pick,
                |text, _| {
                    ApplicationTarget::new(text, "")
                        .ok()
                        .map(Action::OpenApplication)
                },
            ))
    }

    fn input_row(
        id: &'static str,
        input: &Entity<InputState>,
        valid: bool,
        on_pick: &PickFn,
        parse: impl Fn(&str, &App) -> Option<Action> + 'static,
    ) -> gpui::Div {
        let submit = input.clone();
        let on_pick = on_pick.clone();
        h_flex()
            .debug_selector(move || id.to_string())
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(control_input(input).cleanable(true)),
            )
            .child(
                control_button(id)
                    .label(tr!("common.add"))
                    .disabled(!valid)
                    .on_click(move |_, window, cx| {
                        let text = submit.read(cx).value();
                        if let Some(action) = parse(text.trim(), cx) {
                            on_pick(action, window, cx);
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, Render,
        TestAppContext, VisualTestContext, point, px,
    };
    use std::cell::RefCell;

    struct EditorHarness {
        inputs: CustomActionInputs,
        mode: ShortcutMode,
        picked: Rc<RefCell<Vec<Action>>>,
    }

    impl Render for EditorHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.inputs.localize(window, cx);
            let picked = self.picked.clone();
            let inputs = self.inputs.clone();
            let on_pick: PickFn = Rc::new(move |action, window, cx| {
                inputs.clear(window, cx);
                picked.borrow_mut().push(action);
            });
            let view = cx.entity();
            let mode_owner = view.clone();
            let modes = ShortcutModes {
                selected: self.mode,
                read: Rc::new(move |cx| mode_owner.read(cx).mode),
                on_change: Rc::new(move |mode, cx| {
                    view.update(cx, |view, cx| {
                        view.mode = mode;
                        cx.notify();
                    });
                }),
            };
            self.inputs
                .render(Some(modes), &on_pick, crate::ui::theme::palette(cx), cx)
                .w(px(320.))
        }
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click_submit(cx: &mut VisualTestContext, row: &'static str) {
        let bounds = cx.debug_bounds(row).expect("custom action row is visible");
        cx.simulate_click(
            point(bounds.right() - px(16.), bounds.center().y),
            Modifiers::default(),
        );
        draw(cx);
    }

    #[gpui::test]
    fn custom_editor_rejects_invalid_input_then_commits_hold_and_clears_drafts(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let picked = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = cx.add_window_view({
            let picked = picked.clone();
            move |window, cx| EditorHarness {
                inputs: CustomActionInputs::new(window, cx),
                mode: ShortcutMode::Tap,
                picked,
            }
        });
        draw(cx);
        let input = view.read_with(cx, |view, _| view.inputs.shortcut.clone());
        cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
        cx.simulate_input("not-a-key");
        draw(cx);
        assert!(cx.debug_bounds("custom-shortcut-error").is_some());
        click_submit(cx, "custom-shortcut-add");
        assert!(picked.borrow().is_empty());

        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        });
        cx.simulate_input("Ctrl+P");
        draw(cx);
        let hold = cx.debug_bounds("custom-shortcut-hold").unwrap();
        let add = cx.debug_bounds("custom-shortcut-add").unwrap();
        // A second mouse click can arrive before the dirty editor is drawn.
        cx.update(|window, cx| {
            for position in [hold.center(), point(add.right() - px(16.), add.center().y)] {
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
        });
        draw(cx);
        assert_eq!(
            *picked.borrow(),
            vec![Action::HoldShortcut("Ctrl+P".parse().unwrap())]
        );
        assert_eq!(
            input.read_with(cx, |input, _| input.value().to_string()),
            ""
        );
    }
    #[gpui::test]
    fn application_commit_clears_both_shared_drafts(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let picked = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = cx.add_window_view({
            let picked = picked.clone();
            move |window, cx| EditorHarness {
                inputs: CustomActionInputs::new(window, cx),
                mode: ShortcutMode::Tap,
                picked,
            }
        });
        let (application, shortcut) = view.read_with(cx, |view, _| {
            (
                view.inputs.application.clone(),
                view.inputs.shortcut.clone(),
            )
        });
        cx.update(|window, cx| {
            application.update(cx, |input, cx| {
                input.set_value("/Applications/Test.app", window, cx);
            });
            shortcut.update(cx, |input, cx| input.set_value("bad-key", window, cx));
        });
        draw(cx);
        click_submit(cx, "custom-application-add");
        assert_eq!(
            *picked.borrow(),
            vec![Action::OpenApplication(
                ApplicationTarget::new("/Applications/Test.app", "").unwrap()
            )]
        );
        draw(cx);
        assert!(cx.debug_bounds("custom-application-error").is_none());
        assert!(cx.debug_bounds("custom-shortcut-error").is_none());
        assert_eq!(
            application.read_with(cx, |input, _| input.value().to_string()),
            ""
        );
        assert_eq!(
            shortcut.read_with(cx, |input, _| input.value().to_string()),
            ""
        );
    }
}
