//! Direct-touch routing in video coordinates, independent of pointer capture.
use moonlight_sys::{ConnectionControl, InputAction, InputError, MouseButton, TouchEventType};
use scarlet_ui::event::{TouchChange, TouchPhase};

trait TouchSink {
    fn supports_touch(&self) -> Result<bool, InputError>;
    fn touch(&self, phase: TouchEventType, id: u32, x: f32, y: f32) -> Result<(), InputError>;
    fn position(&self, x: f32, y: f32) -> Result<(), InputError>;
    fn button(&self, pressed: bool) -> Result<(), InputError>;
}
impl TouchSink for ConnectionControl {
    fn supports_touch(&self) -> Result<bool, InputError> {
        self.supports_touch()
    }
    fn touch(&self, phase: TouchEventType, id: u32, x: f32, y: f32) -> Result<(), InputError> {
        self.send_touch(phase, id, x, y)
    }
    fn position(&self, x: f32, y: f32) -> Result<(), InputError> {
        self.send_mouse_position(x, y)
    }
    fn button(&self, pressed: bool) -> Result<(), InputError> {
        self.send_mouse_button(
            MouseButton::Left,
            if pressed {
                InputAction::Press
            } else {
                InputAction::Release
            },
        )
    }
}

struct Contact {
    seat: u32,
    local_id: u64,
    remote_id: u32,
    position: (f32, f32),
}

#[derive(Default)]
pub(crate) struct TouchInput {
    native: Option<bool>,
    contacts: Vec<Contact>,
}
impl TouchInput {
    pub(crate) fn handle(
        &mut self,
        change: TouchChange,
        viewport: (u32, u32),
        video: (u32, u32),
        control: &ConnectionControl,
    ) -> Result<bool, InputError> {
        self.route(change, viewport, video, control)
    }
    fn route(
        &mut self,
        change: TouchChange,
        viewport: (u32, u32),
        video: (u32, u32),
        sink: &impl TouchSink,
    ) -> Result<bool, InputError> {
        let existing = self
            .contacts
            .iter()
            .position(|c| c.seat == change.seat_id && c.local_id == change.id);
        let point = video_position(
            change.x,
            change.y,
            viewport,
            video,
            change.phase != TouchPhase::Down,
        );
        if change.phase == TouchPhase::Down {
            if existing.is_some() {
                return Ok(true);
            }
            let Some((x, y)) = point else {
                return Ok(false);
            };
            let native = match self.native {
                Some(value) => value,
                None => {
                    let value = sink.supports_touch()?;
                    self.native = Some(value);
                    value
                }
            };
            if !native && !self.contacts.is_empty() {
                return Ok(false);
            }
            let Some(id) = (0..10).find(|id| !self.contacts.iter().any(|c| c.remote_id == *id))
            else {
                return Ok(false);
            };
            if native {
                sink.touch(TouchEventType::Down, id, x, y)?;
            } else {
                sink.position(x, y)?;
                sink.button(true)?;
            }
            self.contacts.push(Contact {
                seat: change.seat_id,
                local_id: change.id,
                remote_id: id,
                position: (x, y),
            });
            return Ok(true);
        }
        let Some(index) = existing else {
            return Ok(false);
        };
        let contact = &self.contacts[index];
        let (x, y) = point.unwrap_or(contact.position);
        let ending = matches!(change.phase, TouchPhase::Up | TouchPhase::Cancel);
        let result = if self.native == Some(true) {
            let phase = match change.phase {
                TouchPhase::Move => TouchEventType::Move,
                TouchPhase::Up => TouchEventType::Up,
                TouchPhase::Cancel => TouchEventType::Cancel,
                TouchPhase::Down => unreachable!(),
            };
            sink.touch(phase, contact.remote_id, x, y)
        } else if ending {
            // A failed final position must not leave the mouse button pressed.
            let position = if change.phase == TouchPhase::Up {
                sink.position(x, y)
            } else {
                Ok(())
            };
            let release = sink.button(false);
            position.and(release)
        } else {
            sink.position(x, y)
        };
        if ending && result.is_ok() {
            self.contacts.remove(index);
        } else {
            self.contacts[index].position = (x, y);
        }
        result.map(|()| true)
    }
    pub(crate) fn release(&mut self, control: &ConnectionControl) -> Result<(), InputError> {
        self.release_to(control)
    }
    fn release_to(&mut self, sink: &impl TouchSink) -> Result<(), InputError> {
        if self.contacts.is_empty() {
            return Ok(());
        }
        let result = if self.native == Some(true) {
            sink.touch(TouchEventType::CancelAll, 0, 0.0, 0.0)
        } else {
            sink.button(false)
        };
        self.contacts.clear();
        self.native = None;
        result
    }
}

