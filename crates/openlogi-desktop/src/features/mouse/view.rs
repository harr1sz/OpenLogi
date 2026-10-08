use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, RenderOnce,
    StatefulInteractiveElement as _, Styled, Subscription, Window, canvas, div, hsla, img,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_base::Button as BaseButton;
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use openlogi_core::binding::{Action, ButtonId, GestureDirection};

use super::geometry::{
    LabelDistribution, asset_dimensions_for_png, asset_has_button_labels, asset_hotspots_for_png,
    labels_from_hotspots,
};
use super::hotspots::{Hotspot, MOUSE_MODEL_SIZE, MouseControlId, default_hotspots};
use super::inspector::{BindingInspectorData, binding_inspector};
use super::leader_lines::{Geometry as LeaderGeometry, Label, paint as paint_leader_lines};
use crate::app::{glow_canvas, keyboard_glow};
use crate::features::binding_editor::custom::{CustomActionInputs, ShortcutMode};
use crate::features::profiles::{friendly_app_name, profile_canvas_status};
use crate::services::assets::{GlowGeometry, ResolvedAsset};
use crate::state::{
    AppState, BindingEditorKind, BindingEditorScope, DeviceKey, DeviceRecord, StateEvent,
};
use crate::ui::theme::{self, ACCENT_BLUE};

const SIDE_GAP: f32 = 24.;
const LABEL_W: f32 = 156.;
const LABEL_GUTTER: f32 = LABEL_W + SIDE_GAP;
const TWO_SIDED_LABEL_MIN_W: f32 = 700.;

const CARD_EDGE_INSET: f32 = SIDE_GAP;

const HOTSPOT_DOT: f32 = 12.;
/// Vertical space occupied by the device bar, profile context, and canvas
/// padding. Normal operation no longer reserves a footer.
const MODEL_VERTICAL_RESERVE: f32 = 154.;

mod labels;
use labels::{binding_label_for_control, label_control};
/// Floor for the scaled model height. Label cards can make the scrollable
/// canvas taller when a narrow window constrains the image width.
const MODEL_MIN_H: f32 = 360.;

/// Max width the model (side gutter + image) may occupy, matching the
/// `buttons_tab` content cap so a wide keyboard image never overflows the panel.
const MODEL_CONTENT_MAX_W: f32 = 760.;
/// Horizontal chrome the model can't draw into (the buttons-tab padding).
const MODEL_HORIZONTAL_RESERVE: f32 =
    crate::ui::theme::DETAIL_RAIL_W + super::inspector::INSPECTOR_W + 48.;
/// Floor for the model's available width on a narrow window.
const MODEL_MIN_CONTENT_W: f32 = 200.;

struct MouseWorkspaceData<'a> {
    device_key: Option<DeviceKey>,
    asset: Option<&'a ResolvedAsset>,
    active: Option<MouseControlId>,
    bindings: &'a BTreeMap<ButtonId, Action>,
    gesture_maps: &'a BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>>,
    glow: Option<(Arc<GlowGeometry>, Hsla)>,
    thumbwheel: bool,
    dpi_gestures: bool,
    editing_app: Option<String>,
    overridden: Option<&'a BTreeMap<ButtonId, Action>>,
}

impl<'a> MouseWorkspaceData<'a> {
    fn read(cx: &'a App) -> Option<Self> {
        AppState::try_read(cx).map(|state| Self {
            device_key: state.current_record().map(DeviceRecord::device_key),
            asset: state
                .current_record()
                .and_then(|record| record.asset.as_ref()),
            active: state
                .active_button()
                .map(MouseControlId::from_active_button),
            bindings: state.button_bindings(),
            gesture_maps: state.gesture_bindings(),
            glow: state
                .current_record()
                .and_then(|record| keyboard_glow(state, record)),
            thumbwheel: state
                .current_record()
                .and_then(|record| record.capabilities)
                .is_some_and(|capabilities| capabilities.thumbwheel),
            dpi_gestures: state
                .current_record()
                .and_then(|record| record.capabilities)
                .is_some_and(|capabilities| capabilities.dpi_gestures),
            editing_app: state.editing_app().map(|app| {
                state
                    .recent_app_name(app)
                    .map_or_else(|| friendly_app_name(app), str::to_string)
            }),
            overridden: state.editing_app_overrides(),
        })
    }

