use crate::config::{Action, Behavior, Result};
use crate::input::{Button, Controller, Side, Source, Timestamp};
use crate::output::Origin;
use mlua::{Function, Lua, Table, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::c_void;
use std::path::Path;
use yaml_rust2::{Yaml, yaml::Hash};

pub const LOGGER: &str = "logger";
pub const PRIMITIVES: &str = "builtin_primitives";

const BUILTINS: &[(&str, &str)] = &[
    (
        "chord_text",
        include_str!("../example/plugins/chord_text.lua"),
    ),
    ("mouse", include_str!("../example/plugins/mouse.lua")),
];

pub struct Event {
    pub kind: &'static str,
    pub time: Timestamp,
    pub fields: Hash,
    pub target: Option<Target>,
}

pub struct Target {
    pub behavior: Behavior,
    pub activation_id: u64,
}

impl Event {
    pub fn new(kind: &'static str, time: Timestamp) -> Self {
        Self {
            kind,
            time,
            fields: Hash::new(),
            target: None,
        }
    }

    pub fn field(mut self, key: &str, value: impl IntoYaml) -> Self {
        self.fields.insert(yaml_str(key), value.into_yaml());
        self
    }

    pub fn for_behavior(mut self, mode: &str, source: Source, behavior: Behavior, id: u64) -> Self {
        let physical = source.physical();
        let mut source_data = Hash::new();
        source_data.insert(yaml_str("kind"), yaml_str(physical.kind()));
        match physical {
            Source::Stick(side) | Source::Trigger(side) => {
                source_data.insert(yaml_str("side"), yaml_str(side.name()));
            }
            Source::Button(button) => {
                source_data.insert(yaml_str("id"), yaml_str(button.name()));
            }
        }
        self = self
            .field("source", Yaml::Hash(source_data))
            .field("mode", mode);
        if let Source::Button(button) = source {
            self = self.field("button", button.name());
        }
        self.target = Some(Target {
            behavior,
            activation_id: id,
        });
        self
    }
}

pub struct Context<'a> {
    pub active_mode: Option<&'a str>,
    pub controller: &'a Controller,
    pub config: &'a Yaml,
    pub names: &'a [String],
    pub time: Timestamp,
}

pub struct Response {
    pub actions: Vec<Action>,
    pub state: Option<Yaml>,
}

#[derive(Default)]
pub struct Plugins {
    pub lua: Vec<LuaPlugin>,
}

pub struct PluginSources(BTreeMap<String, (String, String)>);

