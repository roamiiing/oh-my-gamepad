use crate::input::{Button, Side, Source};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use yaml_rust2::{Yaml, YamlLoader, yaml::Hash};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub struct Action {
    pub plugin: String,
    pub kind: String,
    pub arguments: Vec<Yaml>,
}

impl Action {
    pub fn parse(value: &Yaml) -> Result<Self> {
        mapping(value, "action")?;
        let arguments = value["arguments"]
            .as_vec()
            .ok_or("action.arguments должен быть списком")?;
        Ok(Self {
            plugin: string(&value["plugin"], "action.plugin")?.into(),
            kind: string(&value["type"], "action.type")?.into(),
            arguments: arguments.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Behavior {
    pub plugin: String,
    pub kind: String,
    pub arguments: Vec<Yaml>,
    pub settings: Option<Yaml>,
}

#[derive(Default)]
pub struct Mode {
    pub mappings: BTreeMap<Source, Action>,
    pub behaviors: Vec<(Source, Behavior)>,
}

pub struct Config {
    pub raw: Yaml,
    pub initial_mode: String,
    pub modes: BTreeMap<String, Mode>,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let parsed = std::fs::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|body| Self::parse(&body));
        parsed.map_err(|err| format!("{} — {err}", path.display()))
    }

    pub fn parse(body: &str) -> Result<Self> {
        let mut docs = YamlLoader::load_from_str(body).map_err(|err| err.to_string())?;
        if docs.len() != 1 {
            return Err("ожидается один документ YAML".into());
        }
        let raw = docs.remove(0);
        mapping(&raw, "конфиг")?;
        let initial_mode = string(&raw["initial_mode"], "initial_mode")?.to_owned();
        let mut modes = BTreeMap::new();
        for (name, value) in mapping(&raw["modes"], "modes")? {
            let name = string(name, "имя режима")?;
            let mode = parse_mode(value).map_err(|err| format!("режим {name} — {err}"))?;
            modes.insert(name.to_owned(), mode);
        }
        if !modes.contains_key(&initial_mode) {
            return Err(format!("режим initial_mode {initial_mode:?} не найден"));
        }

        Ok(Self {
            raw,
            initial_mode,
            modes,
        })
    }

    pub fn validate_plugins(&self, names: &BTreeSet<String>) -> Result<()> {
        for (mode_name, mode) in &self.modes {
            for plugin in mode
                .mappings
                .values()
                .map(|action| &action.plugin)
                .chain(mode.behaviors.iter().map(|(_, behavior)| &behavior.plugin))
            {
                if !names.contains(plugin) {
                    return Err(format!(
                        "режим {mode_name:?} ссылается на неизвестный плагин {plugin:?}"
                    ));
                }
            }
            for action in mode.mappings.values() {
                if action.plugin == "builtin_primitives" && action.kind == "change_mode" {
                    let name = action.arguments.first().and_then(Yaml::as_str);
                    if action.arguments.len() != 1
                        || !name.is_some_and(|name| self.modes.contains_key(name))
                    {
                        return Err(format!(
                            "режим {mode_name:?} — change_mode требует имя существующего режима"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn parse_mode(value: &Yaml) -> Result<Mode> {
    mapping(value, "режим")?;
    let mut mode = Mode::default();
    if !value["mappings"].is_badvalue() {
        for item in value["mappings"]
            .as_vec()
            .ok_or("mappings должен быть списком")?
        {
            let input = string(&item["input"], "mappings.input")?;
            let source = match input {
                "ls" => Source::Stick(Side::Left),
                "rs" => Source::Stick(Side::Right),
                _ => Source::Button(
                    Button::parse(input).ok_or_else(|| format!("неизвестный вход {input:?}"))?,
                ),
            };
            if mode
                .mappings
                .insert(source, Action::parse(&item["action"])?)
                .is_some()
            {
                return Err(format!("повторное назначение для {input:?}"));
            }
        }
    }

    for (section, source_for_side) in [
        ("sticks", Source::Stick as fn(Side) -> Source),
        ("triggers", Source::Trigger as fn(Side) -> Source),
    ] {
        if !value[section].is_badvalue() {
            for (side, entry) in mapping(&value[section], section)? {
                let side = match side.as_str() {
                    Some("left") => Side::Left,
                    Some("right") => Side::Right,
                    _ => return Err(format!("в {section} ожидается left или right")),
                };
                add_behavior(&mut mode, source_for_side(side), entry)?;
            }
        }
    }

    if !value["buttons"].is_badvalue() {
        for (name, entry) in mapping(&value["buttons"], "buttons")? {
            let name = string(name, "имя кнопки")?;
            let button =
                Button::parse(name).ok_or_else(|| format!("неизвестная кнопка {name:?}"))?;
            add_behavior(&mut mode, Source::Button(button), entry)?;
        }
    }
    mode.behaviors.sort_by_key(|(source, _)| match source {
        Source::Stick(side) => (0, side.name()),
        Source::Trigger(side) => (1, side.name()),
        Source::Button(button) => (2, button.name()),
    });
    Ok(mode)
}

fn add_behavior(mode: &mut Mode, source: Source, entry: &Yaml) -> Result<()> {
    mapping(entry, "настройки входа")?;
    let value = &entry["behavior"];
    if !value.is_badvalue() {
        mapping(value, "behavior")?;
        let arguments = match &value["arguments"] {
            Yaml::BadValue => Vec::new(),
            Yaml::Array(items) => items.clone(),
            _ => return Err("behavior.arguments должен быть списком".into()),
        };
        mode.behaviors.push((
            source,
            Behavior {
                plugin: string(&value["plugin"], "behavior.plugin")?.into(),
                kind: string(&value["type"], "behavior.type")?.into(),
                arguments,
                settings: (!value["settings"].is_badvalue()).then(|| value["settings"].clone()),
            },
        ));
    }
    Ok(())
}

fn mapping<'a>(value: &'a Yaml, name: &str) -> Result<&'a Hash> {
    value
        .as_hash()
        .ok_or_else(|| format!("{name} должен быть словарём"))
}

fn string<'a>(value: &'a Yaml, name: &str) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{name} должен быть непустой строкой"))
}
