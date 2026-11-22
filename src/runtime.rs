use crate::config::{Action, Behavior, Config, Result};
use crate::input::{CapturedTime, Change, Controller, Measurement, Source, Timestamp};
use crate::lua::{self, Context, Event, LOGGER, PRIMITIVES, Plugins};
use crate::output::{self, HeldInput, Holds, Key, MouseButton, Origin, Output};
use std::collections::VecDeque;
use std::time::Instant;
use yaml_rust2::Yaml;

const MAX_DEPTH: usize = 16;
const MAX_STEPS: usize = 256;

#[cfg(test)]
mod tests;

#[derive(Default)]
struct Chain {
    steps: usize,
    time: Option<Timestamp>,
}

impl Chain {
    fn reserve(&self, pending: usize) -> Result<()> {
        if self.steps + pending > MAX_STEPS {
            return Err(format!(
                "превышен лимит цепочки ({MAX_STEPS} действий и событий)"
            ));
        }
        Ok(())
    }

    fn step(&mut self, pending: usize) -> Result<()> {
        self.reserve(pending + 1)?;
        self.steps += 1;
        Ok(())
    }
}

#[derive(Clone)]
struct Activation {
    id: u64,
    source: Source,
    behavior: Behavior,
}

pub struct Runtime<O: Output> {
    config: Config,
    plugins: Plugins,
    names: Vec<String>,
    active_count: usize,
    output: O,
    holds: Holds,
    controller: Controller,
    mode: Option<String>,
    activations: Vec<Activation>,
    next_id: u64,
    events: VecDeque<Event>,
    closing: bool,
    clock: Instant,
    last_tick: u64,
}

