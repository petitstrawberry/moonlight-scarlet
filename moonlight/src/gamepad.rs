//! ScarletUI snapshots translated to session-local GameStream controllers.

use std::sync::{Arc, Mutex, MutexGuard};

use moonlight_sys::{ConnectionControl, ControllerButton as Button, ControllerState, InputError};
use scarlet_ui::{GamepadButton, GamepadEvent};

const MAX_GAMEPADS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Device {
    id: u32,
    state: ControllerState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Packet {
    Arrival {
        slot: u8,
        mask: u16,
    },
    State {
        slot: u8,
        mask: u16,
        state: ControllerState,
    },
}

impl Packet {
    fn send(self, control: &ConnectionControl) -> Result<(), InputError> {
        match self {
            Self::Arrival { slot, mask } => control.send_controller_arrival(slot, mask),
            Self::State { slot, mask, state } => control.send_controller(slot, mask, state),
        }
    }
}

#[derive(Default)]
struct Gamepads {
    devices: [Option<Device>; MAX_GAMEPADS],
    sent: [Option<Device>; MAX_GAMEPADS],
}

impl Gamepads {
    fn observe(&mut self, event: GamepadEvent) {
        let existing = self
            .devices
            .iter()
            .position(|device| device.is_some_and(|device| device.id == event.device_id));
        if event.reset {
            if let Some(slot) = existing {
                self.devices[slot] = None;
            }
        } else if let Some(slot) =
            existing.or_else(|| self.devices.iter().position(Option::is_none))
        {
            self.devices[slot] = Some(Device {
                id: event.device_id,
                state: translate(event),
            });
        }
    }

    fn sync(
        &mut self,
        mut send: impl FnMut(Packet) -> Result<(), InputError>,
    ) -> Result<(), InputError> {
        // Remove old identities before reusing their slots. Commit only queued
        // packets so a transient failure can be retried without losing releases.
        for slot in 0..MAX_GAMEPADS {
            if self.sent[slot].is_some()
                && self.sent[slot].map(|device| device.id)
                    != self.devices[slot].map(|device| device.id)
            {
                send(Packet::State {
                    slot: slot as u8,
                    mask: mask(&self.sent) & !(1 << slot),
                    state: ControllerState::default(),
                })?;
                self.sent[slot] = None;
            }
        }
        for slot in 0..MAX_GAMEPADS {
            let Some(device) = self.devices[slot] else {
                continue;
            };
            if self.sent[slot].is_none() {
                send(Packet::Arrival {
                    slot: slot as u8,
                    mask: mask(&self.sent) | (1 << slot),
                })?;
                self.sent[slot] = Some(Device {
                    id: device.id,
                    state: ControllerState::default(),
                });
            }
            if self.sent[slot] != Some(device) {
                send(Packet::State {
                    slot: slot as u8,
                    mask: mask(&self.sent),
                    state: device.state,
                })?;
                self.sent[slot] = Some(device);
            }
        }
        Ok(())
    }
}

fn mask(devices: &[Option<Device>; MAX_GAMEPADS]) -> u16 {
    devices.iter().enumerate().fold(0, |mask, (slot, device)| {
        mask | if device.is_some() { 1 << slot } else { 0 }
    })
}

/// Shared across UI snapshots and the stream worker. Local device IDs never
/// become protocol indices, and existing players keep their slots on hotplug.
#[derive(Clone, Default)]
pub(crate) struct RemoteGamepads(Arc<Mutex<Gamepads>>);

impl RemoteGamepads {
    fn lock(&self) -> MutexGuard<'_, Gamepads> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn observe(&self, event: GamepadEvent) {
        self.lock().observe(event);
    }

    pub(crate) fn active_mask(&self) -> u16 {
        mask(&self.lock().devices)
    }

    pub(crate) fn sync(&self, control: &ConnectionControl) -> Result<(), InputError> {
        self.lock().sync(|packet| packet.send(control))
    }

    pub(crate) fn release_all(
        &self,
        control: Option<&ConnectionControl>,
    ) -> Result<(), InputError> {
        let mut gamepads = self.lock();
        gamepads.devices.fill(None);
        if let Some(control) = control {
            gamepads.sync(|packet| packet.send(control))
        } else {
            gamepads.sent.fill(None);
            Ok(())
        }
    }

    /// The old connection no longer exists; future sessions must announce anew.
    pub(crate) fn reset_session(&self) {
        self.lock().sent.fill(None);
    }
}

