use anyhow::{Context, Result, ensure};
use glam::Vec2;
use openxr as xr;

use crate::i18n::{Text, tr};

const HANDS: [&str; 2] = ["/user/hand/right", "/user/hand/left"];
const STICK_DEAD_ZONE: f32 = 0.26;
const GRIP_PRESS: f32 = 0.30;
const GRIP_RELEASE: f32 = 0.20;

#[derive(Clone, Copy)]
enum GripBinding {
    Pressure,
    Click,
    Both,
}

#[derive(Clone, Copy)]
struct NavigationBindings {
    axis: &'static str,
    reset: &'static str,
    grip: GripBinding,
    touch: Option<&'static str>,
}

const THUMBSTICK: NavigationBindings = NavigationBindings {
    axis: "/input/thumbstick",
    reset: "/input/thumbstick/click",
    grip: GripBinding::Pressure,
    touch: None,
};
const TRACKPAD: NavigationBindings = NavigationBindings {
    axis: "/input/trackpad",
    reset: "/input/trackpad/click",
    grip: GripBinding::Click,
    touch: Some("/input/trackpad/touch"),
};
const MOTION_CONTROLLER: NavigationBindings = NavigationBindings {
    grip: GripBinding::Click,
    ..THUMBSTICK
};
const PICO4: NavigationBindings = NavigationBindings {
    grip: GripBinding::Both,
    ..THUMBSTICK
};
const GENERIC_CONTROLLER: &str = "/interaction_profiles/khr/generic_controller";
const PROFILES: &[(&str, &str, Option<NavigationBindings>)] = &[
    (
        "/interaction_profiles/khr/simple_controller",
        "/input/select/click",
        None,
    ),
    (
        "/interaction_profiles/oculus/touch_controller",
        "/input/trigger/value",
        Some(THUMBSTICK),
    ),
    (
        "/interaction_profiles/bytedance/pico4_controller",
        "/input/trigger/click",
        Some(PICO4),
    ),
    (GENERIC_CONTROLLER, "/input/trigger/value", Some(THUMBSTICK)),
    (
        "/interaction_profiles/valve/index_controller",
        "/input/trigger/click",
        Some(THUMBSTICK),
    ),
    (
        "/interaction_profiles/htc/vive_controller",
        "/input/trigger/click",
        Some(TRACKPAD),
    ),
    (
        "/interaction_profiles/microsoft/motion_controller",
        "/input/trigger/value",
        Some(MOTION_CONTROLLER),
    ),
];

#[derive(Clone, Copy, Default)]
pub(super) struct Hand {
    pub pressed: bool,
    pub down: bool,
    pub active: bool,
    pub aim: Option<xr::Posef>,
}

#[derive(Default)]
pub(super) struct ViewMotion {
    pub zoom: f32,
    pub turn: Vec2,
    pub reset: bool,
}

#[derive(Default)]
pub(super) struct FrameInput {
    pub hands: [Hand; 2],
    pub seek_direction: Option<f64>,
    pub motion: ViewMotion,
}

impl FrameInput {
    fn apply_stick(&mut self, axis: Vec2, turning: bool, seek_armed: &mut bool) {
        self.seek_direction = self
            .seek_direction
            .or(stick_seek(seek_armed, axis, turning));
        if turning {
            let turn = Vec2::new(apply_dead_zone(axis.x), apply_dead_zone(axis.y));
            if turn.length_squared() > self.motion.turn.length_squared() {
                self.motion.turn = turn;
            }
        } else if axis.y.abs() > axis.x.abs() {
            let zoom = apply_dead_zone(axis.y);
            if zoom.abs() > self.motion.zoom.abs() {
                self.motion.zoom = zoom;
            }
        }
    }
}

pub(super) struct Controllers {
    pub view: xr::Space,
    aim_spaces: [xr::Space; 2],
    paths: [xr::Path; 2],
    select: xr::Action<bool>,
    aim: xr::Action<xr::Posef>,
    stick: xr::Action<xr::Vector2f>,
    reset: xr::Action<bool>,
    move_view: xr::Action<bool>,
    move_view_value: xr::Action<f32>,
    pad_touch: xr::Action<bool>,
    set: xr::ActionSet,
    synced: bool,
    seek_armed: [bool; 2],
    grip_held: bool,
}