    fn empty(
        bindings: &'a BTreeMap<ButtonId, Action>,
        gesture_maps: &'a BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>>,
    ) -> Self {
        Self {
            device_key: None,
            asset: None,
            active: None,
            bindings,
            gesture_maps,
            glow: None,
            thumbwheel: false,
            dpi_gestures: false,
            editing_app: None,
            overridden: None,
        }
    }
}

/// The two mutually exclusive outcomes of a physical button press.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ButtonPress {
    #[default]
    Short,
    Long,
}

/// The scope and control that owned an inspector's rendered callbacks.
#[derive(Clone)]
pub(super) struct MouseEditorTarget {
    scope: Option<BindingEditorScope>,
    control: Option<MouseControlId>,
    direction: Option<GestureDirection>,
    press: ButtonPress,
    picker_open: bool,
}

impl MouseEditorTarget {
    /// A pending mouse callback may outlive the frame that selected its owner.
    pub(super) fn is_current(&self, view: &Entity<MouseModelView>, cx: &App) -> bool {
        let view = view.read(cx);
        self.control == view.selected
            && self.direction == view.gesture_active_dir
            && self.press == view.button_press
            && self.picker_open == view.action_picker_open
            && self.scope.as_ref().is_some_and(|scope| {
                AppState::try_read(cx).is_some_and(|state| scope.is_current(state))
            })
    }
}

/// Interactive mouse model with button hotspots.
pub struct MouseModelView {
    focus_handle: FocusHandle,
    current_device_key: Option<DeviceKey>,
    hovered: Option<MouseControlId>,
    selected: Option<MouseControlId>,
    /// The gesture direction whose action is open in the fixed inspector.
    gesture_active_dir: Option<GestureDirection>,
    action_picker_open: bool,
    action_search: Entity<InputState>,
    pub(super) custom_inputs: CustomActionInputs,
    pub(super) shortcut_mode: ShortcutMode,
    pub(super) button_press: ButtonPress,
    editing_scope: Option<String>,
    _state_obs: Subscription,
}

impl MouseModelView {
    fn editor_target(&self, cx: &App) -> MouseEditorTarget {
        MouseEditorTarget {
            scope: AppState::try_read(cx)
                .and_then(|state| state.binding_editor_scope(BindingEditorKind::Buttons)),
            control: self.selected,
            direction: self.gesture_active_dir,
            press: self.button_press,
            picker_open: self.action_picker_open,
        }
    }

    /// Create the mouse model view.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let action_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr!("actions.search_actions")));
        cx.subscribe(&action_search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let state_obs = AppState::repaint_on(cx, |event| {
            matches!(
                event,
                StateEvent::ForegroundChanged
                    | StateEvent::BindingsChanged(_)
                    | StateEvent::LightingChanged(_)
            )
        });
        Self {
            focus_handle: cx.focus_handle(),
            current_device_key: None,
            hovered: None,
            selected: None,
            gesture_active_dir: None,
            action_picker_open: false,
            action_search,
            custom_inputs: CustomActionInputs::new(window, cx),
            shortcut_mode: ShortcutMode::Tap,
            button_press: ButtonPress::Short,
            editing_scope: None,
            _state_obs: state_obs,
        }
    }