fn video_position(
    x: i32,
    y: i32,
    viewport: (u32, u32),
    video: (u32, u32),
    clamp: bool,
) -> Option<(f32, f32)> {
    let (w, h) = crate::video::fit_size(video.0, video.1, viewport.0, viewport.1);
    if w == 0 || h == 0 {
        return None;
    }
    let x = (x as f32 - (viewport.0 - w) as f32 * 0.5) / w as f32;
    let y = (y as f32 - (viewport.1 - h) as f32 * 0.5) / h as f32;
    if !clamp && (!(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y)) {
        return None;
    }
    Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Debug, PartialEq)]
    enum Sent {
        Touch(TouchEventType, u32, f32, f32),
        Position(f32, f32),
        Button(bool),
    }
    struct Sink {
        native: bool,
        sent: RefCell<Vec<Sent>>,
    }
    impl Sink {
        fn new(native: bool) -> Self {
            Self {
                native,
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl TouchSink for Sink {
        fn supports_touch(&self) -> Result<bool, InputError> {
            Ok(self.native)
        }
        fn touch(&self, phase: TouchEventType, id: u32, x: f32, y: f32) -> Result<(), InputError> {
            self.sent.borrow_mut().push(Sent::Touch(phase, id, x, y));
            Ok(())
        }
        fn position(&self, x: f32, y: f32) -> Result<(), InputError> {
            self.sent.borrow_mut().push(Sent::Position(x, y));
            Ok(())
        }
        fn button(&self, pressed: bool) -> Result<(), InputError> {
            self.sent.borrow_mut().push(Sent::Button(pressed));
            Ok(())
        }
    }
    fn event(id: u64, phase: TouchPhase, x: i32, y: i32) -> TouchChange {
        TouchChange {
            seat_id: 0,
            serial: 1,
            time_ns: 1,
            id,
            phase,
            x,
            y,
            pressure: None,
            touch_major: None,
        }
    }
    #[test]
    fn letterbox_is_not_remote_video_and_coordinates_follow_aspect_fit() {
        assert_eq!(
            video_position(400, 100, (800, 800), (1920, 1080), false),
            None
        );
        assert_eq!(
            video_position(400, 400, (800, 800), (1920, 1080), false),
            Some((0.5, 0.5))
        );
        assert_eq!(
            video_position(400, 900, (800, 800), (1920, 1080), true),
            Some((0.5, 1.0))
        );
    }
    #[test]
    fn native_contacts_keep_distinct_ids_and_release_outside_the_image() {
        let sink = Sink::new(true);
        let mut input = TouchInput::default();
        for id in [1, (1u64 << 32) + 1] {
            input
                .route(
                    event(id, TouchPhase::Down, 400, 400),
                    (800, 800),
                    (1920, 1080),
                    &sink,
                )
                .unwrap();
        }
        input
            .route(
                event(1, TouchPhase::Up, 900, 900),
                (800, 800),
                (1920, 1080),
                &sink,
            )
            .unwrap();
        input
            .route(
                event((1u64 << 32) + 1, TouchPhase::Cancel, 0, 0),
                (0, 0),
                (1920, 1080),
                &sink,
            )
            .unwrap();
        assert_eq!(
            *sink.sent.borrow(),
            vec![
                Sent::Touch(TouchEventType::Down, 0, 0.5, 0.5),
                Sent::Touch(TouchEventType::Down, 1, 0.5, 0.5),
                Sent::Touch(TouchEventType::Up, 0, 1.0, 1.0),
                Sent::Touch(TouchEventType::Cancel, 1, 0.5, 0.5)
            ]
        );
        assert!(input.contacts.is_empty());
    }
    #[test]
    fn focus_loss_cancels_native_contacts() {
        let sink = Sink::new(true);
        let mut input = TouchInput::default();
        input
            .route(
                event(9, TouchPhase::Down, 400, 400),
                (800, 800),
                (1920, 1080),
                &sink,
            )
            .unwrap();
        input.release_to(&sink).unwrap();
        assert_eq!(
            sink.sent.borrow().last(),
            Some(&Sent::Touch(TouchEventType::CancelAll, 0, 0.0, 0.0))
        );
        assert!(input.contacts.is_empty());
        assert!(
            !input
                .route(
                    event(9, TouchPhase::Move, 410, 410),
                    (800, 800),
                    (1920, 1080),
                    &sink
                )
                .unwrap()
        );
    }
    #[test]
    fn unsupported_host_uses_one_finger_absolute_mouse_drag() {
        let sink = Sink::new(false);
        let mut input = TouchInput::default();
        input
            .route(
                event(1, TouchPhase::Down, 400, 400),
                (800, 800),
                (1920, 1080),
                &sink,
            )
            .unwrap();
        assert!(
            !input
                .route(
                    event(2, TouchPhase::Down, 500, 400),
                    (800, 800),
                    (1920, 1080),
                    &sink
                )
                .unwrap()
        );
        input
            .route(
                event(1, TouchPhase::Move, 600, 400),
                (800, 800),
                (1920, 1080),
                &sink,
            )
            .unwrap();
        input.release_to(&sink).unwrap();
        assert_eq!(
            *sink.sent.borrow(),
            vec![
                Sent::Position(0.5, 0.5),
                Sent::Button(true),
                Sent::Position(0.75, 0.5),
                Sent::Button(false)
            ]
        );
    }
    #[test]
    fn letterbox_tap_never_presses_remote_input() {
        let sink = Sink::new(true);
        let mut input = TouchInput::default();
        assert!(
            !input
                .route(
                    event(1, TouchPhase::Down, 100, 100),
                    (800, 800),
                    (1920, 1080),
                    &sink
                )
                .unwrap()
        );
        assert!(
            !input
                .route(
                    event(1, TouchPhase::Up, 400, 400),
                    (800, 800),
                    (1920, 1080),
                    &sink
                )
                .unwrap()
        );
        assert!(sink.sent.borrow().is_empty());
    }
}
