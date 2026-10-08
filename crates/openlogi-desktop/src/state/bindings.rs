//! Mouse, gesture, and keyboard binding commits.

use std::collections::BTreeMap;

use openlogi_core::binding::{
    Action, Binding, ButtonId, GestureDirection, LongPressBinding, default_binding,
};
use openlogi_core::bindings::{bindings_for, hidpp_gesture_maps_for, oshook_gestures_for};
use openlogi_core::config::{Config, KeyTrigger};
use tracing::debug;

use crate::features::mouse::thumbwheel::{ThumbwheelPair, ThumbwheelPreset};
use crate::state::devices::DeviceRecord;

use super::events::StateEvents;
use super::{AppState, DeviceKey, StateEvent};

/// The two independently scoped binding editors.
#[derive(Clone, Copy)]
pub(crate) enum BindingEditorKind {
    Buttons,
    ActionRing,
}

impl BindingEditorKind {
    fn editing_app(self, state: &AppState) -> Option<&str> {
        match self {
            Self::Buttons => state.editing_app(),
            Self::ActionRing => state.editing_action_ring_app(),
        }
    }
}

/// The device and application that owned a rendered editor command.
#[derive(Clone)]
pub(crate) struct BindingEditorScope {
    device: DeviceKey,
    app: Option<String>,
    kind: BindingEditorKind,
}

impl BindingEditorScope {
    /// Reject a command retained from a previously selected device or profile.
    pub(crate) fn is_current(&self, state: &AppState) -> bool {
        state.is_current_device(&self.device) && self.app.as_deref() == self.kind.editing_app(state)
    }
}

/// The per-app profile the binding panels are editing, and the device it was
/// chosen for, by the persistent config key its profiles are stored under.
/// Pairing them prevents a scope opened for one mouse from carrying over when
/// selection moves to another.
struct EditingScope {
    persistent_key: String,
    app: String,
}

/// Binding-editor state projected from the persisted configuration.
pub(super) struct BindingState {
    editing_scope: Option<EditingScope>,
    /// The hotspot the user most recently armed by clicking.
    active_button: Option<ButtonId>,
    /// Effective bindings for the selected device and open profile.
    button_bindings: BTreeMap<ButtonId, Action>,
    /// Device-global per-direction gesture bindings.
    gesture_bindings: BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>>,
    /// Global keyboard F-key bindings (Esc + F1-F19).
    keyboard_bindings: BTreeMap<KeyTrigger, Action>,
}

impl BindingState {
    pub(super) fn new(config: &Config, persistent_key: Option<&str>) -> Self {
        let mut state = Self {
            editing_scope: None,
            active_button: None,
            button_bindings: BTreeMap::new(),
            gesture_bindings: BTreeMap::new(),
            keyboard_bindings: config.keyboard.bindings.clone(),
        };
        state.refresh_device(config, persistent_key);
        state
    }

    fn editing_app<'a>(&'a self, persistent_key: Option<&str>) -> Option<&'a str> {
        let key = persistent_key?;
        self.editing_scope
            .as_ref()
            .filter(|scope| scope.persistent_key == key)
            .map(|scope| scope.app.as_str())
    }

    fn set_editing_app(
        &mut self,
        config: &Config,
        persistent_key: Option<&str>,
        app: Option<String>,
    ) {
        self.editing_scope =
            app.zip(persistent_key.map(str::to_string))
                .map(|(app, persistent_key)| EditingScope {
                    persistent_key,
                    app,
                });
        self.refresh_device(config, persistent_key);
    }

    fn refresh_device(&mut self, config: &Config, persistent_key: Option<&str>) {
        let button_bindings =
            bindings_for(config, persistent_key, self.editing_app(persistent_key));
        let gesture_bindings = gesture_maps_for(config, persistent_key);
        self.button_bindings = button_bindings;
        self.gesture_bindings = gesture_bindings;
    }

    fn restore(&mut self, config: &Config, persistent_key: Option<&str>) {
        self.refresh_device(config, persistent_key);
        self.keyboard_bindings = config.keyboard.bindings.clone();
    }
}

fn gesture_maps_for(
    config: &Config,
    persistent_key: Option<&str>,
) -> BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>> {
    let Some(key) = persistent_key else {
        return BTreeMap::new();
    };
    let mut maps = hidpp_gesture_maps_for(config, Some(key), None);
    maps.extend(oshook_gestures_for(config, Some(key), None));
    maps
}