impl Controllers {
    pub fn new(session: &xr::Session<xr::OpenGL>) -> Result<Self> {
        let instance = session.instance();
        let paths = [
            instance.string_to_path(HANDS[0])?,
            instance.string_to_path(HANDS[1])?,
        ];
        let set = instance.create_action_set("playback", "Video playback", 0)?;
        let select =
            set.create_action::<bool>("select", "Open or select playback controls", &paths)?;
        let aim = set.create_action::<xr::Posef>("aim", "Point at playback controls", &paths)?;
        let stick = set.create_action::<xr::Vector2f>("stick", "Zoom, seek, or turn", &paths)?;
        let reset = set.create_action::<bool>("reset_view", "Reset view", &paths)?;
        let move_view = set.create_action::<bool>("move_view", "Hold to turn the view", &paths)?;
        let move_view_value =
            set.create_action::<f32>("move_view_value", "Grip pressure for view rotation", &paths)?;
        let pad_touch = set.create_action::<bool>("pad_touch", "Trackpad contact", &paths)?;

        let mut supported = 0;
        for &(profile, trigger, navigation) in PROFILES {
            if profile == GENERIC_CONTROLLER && instance.exts().khr_generic_controller.is_none() {
                continue;
            }
            let mut bindings = Vec::with_capacity(12);
            bind_hands(instance, &mut bindings, &aim, "/input/aim/pose")?;
            bind_hands(instance, &mut bindings, &select, trigger)?;
            if let Some(navigation) = navigation {
                bind_hands(instance, &mut bindings, &stick, navigation.axis)?;
                bind_hands(instance, &mut bindings, &reset, navigation.reset)?;
                // SteamVR maps a boolean squeeze action to a physical click. Analog grips
                // such as Steam Link's Pico controller expose the pressure component.
                if matches!(navigation.grip, GripBinding::Pressure | GripBinding::Both) {
                    bind_hands(
                        instance,
                        &mut bindings,
                        &move_view_value,
                        "/input/squeeze/value",
                    )?;
                }
                if matches!(navigation.grip, GripBinding::Click | GripBinding::Both) {
                    bind_hands(instance, &mut bindings, &move_view, "/input/squeeze/click")?;
                }
                if let Some(touch) = navigation.touch {
                    bind_hands(instance, &mut bindings, &pad_touch, touch)?;
                }
            }
            match instance
                .suggest_interaction_profile_bindings(instance.string_to_path(profile)?, &bindings)
            {
                Ok(()) => {
                    supported += 1;
                    log::debug!("OpenXR playback controls binding: {profile}");
                }
                Err(xr::sys::Result::ERROR_PATH_UNSUPPORTED) => {
                    log::debug!("OpenXR controller binding unavailable: {profile}");
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("{} ({profile})", tr(Text::ControllerBindingFailed))
                    });
                }
            }
        }
        ensure!(supported > 0, tr(Text::ControllerProfileUnavailable));

        let view =
            session.create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;
        let aim_spaces = [
            aim.create_space(session, paths[0], xr::Posef::IDENTITY)?,
            aim.create_space(session, paths[1], xr::Posef::IDENTITY)?,
        ];
        session.attach_action_sets(&[&set])?;
        Ok(Self {
            view,
            aim_spaces,
            paths,
            select,
            aim,
            stick,
            reset,
            move_view,
            move_view_value,
            pad_touch,
            set,
            synced: false,
            seek_armed: [false; 2],
            grip_held: false,
        })
    }

    pub fn reset_input(&mut self) {
        self.synced = false;
        self.seek_armed = [false; 2];
        self.grip_held = false;
    }

    pub fn read(
        &mut self,
        session: &xr::Session<xr::OpenGL>,
        time: xr::Time,
        focused: bool,
        locate_aim: bool,
    ) -> Result<FrameInput> {
        let mut input = FrameInput::default();
        if !focused {
            self.reset_input();
            return Ok(input);
        }

        session.sync_actions(&[xr::ActiveActionSet::new(&self.set)])?;
        let was_synced = std::mem::replace(&mut self.synced, true);
        // A grip on either hand modifies both sticks, so use OpenXR's combined action state.
        let value = self.move_view_value.state(session, xr::Path::NULL)?;
        let click = self.move_view.state(session, xr::Path::NULL)?;
        let pressure = value.is_active.then_some(value.current_state);
        let clicked = click.is_active && click.current_state;
        let turning = grip_is_held(self.grip_held, pressure, clicked);
        if !was_synced || turning != self.grip_held {
            log::debug!(
                "OpenXR grip held={turning} (pressure={pressure:?}, click={:?})",
                click.is_active.then_some(click.current_state)
            );
        }
        self.grip_held = turning;

        for index in 0..self.paths.len() {
            let path = self.paths[index];
            let state = self.select.state(session, path)?;
            let hand = &mut input.hands[index];
            hand.active = state.is_active;
            hand.down = state.is_active && state.current_state;
            hand.pressed = was_synced && hand.down && state.changed_since_last_sync;
            if (locate_aim || hand.pressed) && self.aim.is_active(session, path)? {
                let aim = self.aim_spaces[index].locate(&self.view, time)?;
                let required = xr::SpaceLocationFlags::ORIENTATION_VALID
                    | xr::SpaceLocationFlags::POSITION_VALID;
                if aim.location_flags.contains(required) {
                    hand.aim = Some(aim.pose);
                }
            }

            let reset = self.reset.state(session, path)?;
            input.motion.reset |= was_synced
                && reset.is_active
                && reset.current_state
                && reset.changed_since_last_sync;
            let stick = self.stick.state(session, path)?;
            if !stick.is_active {
                self.seek_armed[index] = false;
                continue;
            }
            let touch = self.pad_touch.state(session, path)?;
            let axis = Vec2::new(stick.current_state.x, stick.current_state.y);
            let axis = if axis.is_finite() && (!touch.is_active || touch.current_state) {
                axis
            } else {
                Vec2::ZERO
            };
            input.apply_stick(axis, turning, &mut self.seek_armed[index]);
        }
        if input.motion.reset {
            input.seek_direction = None;
        }
        Ok(input)
    }

    pub fn log_profiles(&self, session: &xr::Session<xr::OpenGL>) {
        for (hand, path) in HANDS.into_iter().zip(self.paths) {
            if let Ok(profile) = session.current_interaction_profile(path)
                && profile != xr::Path::NULL
                && let Ok(name) = session.instance().path_to_string(profile)
            {
                log::info!("OpenXR controller {hand}: {name}");
            }
        }
        if log::log_enabled!(log::Level::Debug) {
            let source_paths = |paths: xr::Result<Vec<xr::Path>>| {
                paths.map(|paths| {
                    paths
                        .into_iter()
                        .filter_map(|path| session.instance().path_to_string(path).ok())
                        .collect::<Vec<_>>()
                })
            };
            log::debug!(
                "OpenXR grip sources: pressure={:?}, click={:?}",
                source_paths(self.move_view_value.bound_sources(session)),
                source_paths(self.move_view.bound_sources(session))
            );
        }
    }
}