fn translate(event: GamepadEvent) -> ControllerState {
    if event.reset {
        return ControllerState::default();
    }
    let mut buttons = 0;
    for (local, remote) in [
        (GamepadButton::South, Button::A),
        (GamepadButton::East, Button::B),
        (GamepadButton::West, Button::X),
        (GamepadButton::North, Button::Y),
        (GamepadButton::LeftShoulder, Button::LeftShoulder),
        (GamepadButton::RightShoulder, Button::RightShoulder),
        (GamepadButton::Select, Button::Back),
        (GamepadButton::Start, Button::Start),
        (GamepadButton::Home, Button::Home),
        (GamepadButton::LeftStick, Button::LeftStick),
        (GamepadButton::RightStick, Button::RightStick),
    ] {
        if event.pressed(local) {
            buttons |= remote as u32;
        }
    }
    for (pressed, button) in [
        (event.hat_x < 0, Button::Left),
        (event.hat_x > 0, Button::Right),
        (event.hat_y < 0, Button::Up),
        (event.hat_y > 0, Button::Down),
    ] {
        if pressed {
            buttons |= button as u32;
        }
    }
    ControllerState {
        buttons,
        left_trigger: trigger(
            event.left_trigger,
            event.pressed(GamepadButton::LeftTrigger),
        ),
        right_trigger: trigger(
            event.right_trigger,
            event.pressed(GamepadButton::RightTrigger),
        ),
        left_x: event.left_x,
        left_y: event.left_y.saturating_neg(),
        right_x: event.right_x,
        right_y: event.right_y.saturating_neg(),
    }
}