impl PluginSources {
    pub fn discover(directory: &Path) -> Result<Self> {
        let mut sources: BTreeMap<String, (String, String)> = BUILTINS
            .iter()
            .map(|(name, body)| {
                (
                    name.to_string(),
                    (format!("встроенный {name}.lua"), body.to_string()),
                )
            })
            .collect();
        match std::fs::read_dir(directory) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry
                        .map_err(|err| format!("{} — {err}", directory.display()))?
                        .path();
                    if path.extension().and_then(|ext| ext.to_str()) != Some("lua") || path.is_dir()
                    {
                        continue;
                    }
                    let name = path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| format!("некорректное имя плагина {}", path.display()))?;
                    if matches!(name, LOGGER | PRIMITIVES) {
                        return Err(format!("имя плагина {name:?} зарезервировано"));
                    }
                    let body = std::fs::read_to_string(&path)
                        .map_err(|err| format!("{} — {err}", path.display()))?;
                    sources.insert(name.into(), (path.display().to_string(), body));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("{} — {err}", directory.display())),
        }
        Ok(Self(sources))
    }

    pub fn names(&self) -> BTreeSet<String> {
        builtin_names().chain(self.0.keys().cloned()).collect()
    }

    pub fn instantiate(self) -> Result<Plugins> {
        let lua = self
            .0
            .into_iter()
            .map(|(name, (path, body))| {
                LuaPlugin::new(name, &path, &body).map_err(|err| format!("{path} — {err}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Plugins { lua })
    }
}

impl Plugins {
    pub fn in_load_order(&self) -> Vec<String> {
        builtin_names()
            .chain(self.lua.iter().map(|plugin| plugin.name.clone()))
            .collect()
    }
}

fn builtin_names() -> impl Iterator<Item = String> {
    [LOGGER, PRIMITIVES].into_iter().map(str::to_owned)
}

pub struct LuaPlugin {
    pub name: String,
    pub loaded: bool,
    pub state: Option<Yaml>,
    lua: Lua,
    handlers: Option<Table>,
    on_action: Option<Function>,
}

impl LuaPlugin {
    pub fn new(name: String, path: &str, source: &str) -> mlua::Result<Self> {
        let lua = Lua::new();
        lua.globals().set("yaml_null", Value::NULL)?;
        let strings: Table = lua.globals().get("string")?;
        strings.set(
            "unicode_upper",
            lua.create_function(|_, text: String| Ok(text.to_uppercase()))?,
        )?;
        strings.set(
            "is_cased",
            lua.create_function(|_, text: String| {
                Ok(text.chars().any(|c| c.is_uppercase() || c.is_lowercase()))
            })?,
        )?;

        let module: Table = lua.load(source).set_name(path).eval()?;
        let handlers: Option<Table> = module.get("event_handlers")?;
        if let Some(handlers) = &handlers {
            for entry in handlers.pairs::<String, Function>() {
                entry?;
            }
        }
        let on_action = module.get("on_action")?;
        Ok(Self {
            name,
            loaded: false,
            state: None,
            lua,
            handlers,
            on_action,
        })
    }

    pub fn event(&self, event: &Event, context: &Context<'_>) -> Result<Option<Response>> {
        self.event_inner(event, context)
            .map_err(|err| format!("{}.{} — {err}", self.name, event.kind))
    }

    fn event_inner(&self, event: &Event, context: &Context<'_>) -> mlua::Result<Option<Response>> {
        let Some(handlers) = &self.handlers else {
            return Ok(None);
        };
        let Some(handler) = handlers.get::<Option<Function>>(event.kind)? else {
            return Ok(None);
        };

        let data = self.lua.create_table()?;
        data.set("type", event.kind)?;
        data.set("time_ms", event.time.monotonic_ms)?;
        data.set("unix_ms", event.time.unix_ms)?;
        for (key, value) in &event.fields {
            data.set(to_lua(&self.lua, key, 0)?, to_lua(&self.lua, value, 0)?)?;
        }
        if let Some(target) = &event.target {
            let behavior = self.lua.create_table()?;
            behavior.set("plugin", target.behavior.plugin.as_str())?;
            behavior.set("type", target.behavior.kind.as_str())?;
            behavior.set(
                "arguments",
                sequence(&self.lua, &target.behavior.arguments, 0)?,
            )?;
            if let Some(settings) = &target.behavior.settings {
                behavior.set("settings", to_lua(&self.lua, settings, 0)?)?;
            }
            data.set("behavior", behavior)?;
        }
        let returned = handler.call::<Value>((data, self.context(context, None)?))?;
        response(returned).map(Some)
    }

    pub fn action(
        &self,
        action: &Action,
        context: &Context<'_>,
        origin: &Origin,
    ) -> Result<Response> {
        self.action_inner(action, context, origin)
            .map_err(|err| format!("{}.{} — {err}", self.name, action.kind))
    }

    fn action_inner(
        &self,
        action: &Action,
        context: &Context<'_>,
        origin: &Origin,
    ) -> mlua::Result<Response> {
        let handler = self
            .on_action
            .as_ref()
            .ok_or_else(|| lua_error("плагин не объявил on_action"))?;
        let data = self.lua.create_table()?;
        data.set("plugin", action.plugin.as_str())?;
        data.set("type", action.kind.as_str())?;
        data.set("arguments", sequence(&self.lua, &action.arguments, 0)?)?;
        response(handler.call((data, self.context(context, Some(origin))?))?)
    }

    fn context(&self, context: &Context<'_>, origin: Option<&Origin>) -> mlua::Result<Table> {
        let lua = &self.lua;
        let data = lua.create_table()?;
        data.set("active_mode", context.active_mode)?;
        data.set("controller", controller_table(lua, context.controller)?)?;
        let time = lua.create_table()?;
        time.set("monotonic_ms", context.time.monotonic_ms)?;
        time.set("unix_ms", context.time.unix_ms)?;
        data.set("time", time)?;
        data.set("config", to_lua(lua, context.config, 0)?)?;
        data.set(
            "plugins",
            lua.create_sequence_from(context.names.iter().map(String::as_str))?,
        )?;
        if let Some(state) = &self.state {
            data.set("state", to_lua(lua, state, 0)?)?;
        }
        if let Some(origin) = origin {
            let value = lua.create_table()?;
            value.set("plugin", origin.plugin.as_str())?;
            value.set("activation_id", origin.activation_id)?;
            data.set("origin", value)?;
        }
        Ok(data)
    }
}

fn controller_table(lua: &Lua, controller: &Controller) -> mlua::Result<Table> {
    let data = lua.create_table()?;
    let sticks = lua.create_table()?;
    let triggers = lua.create_table()?;
    let buttons = lua.create_table()?;
    for side in Side::ALL {
        let point = lua.create_table()?;
        let (x, y) = controller.sticks[side as usize];
        point.set("x", x)?;
        point.set("y", y)?;
        sticks.set(side.name(), point)?;
        triggers.set(side.name(), controller.triggers[side as usize])?;
    }
    for button in Button::ALL {
        let value = lua.create_table()?;
        let pressed_at = controller.pressed.get(button).copied();
        value.set("pressed", pressed_at.is_some())?;
        value.set("pressed_at_ms", pressed_at)?;
        buttons.set(button.name(), value)?;
    }
    data.set("connected", controller.connected)?;
    data.set("sticks", sticks)?;
    data.set("triggers", triggers)?;
    data.set("buttons", buttons)?;
    Ok(data)
}

const DATA_DEPTH: usize = 64;

fn to_lua(lua: &Lua, value: &Yaml, depth: usize) -> mlua::Result<Value> {
    check_depth(depth)?;
    Ok(match value {
        Yaml::Boolean(value) => Value::Boolean(*value),
        Yaml::Integer(value) => Value::Integer(*value),
        Yaml::Real(_) => Value::Number(
            value
                .as_f64()
                .ok_or_else(|| lua_error("некорректное число YAML"))?,
        ),
        Yaml::String(text) => Value::String(lua.create_string(text)?),
        Yaml::Null => Value::NULL,
        Yaml::Array(items) => Value::Table(sequence(lua, items, depth + 1)?),
        Yaml::Hash(items) => Value::Table(ordered_table(lua, items, depth + 1)?),
        _ => return Err(lua_error("неподдерживаемое значение YAML")),
    })
}

fn sequence(lua: &Lua, items: &[Yaml], depth: usize) -> mlua::Result<Table> {
    let table = lua.create_table_with_capacity(items.len(), 0)?;
    for (i, value) in items.iter().enumerate() {
        table.raw_set(i + 1, to_lua(lua, value, depth)?)?;
    }
    Ok(table)
}

fn ordered_table(lua: &Lua, items: &Hash, depth: usize) -> mlua::Result<Table> {
    let table = lua.create_table_with_capacity(0, items.len())?;
    let mut keys = Vec::with_capacity(items.len());
    for (key, value) in items {
        let key = to_lua(lua, key, depth)?;
        table.raw_set(key.clone(), to_lua(lua, value, depth)?)?;
        keys.push(key);
    }
    let meta = lua.create_table()?;
    meta.set("__yaml_map", true)?;
    meta.set(
        "__pairs",
        lua.create_function(move |lua, table: Table| {
            let mut keys = keys.clone().into_iter();
            lua.create_function_mut(move |_, ()| {
                if let Some(key) = keys.next() {
                    let value = table.raw_get::<Value>(key.clone())?;
                    Ok((key, value))
                } else {
                    Ok((Value::Nil, Value::Nil))
                }
            })
        })?,
    )?;
    table.set_metatable(Some(meta))?;
    Ok(table)
}

fn from_lua(value: Value, depth: usize) -> mlua::Result<Yaml> {
    read_value(value, depth, &mut HashSet::new())
}

fn read_value(
    value: Value,
    depth: usize,
    stack: &mut HashSet<*const c_void>,
) -> mlua::Result<Yaml> {
    check_depth(depth)?;
    if value == Value::NULL {
        return Ok(Yaml::Null);
    }
    Ok(match value {
        Value::Nil => Yaml::Null,
        Value::Boolean(value) => Yaml::Boolean(value),
        Value::Integer(value) => Yaml::Integer(value),
        Value::Number(value) if value.is_finite() => Yaml::Real(value.to_string()),
        Value::String(value) => Yaml::String(value.to_str()?.to_string()),
        Value::Table(table) => {
            let id = table.to_pointer();
            if !stack.insert(id) {
                return Err(lua_error("Lua-таблица ссылается на себя"));
            }
            let entries = table
                .pairs::<Value, Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            let is_map = table
                .metatable()
                .and_then(|meta| meta.raw_get::<bool>("__yaml_map").ok())
                .unwrap_or(false);
            let converted = if !is_map && is_sequence(&entries) {
                let items = (1..=entries.len())
                    .map(|index| read_value(table.raw_get(index)?, depth + 1, stack))
                    .collect::<mlua::Result<Vec<_>>>()?;
                Yaml::Array(items)
            } else {
                let mut map = Hash::new();
                for (key, value) in entries {
                    map.insert(
                        read_value(key, depth + 1, stack)?,
                        read_value(value, depth + 1, stack)?,
                    );
                }
                Yaml::Hash(map)
            };
            stack.remove(&id);
            converted
        }
        _ => {
            return Err(lua_error(
                "обработчик вернул значение, несовместимое с YAML",
            ));
        }
    })
}

fn is_sequence(entries: &[(Value, Value)]) -> bool {
    entries.iter().all(|(key, _)| {
        matches!(key, Value::Integer(n) if usize::try_from(*n).is_ok_and(|n| n > 0 && n <= entries.len()))
    })
}

fn sequence_values(table: &Table, len: usize, depth: usize) -> mlua::Result<Vec<Yaml>> {
    (1..=len)
        .map(|index| from_lua(table.raw_get(index)?, depth))
        .collect()
}

pub fn response(value: Value) -> mlua::Result<Response> {
    let Value::Table(table) = value else {
        return if value == Value::Nil {
            Ok(Response {
                actions: Vec::new(),
                state: None,
            })
        } else {
            Err(lua_error(
                "ожидается список действий или таблица с actions и state",
            ))
        };
    };
    let actions_field: Value = table.raw_get("actions")?;
    let state: Value = table.raw_get("state")?;
    let (actions, state) = if actions_field != Value::Nil || state != Value::Nil {
        let list = match actions_field {
            Value::Nil => None,
            Value::Table(list) => Some(list),
            _ => return Err(lua_error("поле actions должно быть списком")),
        };
        let state = (state != Value::Nil)
            .then(|| from_lua(state, 0))
            .transpose()?;
        (list, state)
    } else {
        (Some(table), None)
    };
    let mut result = Vec::new();
    if let Some(actions) = actions {
        let entries = actions
            .pairs::<Value, Value>()
            .collect::<mlua::Result<Vec<_>>>()?;
        if !is_sequence(&entries) {
            return Err(lua_error("ожидается список действий без пропусков"));
        }
        for index in 1..=entries.len() {
            let action: Table = actions.raw_get(index)?;
            let arguments: Table = action.raw_get("arguments")?;
            let entries = arguments
                .pairs::<Value, Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            if !is_sequence(&entries) {
                return Err(lua_error(
                    "поле arguments должно быть списком без пропусков",
                ));
            }
            let plugin: String = action.raw_get("plugin")?;
            let kind: String = action.raw_get("type")?;
            if plugin.is_empty() || kind.is_empty() {
                return Err(lua_error(
                    "action.plugin и action.type должны быть непустыми",
                ));
            }
            result.push(Action {
                plugin,
                kind,
                arguments: sequence_values(&arguments, entries.len(), 0)?,
            });
        }
    }
    Ok(Response {
        actions: result,
        state,
    })
}

fn check_depth(depth: usize) -> mlua::Result<()> {
    if depth <= DATA_DEPTH {
        Ok(())
    } else {
        Err(lua_error("слишком глубокая вложенность данных"))
    }
}

fn lua_error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

pub trait IntoYaml {
    fn into_yaml(self) -> Yaml;
}

impl IntoYaml for Yaml {
    fn into_yaml(self) -> Yaml {
        self
    }
}

impl IntoYaml for &str {
    fn into_yaml(self) -> Yaml {
        yaml_str(self)
    }
}

impl IntoYaml for u64 {
    fn into_yaml(self) -> Yaml {
        Yaml::Integer(self as i64)
    }
}

impl IntoYaml for f32 {
    fn into_yaml(self) -> Yaml {
        Yaml::Real(f64::from(self).to_string())
    }
}

fn yaml_str(text: &str) -> Yaml {
    Yaml::String(text.into())
}