fn grip_is_held(was_held: bool, pressure: Option<f32>, clicked: bool) -> bool {
    let threshold = if was_held { GRIP_RELEASE } else { GRIP_PRESS };
    clicked || pressure.is_some_and(|value| value.is_finite() && value >= threshold)
}

fn bind_hands<'a, T: xr::ActionTy>(
    instance: &xr::Instance,
    bindings: &mut Vec<xr::Binding<'a>>,
    action: &'a xr::Action<T>,
    component: &str,
) -> xr::Result<()> {
    for hand in HANDS {
        let path = instance.string_to_path(&format!("{hand}{component}"))?;
        bindings.push(xr::Binding::new(action, path));
    }
    Ok(())
}

fn stick_seek(armed: &mut bool, axis: Vec2, turning: bool) -> Option<f64> {
    let horizontal = axis.x.abs();
    if horizontal <= 0.35 {
        *armed = true;
    } else if horizontal >= 0.72 && (turning || horizontal >= axis.y.abs()) {
        let was_armed = std::mem::replace(armed, false);
        return (was_armed && !turning).then_some(f64::from(axis.x.signum()));
    }
    None
}

fn apply_dead_zone(value: f32) -> f32 {
    ((value.abs() - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE)).clamp(0.0, 1.0) * value.signum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analog_grip_stays_held_between_press_and_release_thresholds() {
        assert!(!grip_is_held(false, Some(0.25), false));
        assert!(grip_is_held(false, Some(0.35), false));
        assert!(grip_is_held(true, Some(0.25), false));
        assert!(!grip_is_held(true, Some(0.15), false));
        assert!(!grip_is_held(true, None, false));
        assert!(grip_is_held(false, None, true));
    }

    #[test]
    fn held_grip_routes_horizontal_stick_to_rotation() {
        let mut armed = true;
        let axis = Vec2::new(0.9, 0.5);
        let grip = grip_is_held(false, Some(0.38), false);
        let mut frame = FrameInput::default();
        frame.apply_stick(axis, grip, &mut armed);
        assert!(frame.motion.turn.x > 0.0 && frame.motion.turn.y > 0.0);
        assert_eq!(frame.seek_direction, None);

        // Releasing the grip while the stick is still tilted must not seek.
        frame.apply_stick(axis, false, &mut armed);
        assert_eq!(frame.seek_direction, None);
        frame.apply_stick(Vec2::ZERO, false, &mut armed);
        frame.apply_stick(axis, false, &mut armed);
        assert_eq!(frame.seek_direction, Some(1.0));
    }
}