    fn close_picker_on_escape(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.action_picker_open {
            self.close_action_picker();
            self.custom_inputs.clear(window, cx);
            self.focus_handle.focus(window, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn sync_editor_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::components::localize_placeholder(
            &self.action_search,
            tr!("actions.search_actions"),
            window,
            cx,
        );
        self.custom_inputs.localize(window, cx);
        let scope = AppState::try_read(cx).and_then(|state| state.editing_app().map(str::to_owned));
        if self.editing_scope != scope {
            self.editing_scope = scope;
            self.button_press = ButtonPress::Short;
            self.close_action_picker();
            self.custom_inputs.clear(window, cx);
        }
    }

    /// Set (or clear, with `None`) the activated gesture direction. Callers must
    /// `cx.notify()` to re-render.
    pub(crate) fn set_gesture_selected_dir(&mut self, dir: Option<GestureDirection>) {
        self.gesture_active_dir = dir;
        self.action_picker_open = false;
    }

    pub(super) fn toggle_action_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.action_picker_open {
            self.custom_inputs.clear(window, cx);
            self.shortcut_mode = ShortcutMode::Tap;
        }
        self.action_picker_open = !self.action_picker_open;
    }

    pub(super) fn close_action_picker(&mut self) {
        self.action_picker_open = false;
    }

    fn reset_for_device(&mut self, device_key: Option<DeviceKey>) {
        if self.current_device_key == device_key {
            return;
        }
        self.current_device_key = device_key;
        self.hovered = None;
        self.selected = None;
        self.gesture_active_dir = None;
        self.button_press = ButtonPress::Short;
        self.action_picker_open = false;
    }

    fn select(&mut self, control: MouseControlId) {
        if self.selected != Some(control) {
            self.selected = Some(control);
            self.gesture_active_dir = None;
            self.button_press = ButtonPress::Short;
            self.action_picker_open = false;
        }
    }
}

impl Focusable for MouseModelView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn set_control_hovered(
    view: &Entity<MouseModelView>,
    control: MouseControlId,
    hovered: bool,
    cx: &mut App,
) {
    view.update(cx, |this, cx| {
        if hovered {
            this.hovered = Some(control);
        } else if this.hovered == Some(control) {
            this.hovered = None;
        }
        cx.notify();
    });
}

impl Render for MouseModelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_editor_scope(window, cx);

        let (empty_bindings, empty_gesture_maps) = (BTreeMap::new(), BTreeMap::new());
        let MouseWorkspaceData {
            device_key,
            asset,
            active,
            bindings,
            gesture_maps,
            glow,
            thumbwheel,
            dpi_gestures,
            editing_app,
            overridden,
        } = MouseWorkspaceData::read(cx)
            .unwrap_or_else(|| MouseWorkspaceData::empty(&empty_bindings, &empty_gesture_maps));

        self.reset_for_device(device_key);

        let gesture_buttons: Vec<ButtonId> = gesture_maps
            .keys()
            .copied()
            .filter(|button| {
                editing_app.is_none()
                    || !overridden.is_some_and(|overrides| overrides.contains_key(button))
            })
            .collect();

        let viewport_h = f32::from(window.viewport_size().height);
        let viewport_w = f32::from(window.viewport_size().width);
        let ModelLayout {
            canvas_w,
            canvas_h,
            mouse_left,
            mouse_w,
            mouse_h,
            hotspots,
            labels,
        } = model_layout(asset, viewport_w, viewport_h, thumbwheel);
        let highlight = self.hovered.or(active).or(self.selected);
        let view = cx.entity();
        let hovered = self.hovered;
        let profile_status = profile_canvas_status(cx);

        let hotspots_outer = hotspots.clone();
        let labels_outer = labels.clone();
        let leader_canvas = leader_canvas(hotspots, labels, highlight, mouse_left, mouse_w);
        let breathing_art = breathing_art(asset, mouse_left, mouse_w, mouse_h, glow);
        let model = ModelRect {
            left: mouse_left,
            width: mouse_w,
            height: mouse_h,
        };
        let hotspots_layer = hotspots_layer(
            &hotspots_outer,
            model,
            hovered,
            active,
            self.selected,
            &view,
        );
        let canvas = div()
            .relative()
            .w(px(canvas_w))
            .h(px(canvas_h))
            .child(breathing_art)
            .child(leader_canvas)
            .children(labels_outer.iter().enumerate().map(|(idx, label)| {
                let binding = binding_label_for_control(label.id, bindings, &gesture_buttons);
                label_control(
                    idx,
                    *label,
                    binding,
                    hovered == Some(label.id) || active == Some(label.id),
                    model,
                    self.selected == Some(label.id),
                    &view,
                )
            }))
            .child(hotspots_layer);