/// Write both halves of a thumb-wheel preset into `app`'s profile, or the
/// device's global bindings when `app` is `None`.
pub(super) fn apply_thumbwheel_pair(
    button_bindings: &mut BTreeMap<ButtonId, Action>,
    config: &mut openlogi_core::config::Config,
    persistent_key: Option<&str>,
    app: Option<&str>,
    pair: ThumbwheelPair,
) -> bool {
    button_bindings.insert(ButtonId::ThumbwheelScrollDown, pair.backward.clone());
    button_bindings.insert(ButtonId::ThumbwheelScrollUp, pair.forward.clone());

    let Some(key) = persistent_key else {
        return false;
    };
    for (button, action) in [
        (ButtonId::ThumbwheelScrollDown, pair.backward),
        (ButtonId::ThumbwheelScrollUp, pair.forward),
    ] {
        match app {
            Some(app) => config.set_per_app_binding(key, app, button, Some(action)),
            None => config.set_binding(key, button, Binding::Single(action)),
        }
    }
    true
}

impl AppState {
    /// Capture the actual editor scope before installing a frame's callbacks.
    pub(crate) fn binding_editor_scope(
        &self,
        kind: BindingEditorKind,
    ) -> Option<BindingEditorScope> {
        Some(BindingEditorScope {
            device: self.current_record()?.device_key(),
            app: kind.editing_app(self).map(str::to_owned),
            kind,
        })
    }

    /// The application whose profile the binding panels are editing, or `None`
    /// for the device's global profile.
    #[must_use]
    pub fn editing_app(&self) -> Option<&str> {
        self.bindings.editing_app(
            self.current_record()
                .and_then(DeviceRecord::persistent_config_key),
        )
    }

