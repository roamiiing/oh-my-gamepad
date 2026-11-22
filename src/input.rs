use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub struct CapturedTime {
    instant: Instant,
    unix_ms: u64,
}

impl CapturedTime {
    pub fn now() -> Self {
        Self {
            instant: Instant::now(),
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    pub fn since(self, start: Instant) -> Timestamp {
        Timestamp {
            monotonic_ms: self.instant.saturating_duration_since(start).as_millis() as u64,
            unix_ms: self.unix_ms,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Timestamp {
    pub monotonic_ms: u64,
    pub unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn trigger_button(self) -> Button {
        match self {
            Self::Left => Button::Lt,
            Self::Right => Button::Rt,
        }
    }
}

macro_rules! buttons {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub enum Button { $($variant),+ }

        impl Button {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            pub fn parse(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $name,)+ }
            }
        }
    };
}

buttons! {
    A => "a", B => "b", X => "x", Y => "y",
    Lb => "lb", Rb => "rb", Lsb => "lsb", Rsb => "rsb",
    Lt => "lt", Rt => "rt", Menu => "menu", Options => "options",
    Home => "home", DpadUp => "dpad_up", DpadDown => "dpad_down",
    DpadLeft => "dpad_left", DpadRight => "dpad_right", Share => "share",
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Source {
    Stick(Side),
    Trigger(Side),
    Button(Button),
}

impl Source {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Stick(_) => "stick",
            Self::Trigger(_) => "trigger",
            Self::Button(_) => "button",
        }
    }

    pub fn physical(self) -> Self {
        match self {
            Self::Button(Button::Lt) => Self::Trigger(Side::Left),
            Self::Button(Button::Rt) => Self::Trigger(Side::Right),
            _ => self,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Measurement {
    Connected,
    Disconnected,
    Stick(Side, f32, f32),
    Trigger(Side, f32),
    Button(Button, bool),
}

#[derive(Clone, Copy, Debug)]
pub enum Change {
    Stick(Side, f32, f32),
    Trigger(Side, f32),
    Button(Button, bool),
}

impl Change {
    pub fn source(self) -> Source {
        match self {
            Self::Stick(side, _, _) => Source::Stick(side),
            Self::Trigger(side, _) => Source::Trigger(side),
            Self::Button(button, _) => Source::Button(button),
        }
    }
}

pub const STICK_THRESHOLD: f32 = 0.25;
const TRIGGER_THRESHOLD: f32 = 0.30;

#[derive(Clone, Default)]
pub struct Controller {
    pub connected: bool,
    pub sticks: [(f32, f32); 2],
    pub triggers: [f32; 2],
    pub pressed: BTreeMap<Button, u64>,
}

impl Controller {
    pub fn update(&mut self, measurement: Measurement, time: u64) -> (Option<Source>, Vec<Change>) {
        let mut changes = Vec::new();
        let mut mapping = None;
        match measurement {
            Measurement::Stick(side, x, y) => {
                let previous = &mut self.sticks[side as usize];
                if *previous != (x, y) {
                    let was_outside = previous.0.hypot(previous.1) > STICK_THRESHOLD;
                    if !was_outside && x.hypot(y) > STICK_THRESHOLD {
                        mapping = Some(Source::Stick(side));
                    }
                    *previous = (x, y);
                    changes.push(Change::Stick(side, x, y));
                }
            }
            Measurement::Trigger(side, value) => {
                let previous = &mut self.triggers[side as usize];
                if *previous != value {
                    *previous = value;
                    changes.push(Change::Trigger(side, value));
                    if value != TRIGGER_THRESHOLD {
                        mapping = self.update_button(
                            side.trigger_button(),
                            value > TRIGGER_THRESHOLD,
                            time,
                            &mut changes,
                        );
                    }
                }
            }
            Measurement::Button(button, down) => {
                mapping = self.update_button(button, down, time, &mut changes);
            }
            Measurement::Connected | Measurement::Disconnected => {}
        }
        (mapping, changes)
    }

    fn update_button(
        &mut self,
        button: Button,
        down: bool,
        time: u64,
        changes: &mut Vec<Change>,
    ) -> Option<Source> {
        if self.pressed.contains_key(&button) == down {
            return None;
        }
        changes.push(Change::Button(button, down));
        if down {
            self.pressed.insert(button, time);
            Some(Source::Button(button))
        } else {
            self.pressed.remove(&button);
            None
        }
    }
}