fn trigger(value: u16, digital: bool) -> u8 {
    // Some devices expose both an analog axis and a threshold button. Preserve
    // the analog pressure instead of snapping to full scale at that threshold.
    if digital && value == 0 {
        255
    } else {
        ((u32::from(value.min(32767)) * 255 + 16383) / 32767) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: u32) -> GamepadEvent {
        GamepadEvent {
            device_id: id,
            ..GamepadEvent::default()
        }
    }

    fn flush(gamepads: &mut Gamepads) -> Vec<Packet> {
        let mut packets = Vec::new();
        gamepads
            .sync(|packet| {
                packets.push(packet);
                Ok(())
            })
            .unwrap();
        packets
    }

    #[test]
    fn maps_positions_hat_sticks_and_trigger_endpoints() {
        let state = translate(GamepadEvent {
            buttons: (1 << GamepadButton::South as u8) | (1 << GamepadButton::West as u8),
            hat_x: -1,
            hat_y: 1,
            left_x: -32767,
            left_y: -32767,
            right_x: 32767,
            right_y: i16::MIN,
            left_trigger: 16384,
            right_trigger: u16::MAX,
            ..event(900)
        });
        assert_eq!(state.buttons, 0x5006);
        assert_eq!(
            (state.left_x, state.left_y, state.right_x, state.right_y),
            (-32767, 32767, 32767, 32767)
        );
        assert_eq!((state.left_trigger, state.right_trigger), (128, 255));
        assert_eq!(trigger(0, false), 0);
        assert_eq!(trigger(32767, false), 255);
        assert_eq!(trigger(0, true), 255);
        assert_eq!(trigger(16384, true), 128);
    }

    #[test]
    fn uses_stable_slots_and_reuses_only_the_removed_slot() {
        let mut pads = Gamepads::default();
        pads.observe(event(900));
        pads.observe(event(42));
        assert_eq!(
            flush(&mut pads),
            vec![
                Packet::Arrival { slot: 0, mask: 1 },
                Packet::Arrival { slot: 1, mask: 3 }
            ]
        );
        pads.observe(GamepadEvent {
            reset: true,
            ..event(900)
        });
        pads.observe(event(12345));
        assert_eq!(
            flush(&mut pads),
            vec![
                Packet::State {
                    slot: 0,
                    mask: 2,
                    state: ControllerState::default()
                },
                Packet::Arrival { slot: 0, mask: 3 },
            ]
        );
        assert_eq!(pads.sent[1].unwrap().id, 42);
    }

    #[test]
    fn sends_held_state_after_arrival_and_deduplicates_snapshots() {
        let mut pads = Gamepads::default();
        let held = GamepadEvent {
            buttons: 1,
            left_trigger: 32767,
            ..event(1)
        };
        pads.observe(held);
        assert_eq!(
            flush(&mut pads),
            vec![
                Packet::Arrival { slot: 0, mask: 1 },
                Packet::State {
                    slot: 0,
                    mask: 1,
                    state: translate(held)
                },
            ]
        );
        pads.observe(GamepadEvent {
            time_ns: 99,
            ..held
        });
        assert!(flush(&mut pads).is_empty());
        pads.observe(event(1));
        assert_eq!(
            flush(&mut pads),
            vec![Packet::State {
                slot: 0,
                mask: 1,
                state: ControllerState::default()
            }]
        );
    }

    #[test]
    fn reset_ignores_payload_and_unknown_devices() {
        let mut pads = Gamepads::default();
        pads.observe(event(5));
        flush(&mut pads);
        pads.observe(GamepadEvent {
            reset: true,
            buttons: u32::MAX,
            ..event(99)
        });
        assert!(flush(&mut pads).is_empty());
        pads.observe(GamepadEvent {
            reset: true,
            buttons: u32::MAX,
            ..event(5)
        });
        assert_eq!(
            flush(&mut pads),
            vec![Packet::State {
                slot: 0,
                mask: 0,
                state: ControllerState::default()
            }]
        );
    }

    #[test]
    fn failed_release_is_retried_before_slot_reuse() {
        let mut pads = Gamepads::default();
        pads.observe(event(1));
        flush(&mut pads);
        pads.observe(GamepadEvent {
            reset: true,
            ..event(1)
        });
        assert_eq!(
            pads.sync(|_| Err(InputError::ConnectionInactive)),
            Err(InputError::ConnectionInactive)
        );
        pads.observe(event(2));
        assert_eq!(
            flush(&mut pads),
            vec![
                Packet::State {
                    slot: 0,
                    mask: 0,
                    state: ControllerState::default()
                },
                Packet::Arrival { slot: 0, mask: 1 },
            ]
        );
    }

    #[test]
    fn caps_devices_at_sixteen_without_wrapping_ids() {
        let mut pads = Gamepads::default();
        for id in 100..117 {
            pads.observe(event(id));
        }
        assert_eq!(mask(&pads.devices), u16::MAX);
        assert_eq!(flush(&mut pads).len(), 16);
        assert_eq!(pads.sent[15].unwrap().id, 115);
    }

    #[test]
    fn failed_state_packet_retries_without_reannouncing() {
        let mut pads = Gamepads::default();
        let held = GamepadEvent {
            buttons: 1,
            ..event(1)
        };
        pads.observe(held);
        pads.sync(|packet| match packet {
            Packet::Arrival { .. } => Ok(()),
            Packet::State { .. } => Err(InputError::ConnectionInactive),
        })
        .unwrap_err();
        assert_eq!(
            flush(&mut pads),
            vec![Packet::State {
                slot: 0,
                mask: 1,
                state: translate(held)
            }]
        );
    }

    #[test]
    fn focus_loss_releases_every_device_and_session_restart_reannounces() {
        let remote = RemoteGamepads::default();
        remote.observe(GamepadEvent {
            buttons: 1,
            ..event(1)
        });
        remote.observe(event(2));
        flush(&mut remote.lock());
        remote.reset_session();
        assert_eq!(flush(&mut remote.lock()).len(), 3);
        // The same clear-and-sync path used by release_all with a live control.
        let mut pads = remote.lock();
        pads.devices.fill(None);
        assert_eq!(
            flush(&mut pads),
            vec![
                Packet::State {
                    slot: 0,
                    mask: 2,
                    state: ControllerState::default()
                },
                Packet::State {
                    slot: 1,
                    mask: 0,
                    state: ControllerState::default()
                },
            ]
        );
        drop(pads);
        remote.release_all(None).unwrap();
        assert_eq!(remote.active_mask(), 0);
    }
}
