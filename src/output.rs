use crate::config::Result;
use std::collections::{BTreeMap, BTreeSet};
use yaml_rust2::Yaml;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Key(pub u16);

impl Key {
    pub fn parse(value: &Yaml) -> Result<Self> {
        // macOS virtual key codes use the physical positions of the ANSI keyboard.
        if let Some(code) = value.as_i64().and_then(|code| u16::try_from(code).ok()) {
            return Ok(Self(code));
        }
        let name = value.as_str().ok_or("ожидается имя или код клавиши")?;
        let code = match name.to_ascii_lowercase().as_str() {
            "a" => 0,
            "s" => 1,
            "d" => 2,
            "f" => 3,
            "h" => 4,
            "g" => 5,
            "z" => 6,
            "x" => 7,
            "c" => 8,
            "v" => 9,
            "b" => 11,
            "q" => 12,
            "w" => 13,
            "e" => 14,
            "r" => 15,
            "y" => 16,
            "t" => 17,
            "1" => 18,
            "2" => 19,
            "3" => 20,
            "4" => 21,
            "6" => 22,
            "5" => 23,
            "=" => 24,
            "9" => 25,
            "7" => 26,
            "-" => 27,
            "8" => 28,
            "0" => 29,
            "]" => 30,
            "o" => 31,
            "u" => 32,
            "[" => 33,
            "i" => 34,
            "p" => 35,
            "return" | "enter" => 36,
            "l" => 37,
            "j" => 38,
            "'" => 39,
            "k" => 40,
            ";" => 41,
            "\\" => 42,
            "," => 43,
            "/" => 44,
            "n" => 45,
            "m" => 46,
            "." => 47,
            "tab" => 48,
            "space" => 49,
            "`" => 50,
            "backspace" => 51,
            "escape" | "esc" => 53,
            "right_command" => 54,
            "command" | "cmd" => 55,
            "shift" => 56,
            "caps_lock" => 57,
            "option" | "alt" => 58,
            "control" | "ctrl" => 59,
            "right_shift" => 60,
            "right_option" => 61,
            "right_control" => 62,
            "fn" => 63,
            "f5" => 96,
            "f6" => 97,
            "f7" => 98,
            "f3" => 99,
            "f8" => 100,
            "f9" => 101,
            "f11" => 103,
            "f10" => 109,
            "f12" => 111,
            "home" => 115,
            "page_up" => 116,
            "delete" => 117,
            "f4" => 118,
            "end" => 119,
            "f2" => 120,
            "page_down" => 121,
            "f1" => 122,
            "left" => 123,
            "right" => 124,
            "down" => 125,
            "up" => 126,
            _ => return Err(format!("неизвестная клавиша {name:?}")),
        };
        Ok(Self(code))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn parse(value: &Yaml) -> Result<Self> {
        match value.as_str() {
            Some("left") => Ok(Self::Left),
            Some("right") => Ok(Self::Right),
            Some("middle") => Ok(Self::Middle),
            _ => Err("ожидается кнопка мыши left, right или middle".into()),
        }
    }
}

pub trait Output {
    fn key(&mut self, key: Key, down: bool) -> Result<()>;
    fn mouse(&mut self, button: MouseButton, down: bool) -> Result<()>;
    fn move_pointer(&mut self, dx: f64, dy: f64) -> Result<()>;
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()>;
    fn type_text(&mut self, text: &str) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct Origin {
    pub plugin: String,
    pub activation_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HeldInput {
    Key(Key),
    Mouse(MouseButton),
}

impl HeldInput {
    fn send(self, output: &mut impl Output, down: bool) -> Result<()> {
        match self {
            Self::Key(key) => output.key(key, down),
            Self::Mouse(button) => output.mouse(button, down),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Owner {
    Behavior(u64),
    Plugin(String),
}

impl From<&Origin> for Owner {
    fn from(origin: &Origin) -> Self {
        match origin.activation_id {
            Some(id) => Self::Behavior(id),
            None => Self::Plugin(origin.plugin.clone()),
        }
    }
}

#[derive(Default)]
pub struct Holds(BTreeMap<HeldInput, BTreeSet<Owner>>);

impl Holds {
    pub fn tap(
        &mut self,
        output: &mut impl Output,
        input: HeldInput,
        origin: &Origin,
    ) -> Result<()> {
        // A tap while physically held cannot emit an up event for another owner.
        if self.0.contains_key(&input) {
            return input.send(output, true);
        }
        self.set(output, input, true, origin.into())?;
        self.set(output, input, false, origin.into())
    }

    pub fn set_for(
        &mut self,
        output: &mut impl Output,
        input: HeldInput,
        down: bool,
        origin: &Origin,
    ) -> Result<()> {
        self.set(output, input, down, origin.into())
    }

    fn set(
        &mut self,
        output: &mut impl Output,
        input: HeldInput,
        down: bool,
        owner: Owner,
    ) -> Result<()> {
        if down {
            if !self.0.contains_key(&input) {
                input.send(output, true)?;
            }
            self.0.entry(input).or_default().insert(owner);
        } else if let Some(owners) = self.0.get_mut(&input) {
            if owners.len() == 1 && owners.contains(&owner) {
                input.send(output, false)?;
                self.0.remove(&input);
            } else {
                owners.remove(&owner);
            }
        }
        Ok(())
    }

    pub fn release_behavior(&mut self, output: &mut impl Output, id: u64) {
        let owner = Owner::Behavior(id);
        let inputs: Vec<_> = self
            .0
            .iter()
            .filter_map(|(input, owners)| owners.contains(&owner).then_some(*input))
            .collect();
        for input in inputs {
            if let Err(error) = self.set(output, input, false, owner.clone()) {
                tracing::error!(%error, ?input, "не удалось снять удержание");
            }
        }
    }

    pub fn release_all(&mut self, output: &mut impl Output) {
        self.0.retain(|input, _| match input.send(output, false) {
            Ok(()) => false,
            Err(error) => {
                tracing::error!(%error, ?input, "не удалось снять удержание");
                true
            }
        });
    }
}

pub fn init_logger() {
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();
    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init()
    {
        eprintln!("не удалось запустить журнал — {error}");
    }
}

pub fn log(arguments: &[Yaml], origin: &Origin) -> Result<()> {
    let (level, message, fields) = match arguments {
        [Yaml::String(level), Yaml::String(message)] => (level, message, None),
        [
            Yaml::String(level),
            Yaml::String(message),
            fields @ Yaml::Hash(_),
        ] => (level, message, Some(fields)),
        _ => return Err("logger.log ожидает уровень, сообщение и необязательный словарь".into()),
    };
    macro_rules! write_log {
        ($level:expr) => {{
            if let Some(fields) = fields {
                tracing::event!($level, plugin = %origin.plugin, fields = ?fields, "{message}");
            } else {
                tracing::event!($level, plugin = %origin.plugin, "{message}");
            }
        }};
    }
    match level.as_str() {
        "trace" => write_log!(tracing::Level::TRACE),
        "debug" => write_log!(tracing::Level::DEBUG),
        "info" => write_log!(tracing::Level::INFO),
        "warn" => write_log!(tracing::Level::WARN),
        "error" => write_log!(tracing::Level::ERROR),
        _ => {
            eprintln!("logger — неизвестный уровень {level:?} для сообщения {message:?}");
            return Err(format!("неизвестный уровень журнала {level:?}"));
        }
    }
    Ok(())
}