impl<O: Output> Runtime<O> {
    pub fn new(config: Config, plugins: Plugins, output: O) -> Self {
        let names = plugins.in_load_order();
        Self {
            config,
            plugins,
            names,
            output,
            active_count: 0,
            holds: Holds::default(),
            controller: Controller::default(),
            mode: None,
            activations: Vec::new(),
            next_id: 1,
            events: VecDeque::new(),
            closing: false,
            clock: Instant::now(),
            last_tick: 0,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let result = self.start_inner();
        if result.is_err() {
            self.leave_mode(self.now());
            self.report_events();
            self.unload();
        }
        result
    }

    fn start_inner(&mut self) -> Result<()> {
        self.active_count = 2;
        for index in 0..self.plugins.lua.len() {
            let event = Event::new("load", self.now());
            let response = self.plugins.lua[index].event(&event, &self.context(event.time))?;
            self.plugins.lua[index].loaded = true;
            self.active_count += 1;
            if let Some(response) = response {
                let actions = self.accept(index, response, None);
                self.perform(actions, &mut Chain::default())?;
            }
        }

        self.change_mode(&self.config.initial_mode.clone(), self.now())?;
        self.drain(&mut Chain::default())?;
        self.events.push_back(Event::new("ready", self.now()));
        self.drain(&mut Chain::default())?;
        self.last_tick = self.now().monotonic_ms;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.events
            .push_back(Event::new("shutdown_started", self.now()));
        self.report_events();
        self.closing = true;
        self.leave_mode(self.now());
        self.holds.release_all(&mut self.output);
        self.report_events();
        self.unload();
    }

    fn unload(&mut self) {
        self.closing = true;
        while let Some(index) = self.plugins.lua.len().checked_sub(1) {
            if !self.plugins.lua[index].loaded {
                self.plugins.lua.pop();
                continue;
            }
            let event = Event::new("unload", self.now());
            match self.plugins.lua[index].event(&event, &self.context(event.time)) {
                Ok(Some(response)) => {
                    let actions = self.accept(index, response, None);
                    if let Err(error) = self.perform(actions, &mut Chain::default()) {
                        tracing::error!(%error, "ошибка выгрузки");
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "ошибка выгрузки"),
            }
            self.plugins.lua.pop();
            self.active_count -= 1;
        }
        self.holds.release_all(&mut self.output);
        self.active_count = 0;
    }

    pub fn measure(&mut self, measurement: Measurement, observed: CapturedTime) {
        self.measure_timestamp(measurement, observed.since(self.clock));
    }

    #[cfg(test)]
    fn measure_at(&mut self, measurement: Measurement, time: u64) {
        let timestamp = Timestamp {
            monotonic_ms: time,
            ..self.now()
        };
        self.measure_timestamp(measurement, timestamp);
    }

    fn measure_timestamp(&mut self, measurement: Measurement, time: Timestamp) {
        match measurement {
            Measurement::Connected => {
                self.controller = Controller {
                    connected: true,
                    ..Controller::default()
                };
                self.events
                    .push_back(Event::new("controller_connected", time));
                self.report_events();
            }
            Measurement::Disconnected => {
                self.controller = Controller::default();
                self.holds.release_all(&mut self.output);
                self.events
                    .push_back(Event::new("controller_disconnected", time));
                self.report_events();
                self.holds.release_all(&mut self.output);
            }
            _ if self.controller.connected => {
                let (mapping, changes) = self.controller.update(measurement, time.monotonic_ms);
                let input_mode = self.mode.clone();
                let mut chain = Chain {
                    time: Some(time),
                    ..Chain::default()
                };
                if let Some(action) = mapping.and_then(|source| {
                    self.mode
                        .as_ref()
                        .and_then(|mode| self.config.modes[mode].mappings.get(&source))
                        .cloned()
                }) {
                    let origin = Origin {
                        plugin: "mappings".into(),
                        activation_id: None,
                    };
                    if let Err(error) = self.perform(vec![(action, origin)], &mut chain) {
                        tracing::error!(%error, "ошибка назначения");
                    }
                    self.report_chain(&mut chain);
                }

                for change in changes {
                    if let Some(event) = self.input_event(change, time, input_mode.as_deref()) {
                        self.events.push_back(event);
                        self.report_chain(&mut chain);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        self.tick_timestamp(self.now());
    }

    #[cfg(test)]
    fn tick_at(&mut self, time: u64) {
        self.tick_timestamp(Timestamp {
            monotonic_ms: time,
            ..self.now()
        });
    }

    fn tick_timestamp(&mut self, time: Timestamp) {
        let dt_ms = time.monotonic_ms.saturating_sub(self.last_tick);
        self.last_tick = time.monotonic_ms;
        self.events
            .push_back(Event::new("tick", time).field("dt_ms", dt_ms));
        self.report_events();
    }

    fn input_event(
        &self,
        change: Change,
        time: Timestamp,
        input_mode: Option<&str>,
    ) -> Option<Event> {
        let source = change.source();
        let activation = self
            .activations
            .iter()
            .find(|activation| activation.source == source)?;
        let mut event = match change {
            Change::Stick(_, x, y) => Event::new("stick_moved", time).field("x", x).field("y", y),
            Change::Trigger(_, value) => Event::new("trigger_moved", time).field("value", value),
            Change::Button(_, true) => Event::new("button_pressed", time),
            Change::Button(_, false) => Event::new("button_unpressed", time),
        };
        if let Some(input_mode) = input_mode {
            event = event.field("input_mode", input_mode);
        }
        Some(event.for_behavior(
            self.mode.as_deref()?,
            source,
            activation.behavior.clone(),
            activation.id,
        ))
    }

    fn change_mode(&mut self, new_mode: &str, time: Timestamp) -> Result<()> {
        if self.closing {
            return Err("смена режима во время выгрузки недоступна".into());
        }
        let mode = self
            .config
            .modes
            .get(new_mode)
            .ok_or_else(|| format!("неизвестный режим {new_mode:?}"))?;
        if self.mode.as_deref() == Some(new_mode) {
            return Ok(());
        }
        let behaviors = mode.behaviors.clone();
        if let Some(previous_mode) = self.leave_mode(time) {
            self.events.push_back(
                Event::new("mode_changed", time)
                    .field("previous_mode", previous_mode.as_str())
                    .field("new_mode", new_mode),
            );
        }
        self.mode = Some(new_mode.into());

        for (source, behavior) in behaviors {
            let id = self.next_id;
            self.next_id += 1;
            self.events
                .push_back(Event::new(lifecycle(source, true), time).for_behavior(
                    new_mode,
                    source,
                    behavior.clone(),
                    id,
                ));
            self.activations.push(Activation {
                id,
                source,
                behavior,
            });
        }
        Ok(())
    }

    fn leave_mode(&mut self, time: Timestamp) -> Option<String> {
        let mode = self.mode.take()?;
        for activation in self.activations.drain(..) {
            self.holds.release_behavior(&mut self.output, activation.id);
            self.events.push_back(
                Event::new(lifecycle(activation.source, false), time).for_behavior(
                    &mode,
                    activation.source,
                    activation.behavior,
                    activation.id,
                ),
            );
        }
        Some(mode)
    }

    fn report_events(&mut self) {
        self.report_chain(&mut Chain::default());
    }

    fn report_chain(&mut self, chain: &mut Chain) {
        if let Err(error) = self.drain(chain) {
            tracing::error!(%error, "ошибка обработки событий");
        }
    }

    fn drain(&mut self, chain: &mut Chain) -> Result<()> {
        let mut first_error = None;
        while let Some(event) = self.events.pop_front() {
            if let Err(error) = chain.step(self.events.len()) {
                self.events.clear();
                return Err(error);
            }
            if let Err(error) = self.dispatch(&event, chain) {
                if first_error.is_some() {
                    tracing::error!(%error, "ошибка обработки события");
                } else {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn dispatch(&mut self, event: &Event, chain: &mut Chain) -> Result<()> {
        let mut actions = Vec::new();
        let mut first_error = None;
        for index in 0..self.plugins.lua.len() {
            let plugin = &self.plugins.lua[index];
            if !plugin.loaded
                || event
                    .target
                    .as_ref()
                    .is_some_and(|target| target.behavior.plugin != plugin.name)
            {
                continue;
            }
            match plugin.event(event, &self.context(event.time)) {
                Ok(Some(response)) => {
                    actions.extend(self.accept(
                        index,
                        response,
                        event.target.as_ref().map(|target| target.activation_id),
                    ));
                }
                Ok(None) => {}
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(error) => tracing::error!(%error, "ошибка обработчика"),
            }
        }
        chain.time = Some(event.time);
        let result = self.perform(actions, chain);
        match (first_error, result) {
            (Some(error), Err(action_error)) => {
                tracing::error!(%action_error, "ошибка действия");
                Err(error)
            }
            (Some(error), Ok(())) => Err(error),
            (None, result) => result,
        }
    }

    fn context(&self, time: Timestamp) -> Context<'_> {
        Context {
            active_mode: self.mode.as_deref(),
            controller: &self.controller,
            config: &self.config.raw,
            names: &self.names,
            time,
        }
    }

    fn accept(
        &mut self,
        index: usize,
        response: lua::Response,
        activation_id: Option<u64>,
    ) -> Vec<(Action, Origin)> {
        let plugin = &mut self.plugins.lua[index];
        if let Some(state) = response.state {
            plugin.state = Some(state);
        }
        let origin = Origin {
            plugin: plugin.name.clone(),
            activation_id,
        };
        response
            .actions
            .into_iter()
            .map(|action| (action, origin.clone()))
            .collect()
    }

    fn perform(&mut self, actions: Vec<(Action, Origin)>, chain: &mut Chain) -> Result<()> {
        self.perform_nested(actions, chain, 1)
    }

    fn perform_nested(
        &mut self,
        actions: Vec<(Action, Origin)>,
        chain: &mut Chain,
        depth: usize,
    ) -> Result<()> {
        if !actions.is_empty() && depth > MAX_DEPTH {
            return Err(format!("превышена вложенность действий ({MAX_DEPTH})"));
        }
        for (action, origin) in actions {
            chain.step(self.events.len())?;
            let nested = self
                .perform_one(&action, &origin, chain)
                .map_err(|error| format!("{}.{} — {error}", action.plugin, action.kind))?;
            self.perform_nested(nested, chain, depth + 1)?;
        }
        Ok(())
    }

    fn perform_one(
        &mut self,
        action: &Action,
        origin: &Origin,
        chain: &Chain,
    ) -> Result<Vec<(Action, Origin)>> {
        if !self.names[..self.active_count].contains(&action.plugin) {
            return Err(format!("плагин {:?} не загружен", action.plugin));
        }
        match action.plugin.as_str() {
            LOGGER => {
                if action.kind != "log" {
                    return Err("неизвестный тип действия".into());
                }
                output::log(&action.arguments, origin)?;
            }
            PRIMITIVES => self.primitive(&action.kind, &action.arguments, origin, chain)?,
            name => {
                let index = self
                    .plugins
                    .lua
                    .iter()
                    .position(|plugin| plugin.name == name)
                    .unwrap();
                let response =
                    self.plugins.lua[index].action(action, &self.context(self.now()), origin)?;
                return Ok(self.accept(index, response, origin.activation_id));
            }
        }
        Ok(Vec::new())
    }

    fn primitive(
        &mut self,
        kind: &str,
        arguments: &[Yaml],
        origin: &Origin,
        chain: &Chain,
    ) -> Result<()> {
        match (kind, arguments) {
            ("change_mode", [Yaml::String(mode)]) if self.mode.is_some() => {
                if self.mode.as_deref() != Some(mode)
                    && let Some(next) = self.config.modes.get(mode)
                {
                    // Оставляем место для всех событий перехода до изменения режима.
                    chain.reserve(
                        self.events.len() + self.activations.len() + 1 + next.behaviors.len(),
                    )?;
                }
                self.change_mode(mode, chain.time.unwrap_or_else(|| self.now()))
            }
            ("change_mode", [Yaml::String(_)]) => {
                Err("начальный режим ещё не активирован или уже закрыт".into())
            }
            ("key", [key]) => {
                self.holds
                    .tap(&mut self.output, HeldInput::Key(Key::parse(key)?), origin)
            }
            ("key_down" | "key_up", [key]) => {
                self.set_hold(HeldInput::Key(Key::parse(key)?), kind == "key_down", origin)
            }
            ("mouse_key", [button]) => self.holds.tap(
                &mut self.output,
                HeldInput::Mouse(MouseButton::parse(button)?),
                origin,
            ),
            ("mouse_key", [button, Yaml::String(op)]) => {
                let input = HeldInput::Mouse(MouseButton::parse(button)?);
                match op.as_str() {
                    "click" => self.holds.tap(&mut self.output, input, origin),
                    "down" | "up" => self.set_hold(input, op == "down", origin),
                    _ => Err("для mouse_key ожидается click, down или up".into()),
                }
            }
            ("move_pointer", [dx, dy]) => self
                .output
                .move_pointer(finite_number(dx)?, finite_number(dy)?),
            ("scroll", [dx, dy]) => self.output.scroll(integer(dx)?, integer(dy)?),
            ("type_text", [Yaml::String(text)]) => self.output.type_text(text),
            (
                "change_mode" | "key" | "key_down" | "key_up" | "mouse_key" | "move_pointer"
                | "scroll" | "type_text",
                _,
            ) => Err("некорректные аргументы".into()),
            _ => Err("неизвестный тип действия".into()),
        }
    }

    fn set_hold(&mut self, input: HeldInput, down: bool, origin: &Origin) -> Result<()> {
        if down
            && origin.activation_id.is_some_and(|id| {
                !self
                    .activations
                    .iter()
                    .any(|activation| activation.id == id)
            })
        {
            return Err("поведение больше не активно".into());
        }
        self.holds.set_for(&mut self.output, input, down, origin)
    }

    fn now(&self) -> Timestamp {
        CapturedTime::now().since(self.clock)
    }
}

fn lifecycle(source: Source, entering: bool) -> &'static str {
    match (source, entering) {
        (Source::Stick(_), true) => "stick_behavior_entered",
        (Source::Stick(_), false) => "stick_behavior_exited",
        (Source::Trigger(_), true) => "trigger_behavior_entered",
        (Source::Trigger(_), false) => "trigger_behavior_exited",
        (Source::Button(_), true) => "button_behavior_entered",
        (Source::Button(_), false) => "button_behavior_exited",
    }
}

fn finite_number(value: &Yaml) -> Result<f64> {
    let number = value.as_f64().or_else(|| value.as_i64().map(|n| n as f64));
    number
        .filter(|n| n.is_finite())
        .ok_or_else(|| "ожидается конечное число".into())
}

fn integer(value: &Yaml) -> Result<i32> {
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| "ожидается целое число".into())
}