        let editor_target = self.editor_target(cx);
        let inspector = binding_inspector(
            BindingInspectorData {
                target: &editor_target,
                selected: self.selected,
                gesture_direction: self.gesture_active_dir,
                action_picker_open: self.action_picker_open,
                button_press: self.button_press,
                shortcut_mode: self.shortcut_mode,
                custom_inputs: &self.custom_inputs,
                bindings,
                gesture_maps,
                dpi_gestures,
                editing_app: editing_app.as_deref(),
                overridden,
            },
            &self.action_search,
            &view,
            cx,
        );
        workspace_layout(canvas, profile_status, inspector, &self.focus_handle)
            .on_key_down(cx.listener(Self::close_picker_on_escape))
    }
}

fn workspace_layout(
    canvas: impl IntoElement,
    profile_status: Option<gpui::Div>,
    inspector: impl IntoElement,
    focus_handle: &FocusHandle,
) -> gpui::Div {
    h_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .w_full()
        .items_stretch()
        .tab_group()
        .track_focus(focus_handle)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .children(profile_status)
                .child(
                    div()
                        .id("mouse-model-scroll")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .min_h_full()
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .p_4()
                                .child(canvas),
                        ),
                ),
        )
        .child(inspector)
}

struct ModelLayout {
    canvas_w: f32,
    canvas_h: f32,
    mouse_left: f32,
    mouse_w: f32,
    mouse_h: f32,
    hotspots: Vec<Hotspot>,
    labels: Vec<Label>,
}

/// Scale the model to fit the content area in both axes. A tall mouse is bound
/// by the viewport height; a wide keyboard is bound by the available width and
/// drops the label gutter so it remains centred.
fn model_layout(
    asset: Option<&ResolvedAsset>,
    viewport_w: f32,
    viewport_h: f32,
    thumbwheel: bool,
) -> ModelLayout {
    let target_h = (viewport_h - MODEL_VERTICAL_RESERVE).clamp(MODEL_MIN_H, MOUSE_MODEL_SIZE.1);
    let has_labels = asset.is_none_or(asset_has_button_labels) && viewport_w >= 960.;
    let content_w =
        (viewport_w - MODEL_HORIZONTAL_RESERVE).clamp(MODEL_MIN_CONTENT_W, MODEL_CONTENT_MAX_W);
    let label_distribution = if has_labels && content_w >= TWO_SIDED_LABEL_MIN_W {
        LabelDistribution::BothSides
    } else {
        LabelDistribution::LeftOnly
    };
    let left_gutter = if has_labels { LABEL_GUTTER } else { 0. };
    let right_gutter = if label_distribution == LabelDistribution::BothSides {
        LABEL_GUTTER
    } else {
        0.
    };
    let max_image_w = (content_w - left_gutter - right_gutter).max(MODEL_MIN_CONTENT_W / 2.);
    let (mouse_w, mouse_h, hotspots, mut labels) =
        scaled_model(asset, target_h, max_image_w, thumbwheel, label_distribution);
    if !has_labels {
        labels.clear();
    }

    ModelLayout {
        canvas_w: left_gutter + mouse_w + right_gutter,
        canvas_h: labels.iter().fold(mouse_h, |height, label| {
            height.max(label.y + super::geometry::LABEL_H / 2.)
        }),
        mouse_left: left_gutter,
        mouse_w,
        mouse_h,
        hotspots,
        labels,
    }
}