    /// Edit `app`'s profile for the active device, or its global profile with
    /// `None`. Re-derives the editor projections without persisting this
    /// window-local choice.
    pub fn set_editing_app(&mut self, app: Option<String>) -> StateEvents {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string);
        self.bindings
            .set_editing_app(&self.config, key.as_deref(), app);
        self.for_current_device(StateEvent::BindingsChanged)
    }

    /// The hotspot most recently armed in the mouse editor.
    #[must_use]
    pub fn active_button(&self) -> Option<ButtonId> {
        self.bindings.active_button
    }

    /// Effective mouse bindings for the selected device and open profile.
    #[must_use]
    pub fn button_bindings(&self) -> &BTreeMap<ButtonId, Action> {
        &self.bindings.button_bindings
    }

    /// Device-global gesture direction maps for the selected device.
    #[must_use]
    pub fn gesture_bindings(&self) -> &BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>> {
        &self.bindings.gesture_bindings
    }

    /// Global keyboard F-key bindings.
    #[must_use]
    pub fn keyboard_bindings(&self) -> &BTreeMap<KeyTrigger, Action> {
        &self.bindings.keyboard_bindings
    }

    /// Long-press pairs require a stable device identity and the global profile.
    #[must_use]
    pub fn is_long_press_editable(&self) -> bool {
        self.editing_app().is_none()
            && self
                .current_record()
                .and_then(DeviceRecord::persistent_config_key)
                .is_some()
    }

    /// The selected button's global short/long pair, when this scope supports it.
    pub fn long_press_binding(&self, button: ButtonId) -> Option<&LongPressBinding> {
        if self.editing_app().is_some() {
            return None;
        }
        let key = self.current_record()?.persistent_config_key()?;
        match self.config.devices.get(key)?.bindings.get(&button)? {
            Binding::LongPress(pair) => Some(pair),
            Binding::Single(_) | Binding::Gesture(_) => None,
        }
    }

    pub(super) fn refresh_binding_projections(&mut self) {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string);
        self.bindings.refresh_device(&self.config, key.as_deref());
    }

    pub(super) fn restore_binding_projections(&mut self) {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string);
        self.bindings.restore(&self.config, key.as_deref());
    }

    /// Update a single binding in memory, on disk, and in the shared hook
    /// map for the currently selected device — in whichever profile
    /// [`AppState::editing_app`] has open.
    ///
    /// Disk failures restore the persisted projection and surface a config
    /// error instead of crashing the UI thread.
    pub fn commit_binding(&mut self, button: ButtonId, action: Action) -> StateEvents {
        if action.held_combo().is_some() && self.long_press_binding(button).is_some() {
            return StateEvents::none();
        }
        let events = self.for_current_device(StateEvent::BindingsChanged);
        self.bindings.button_bindings.insert(button, action.clone());

        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            debug!(
                ?button,
                "no persistent device key — binding kept in memory only"
            );
            return events;
        };
        let app = self.editing_app().map(str::to_string);
        self.config.edit(|config| {
            if let Some(app) = app {
                // A per-app entry is `Action`-valued, so an override always
                // replaces the whole button — which is exactly what picking one
                // action means, and why gesture mode is not offered in this scope.
                config.set_per_app_binding(&key, &app, button, Some(action));
            } else {
                let binding = match config
                    .devices
                    .get(&key)
                    .and_then(|device| device.bindings.get(&button))
                {
                    Some(Binding::LongPress(pair)) => {
                        Binding::LongPress(LongPressBinding::new(action, pair.long().clone()))
                    }
                    _ => Binding::Single(action),
                };
                config.set_binding(&key, button, binding);
            }
        });
        // The agent owns the hook; have it rebuild its live map from config.
        self.persist_and_reload("binding");
        events
    }

    /// Whether delaying this button until release preserves its current action.
    #[must_use]
    pub fn is_button_press_delayable(&self, button: ButtonId) -> bool {
        self.bindings
            .button_bindings
            .get(&button)
            .is_none_or(|action| action.held_combo().is_none())
    }

    /// Set the long action without changing the short action. App overlays
    /// remain single-action maps; they cannot create hidden global edits.
    pub fn commit_long_binding(&mut self, button: ButtonId, action: Action) -> StateEvents {
        if self.bindings.gesture_bindings.contains_key(&button)
            || !self.is_button_press_delayable(button)
        {
            return StateEvents::none();
        }
        let short = self
            .bindings
            .button_bindings
            .get(&button)
            .cloned()
            .unwrap_or_else(|| default_binding(button));
        self.commit_global_binding(
            button,
            Binding::LongPress(LongPressBinding::new(short, action)),
        )
    }

    /// Explicitly return to immediate single-action behavior, keeping short.
    pub fn clear_long_binding(&mut self, button: ButtonId) -> StateEvents {
        let Some(pair) = self.long_press_binding(button) else {
            return StateEvents::none();
        };
        self.commit_global_binding(button, Binding::Single(pair.short().clone()))
    }

    fn commit_global_binding(&mut self, button: ButtonId, binding: Binding) -> StateEvents {
        if self.editing_app().is_some() {
            return StateEvents::none();
        }
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_owned)
        else {
            return StateEvents::none();
        };
        self.config
            .edit(|config| config.set_binding(&key, button, binding));
        self.refresh_binding_projections();
        self.persist_and_reload("binding");
        self.for_current_device(StateEvent::BindingsChanged)
    }

    /// Drop `button`'s override in the open per-app profile, so it inherits the
    /// device's global binding again. A no-op in the global profile, which has
    /// nothing to inherit from.
    pub fn clear_app_binding(&mut self, button: ButtonId) -> StateEvents {
        self.clear_app_bindings([button])
    }

    /// Drop both halves of a thumb-wheel override together.
    pub fn clear_app_thumbwheel(&mut self) -> StateEvents {
        self.clear_app_bindings([ButtonId::ThumbwheelScrollDown, ButtonId::ThumbwheelScrollUp])
    }

    fn clear_app_bindings(&mut self, buttons: impl IntoIterator<Item = ButtonId>) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        let Some(app) = self.editing_app().map(str::to_string) else {
            return events;
        };
        self.config.edit(|config| {
            for button in buttons {
                config.set_per_app_binding(&key, &app, button, None);
            }
        });
        self.refresh_binding_projections();
        self.persist_and_reload("per-app binding");
        events
    }

    /// Restore live default inheritance without leaving the application's editor.
    pub fn reset_app_profile(&mut self, key: &DeviceKey, app: &str) -> StateEvents {
        self.clear_app_profile(key.as_str(), app);
        StateEvent::BindingsChanged(key.clone()).into()
    }

    /// Delete both per-app configuration sections in one save/reload. A failed
    /// save restores both sections and leaves both editor selections intact.
    pub fn remove_all_app_profiles(&mut self, key: &DeviceKey, app: &str) -> StateEvents {
        let changed = self.config.edit(|config| {
            let Some(device) = config.devices.get_mut(key.as_str()) else {
                return false;
            };
            let buttons = device.per_app_bindings.remove(app).is_some();
            let ring = device.action_ring.per_app.remove(app).is_some();
            buttons || ring
        });
        if changed && !self.persist_and_reload("all application profiles") {
            return StateEvent::BindingsChanged(key.clone()).into();
        }

        // Both saved sections are gone, so the scoped removals only close their
        // matching editors; neither writes or reloads an already absent profile.
        let events = self.remove_app_profile(key, app);
        let _ = self.remove_action_ring_profile(key, app);
        events
    }

    /// Remove the named profile, not whichever profile is selected when a
    /// confirmation completes. Only leave its editor after a successful save.
    pub fn remove_app_profile(&mut self, key: &DeviceKey, app: &str) -> StateEvents {
        if self.clear_app_profile(key.as_str(), app)
            && self
                .bindings
                .editing_scope
                .as_ref()
                .is_some_and(|scope| scope.persistent_key == key.as_str() && scope.app == app)
        {
            self.bindings.editing_scope = None;
            self.refresh_binding_projections();
        }
        StateEvent::BindingsChanged(key.clone()).into()
    }

    fn clear_app_profile(&mut self, key: &str, app: &str) -> bool {
        if !self
            .config
            .app_profiles(key)
            .any(|candidate| candidate == app)
        {
            // An application selected but never edited is window-local only.
            return true;
        }
        self.config
            .edit(|config| config.remove_app_profile(key, app));
        let saved = self.persist_and_reload("per-app profile");
        self.refresh_binding_projections();
        saved
    }

    /// The open per-app profile's overrides, so the panel can tell an override
    /// apart from a binding inherited from the global profile. `None` in the
    /// global profile, where there is nothing to distinguish.
    #[must_use]
    pub fn editing_app_overrides(&self) -> Option<&BTreeMap<ButtonId, Action>> {
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)?;
        self.editing_app()
            .and_then(|app| self.config.per_app_overrides(key, app))
    }

    /// Apply one paired thumb-wheel preset atomically. Both directional
    /// bindings are updated before the single config persistence/reload.
    pub fn commit_thumbwheel_preset(&mut self, preset: ThumbwheelPreset) -> StateEvents {
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let pair = preset.pair();
        let key = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string);
        let app = self.editing_app().map(str::to_string);
        let changed = self.config.edit(|config| {
            apply_thumbwheel_pair(
                &mut self.bindings.button_bindings,
                config,
                key.as_deref(),
                app.as_deref(),
                pair,
            )
        });
        if !changed {
            debug!("no persistent device key — thumb-wheel pair kept in memory only");
            return events;
        }
        self.persist_and_reload("thumb-wheel binding");
        events
    }
    /// Records (or, with `action = None`, clears) the F-key `trigger` binding
    /// in the global `[keyboard]` map. Mirrors [`Self::commit_binding`] minus
    /// the device key — keyboard bindings are device-agnostic, so the write
    /// happens with or without a selected device; only the event it reports is
    /// addressed to the selected device, as every binding change's is. The
    /// agent's `rebuild()` republishes its shared keyboard map on
    /// `reload_config`, so this lands live.
    pub fn commit_keyboard_binding(
        &mut self,
        trigger: KeyTrigger,
        action: Option<Action>,
    ) -> StateEvents {
        match action {
            Some(ref a) => {
                self.bindings
                    .keyboard_bindings
                    .insert(trigger.clone(), a.clone());
            }
            None => {
                self.bindings.keyboard_bindings.remove(&trigger);
            }
        }
        self.config
            .edit(|config| config.set_keyboard_binding(trigger, action));
        self.persist_and_reload("keyboard binding");
        self.for_current_device(StateEvent::BindingsChanged)
    }
    /// Per-direction maps for every gesture-mode button of the current device,
    /// keyed by button — what the runtime dispatches for it. HID++ sources come
    /// fully seeded (matching the gesture watcher's projection); OS-hook
    /// buttons show their raw stored map (matching the OS hook's dispatch).
    /// Empty when no device is selected.
    ///
    /// Device-level: direction maps live only in the global profile, so this
    /// does not vary with the profile this window has open.
    #[must_use]
    pub(crate) fn device_gesture_maps(
        &self,
    ) -> BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>> {
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
        else {
            return BTreeMap::new();
        };
        gesture_maps_for(&self.config, Some(key))
    }

    /// How many gesture directions the active device has bound, across every
    /// gesture-mode button. Device-level like [`Self::device_gesture_maps`].
    #[must_use]
    pub fn device_gesture_binding_count(&self) -> usize {
        self.device_gesture_maps().values().map(BTreeMap::len).sum()
    }

    /// The gesture menus the panel offers: [`Self::device_gesture_maps`], or
    /// nothing while a per-app profile is open.
    ///
    /// A per-app entry holds one `Action` and has no per-direction shape, so
    /// there is nothing to edit in that scope: every button falls through to
    /// the single-action picker, and overriding one is what stops it gesturing
    /// in that app. Offering the gesture menu instead would edit the global
    /// profile from a screen labelled with an application.
    #[must_use]
    #[cfg(test)]
    pub fn current_gesture_maps(&self) -> BTreeMap<ButtonId, BTreeMap<GestureDirection, Action>> {
        if self.editing_app().is_some() {
            return BTreeMap::new();
        }
        self.device_gesture_maps()
    }

    /// Turn gesture mode on or off for one button of the current device —
    /// independently of every other button. Persists, tells the agent to
    /// rebuild, and refreshes the projected maps the UI reads.
    pub fn commit_gesture_mode(&mut self, button: ButtonId, enabled: bool) -> StateEvents {
        if enabled && !self.is_button_press_delayable(button) {
            return StateEvents::none();
        }
        let events = self.for_current_device(StateEvent::BindingsChanged);
        if enabled && !button.supports_gesture_mode() {
            debug!(?button, "gesture mode is not supported for this control");
            return events;
        }
        if enabled
            && button == ButtonId::DpiToggle
            && !self
                .current_record()
                .and_then(|record| record.capabilities)
                .is_some_and(|caps| caps.dpi_gestures)
        {
            debug!("DPI gestures require measured raw-XY support");
            return events;
        }
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            return events;
        };
        // Gesture mode is a property of the device's global bindings — a
        // per-app entry holds one `Action` and has no per-direction shape to
        // promote into. The picker hides the entry point in a per-app profile;
        // this is the backstop, because writing it here would silently change
        // every app instead of the one on screen.
        if self.editing_app().is_some() {
            debug!(?button, "gesture mode is not editable in a per-app profile");
            return events;
        }
        if self.config.is_gesture_mode(&key, button) == enabled {
            return events;
        }
        self.config
            .edit(|config| config.set_gesture_mode(&key, button, enabled));
        // The mode change shuffles bindings between the single + gesture maps.
        self.refresh_binding_projections();
        self.persist_and_reload("gesture-mode change");
        events
    }

    /// Update one direction of `button`'s gesture binding in memory, on disk,
    /// and (via reload) in the maps the agent dispatches from.
    pub fn commit_gesture_binding(
        &mut self,
        button: ButtonId,
        direction: GestureDirection,
        action: Action,
    ) -> StateEvents {
        if direction == GestureDirection::Click && action.held_combo().is_some() {
            return StateEvents::none();
        }
        let events = self.for_current_device(StateEvent::BindingsChanged);
        let Some(key) = self
            .current_record()
            .and_then(DeviceRecord::persistent_config_key)
            .map(str::to_string)
        else {
            debug!(
                ?button,
                ?direction,
                "no persistent device key — gesture binding edit ignored"
            );
            return events;
        };
        let is_gesture_mode = self.config.is_gesture_mode(&key, button);
        let is_stored_os_hook_gesture = button.is_os_hook_button() && is_gesture_mode;
        if !button.supports_gesture_mode() && !is_stored_os_hook_gesture {
            debug!(?button, "gestures are not supported for this control");
            return events;
        }
        // Same backstop as `commit_gesture_mode`: direction maps live only in
        // the global profile, so an edit arriving while a per-app one is open
        // would change every app instead of the one on screen.
        if self.editing_app().is_some() {
            debug!(
                ?button,
                ?direction,
                "gestures are not editable in a per-app profile"
            );
            return events;
        }
        // A stray edit on a button not in gesture mode must NOT silently
        // promote it (the gesture editor shouldn't be reachable in that
        // state): no-op instead. Checking the stored mode rather than the
        // current enablement policy keeps v0.8.0 Middle Click gesture maps
        // editable until the user explicitly turns them off.
        if !is_gesture_mode {
            debug!(
                ?button,
                ?direction,
                "button is not in gesture mode — ignoring gesture binding edit"
            );
            return events;
        }
        self.bindings
            .gesture_bindings
            .entry(button)
            .or_default()
            .insert(direction, action.clone());
        self.config
            .edit(|config| config.set_gesture_direction(&key, button, direction, action));
        // The agent owns the gesture watcher; have it rebuild from config.
        self.persist_and_reload("gesture binding");
        events
    }
}