/// Model geometry fit inside a `max_w` × `target_h` box. With a real asset the
/// hotspots and labels are recomputed from the scaled dimensions; the synthetic
/// silhouette's authored coordinates are scaled by the same factor. Returns
/// `(mouse_w, mouse_h, hotspots, labels)`.
fn scaled_model(
    asset: Option<&ResolvedAsset>,
    target_h: f32,
    max_w: f32,
    thumbwheel: bool,
    label_distribution: LabelDistribution,
) -> (f32, f32, Vec<Hotspot>, Vec<Label>) {
    if let Some(a) = asset {
        let (w, h) = asset_dimensions_for_png(a, target_h, max_w);
        let hotspots = asset_hotspots_for_png(a, w, h);
        let labels = labels_from_hotspots(&hotspots, h, label_distribution);
        (w, h, hotspots, labels)
    } else {
        let scale = (target_h / MOUSE_MODEL_SIZE.1).min(max_w / MOUSE_MODEL_SIZE.0);
        let hotspots: Vec<_> = default_hotspots(thumbwheel)
            .into_iter()
            .map(|hs| Hotspot {
                x: hs.x * scale,
                y: hs.y * scale,
                w: hs.w * scale,
                h: hs.h * scale,
                ..hs
            })
            .collect();
        let labels =
            labels_from_hotspots(&hotspots, MOUSE_MODEL_SIZE.1 * scale, label_distribution);
        (
            MOUSE_MODEL_SIZE.0 * scale,
            MOUSE_MODEL_SIZE.1 * scale,
            hotspots,
            labels,
        )
    }
}

fn leader_canvas(
    hotspots: Vec<Hotspot>,
    labels: Vec<Label>,
    highlight: Option<MouseControlId>,
    mouse_left: f32,
    mouse_w: f32,
) -> impl IntoElement {
    canvas(
        move |_bounds, _, _| (hotspots, labels, highlight),
        move |bounds, payload, window, _app| {
            let (hotspots, labels, highlight) = payload;
            paint_leader_lines(
                bounds,
                LeaderGeometry {
                    mouse_origin: gpui::point(px(mouse_left), px(0.)),
                    mouse_w,
                    card_edge_inset: CARD_EDGE_INSET,
                },
                &hotspots,
                &labels,
                highlight,
                window,
            );
        },
    )
    .size_full()
}

fn breathing_art(
    asset: Option<&ResolvedAsset>,
    mouse_left: f32,
    mouse_w: f32,
    mouse_h: f32,
    glow: Option<(Arc<GlowGeometry>, Hsla)>,
) -> impl IntoElement {
    let device_art: AnyElement = match asset {
        Some(a) => img(a.image_path.clone())
            .w(px(mouse_w))
            .h(px(mouse_h))
            .into_any_element(),
        None => Silhouette {
            w: mouse_w,
            h: mouse_h,
        }
        .into_any_element(),
    };
    div()
        .absolute()
        .left(px(mouse_left))
        .top(px(0.))
        .w(px(mouse_w))
        .h(px(mouse_h))
        // Paint the keyboard's RGB *behind* the render so the opaque keys occlude
        // it and the colour only reads through the inter-key gaps — light from
        // behind, not specks on top. Same effect as the home gallery, scaled to
        // this render with no pre-baked PNG (#272).
        .when_some(glow, |this, (geom, color)| {
            this.child(glow_canvas(geom, color))
        })
        .child(device_art)
}

#[derive(Clone, Copy)]
struct ModelRect {
    left: f32,
    width: f32,
    height: f32,
}

fn hotspots_layer(
    hotspots: &[Hotspot],
    model: ModelRect,
    hovered: Option<MouseControlId>,
    active: Option<MouseControlId>,
    selected: Option<MouseControlId>,
    view: &Entity<MouseModelView>,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(model.left))
        .top(px(0.))
        .w(px(model.width))
        .h(px(model.height))
        .children(hotspots.iter().enumerate().map(|(idx, hotspot)| {
            hotspot_control(
                idx,
                *hotspot,
                hovered,
                active,
                selected == Some(hotspot.id),
                view,
            )
        }))
}

/// Shape-based silhouette used when no asset is cached for the device.
///
/// Its `rounded_*` values are illustration proportions — the body shell and the
/// two drawn side buttons — not UI chrome, so they stay fixed rather than
/// tracking the `Palette` radius tokens the way real cards and controls do.
#[derive(IntoElement)]
struct Silhouette {
    w: f32,
    h: f32,
}

impl RenderOnce for Silhouette {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { w, h } = self;
        let pal = theme::palette(cx);
        div()
            .absolute()
            .inset_0()
            .w(px(w))
            .h(px(h))
            .rounded_3xl()
            .border_1()
            .border_color(pal.text_muted)
            .bg(pal.muted)
            .child(
                div()
                    .absolute()
                    .left(px(w / 2. - 14.))
                    .top(px(90.))
                    .w(px(28.))
                    .h(px(110.))
                    .rounded_md()
                    .bg(hsla(0., 0., 0.25, 1.0)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(w / 2.))
                    .top(px(20.))
                    .w(px(1.))
                    .h(px(240.))
                    .bg(pal.border),
            )
            .child(
                div()
                    .absolute()
                    .left(px(8.))
                    .top(px(210.))
                    .w(px(34.))
                    .h(px(150.))
                    .rounded_md()
                    .bg(hsla(0., 0., 0.25, 1.0)),
            )
    }
}

fn hotspot_control(
    idx: usize,
    hotspot: Hotspot,
    hovered: Option<MouseControlId>,
    active: Option<MouseControlId>,
    selected: bool,
    view: &Entity<MouseModelView>,
) -> gpui::Div {
    let view = view.clone();
    let trigger = HotspotTrigger {
        id: ("hotspot-trigger", idx).into(),
        hotspot,
        hovered: hovered == Some(hotspot.id) || active == Some(hotspot.id),
        view,
        selected,
    };
    div()
        .absolute()
        .left(px(hotspot.x))
        .top(px(hotspot.y))
        .w(px(hotspot.w))
        .h(px(hotspot.h))
        .child(trigger)
}

#[derive(IntoElement)]
struct HotspotTrigger {
    id: ElementId,
    hotspot: Hotspot,
    hovered: bool,
    view: Entity<MouseModelView>,
    selected: bool,
}

impl RenderOnce for HotspotTrigger {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let highlighted = self.hovered || self.selected;
        let selected = self.selected;
        let view = self.view;
        let click_view = view.clone();
        let hotspot = self.hotspot;
        let btn = hotspot.id;

        BaseButton::new(self.id)
            .selected(selected)
            .accessibility_label(tr!(
                "actions.bind_control",
                name => tr!(btn.translation_key())
            ))
            .aria_selected(selected)
            .flex()
            .items_center()
            .justify_center()
            .w(px(hotspot.w))
            .h(px(hotspot.h))
            .child(
                div()
                    .w(px(HOTSPOT_DOT))
                    .h(px(HOTSPOT_DOT))
                    .rounded_full()
                    .border_1()
                    .border_color(if highlighted {
                        gpui::Hsla::from(rgb(ACCENT_BLUE))
                    } else {
                        hsla(0., 0., 0.95, 0.85)
                    })
                    .bg(if highlighted {
                        gpui::Hsla::from(rgb(ACCENT_BLUE))
                    } else {
                        hsla(0., 0., 0.18, 0.85)
                    }),
            )
            .focus_visible(|style| {
                style
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(ACCENT_BLUE))
            })
            .on_click(move |_event, _window, cx| {
                click_view.update(cx, |this, cx| {
                    this.select(btn);
                    cx.notify();
                });
            })
            .on_hover(move |hovered, _window, cx| {
                set_control_hovered(&view, btn, *hovered, cx);
            })
    }
}

#[cfg(test)]
mod tests;
