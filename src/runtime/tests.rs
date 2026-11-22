use super::*;
use crate::input::{Button, Side};
use crate::lua::LuaPlugin;
use std::path::Path;

#[derive(Default)]
struct TestOutput(Vec<String>);

impl Output for TestOutput {
    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        self.0.push(format!("key {} {down}", key.0));
        Ok(())
    }

    fn mouse(&mut self, button: MouseButton, down: bool) -> Result<()> {
        self.0.push(format!("mouse {button:?} {down}"));
        Ok(())
    }

    fn move_pointer(&mut self, dx: f64, dy: f64) -> Result<()> {
        self.0.push(format!("pointer {dx:.1} {dy:.1}"));
        Ok(())
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.0.push(format!("scroll {dx} {dy}"));
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        self.0.push(text.to_owned());
        Ok(())
    }
}

fn plugin(name: &str, code: &str) -> LuaPlugin {
    LuaPlugin::new(name.into(), name, code).unwrap()
}

fn setup(config: &str, plugins: Plugins) -> Runtime<TestOutput> {
    let config = Config::parse(config).unwrap();
    config
        .validate_plugins(&plugins.in_load_order().into_iter().collect())
        .unwrap();
    Runtime::new(config, plugins, TestOutput::default())
}

fn text_action(text: &str) -> Action {
    Action {
        plugin: PRIMITIVES.into(),
        kind: "type_text".into(),
        arguments: vec![Yaml::String(text.into())],
    }
}

fn action(runtime: &mut Runtime<TestOutput>, kind: &str, arguments: Vec<Yaml>) {
    runtime
        .perform(
            vec![(
                Action {
                    plugin: "chord_text".into(),
                    kind: kind.into(),
                    arguments,
                },
                Origin {
                    plugin: "test".into(),
                    activation_id: None,
                },
            )],
            &mut Chain::default(),
        )
        .unwrap();
}

#[test]
fn example_layout_drives_mouse_and_confirms_unicode_text() {
    let plugins = lua::PluginSources::discover(Path::new("example/plugins"))
        .unwrap()
        .instantiate()
        .unwrap();
    let mut runtime = setup(include_str!("../../example/config.yaml"), plugins);
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 1);
    runtime.measure_at(Measurement::Button(Button::Lb, true), 2);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 3);
    assert_eq!(runtime.mode.as_deref(), Some("mouse"));
    assert!(runtime.output.0.is_empty());

    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 4);
    runtime.tick_at(14);
    assert_eq!(runtime.output.0.len(), 1);
    assert!(runtime.output.0[0].starts_with("pointer 0.0 -"));

    runtime.measure_at(Measurement::Button(Button::Lb, true), 15);
    assert_eq!(&runtime.output.0[1..], ["mouse Left true"]);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 16);
    assert_eq!(
        &runtime.output.0[1..],
        ["mouse Left true", "mouse Left false"]
    );

    runtime.measure_at(Measurement::Trigger(Side::Right, 0.31), 17);
    runtime.measure_at(Measurement::Trigger(Side::Right, 0.0), 18);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 19);
    let upper_right_x = 3.0_f32.sqrt() / 2.0;
    runtime.measure_at(Measurement::Stick(Side::Left, upper_right_x, 0.5), 20);
    assert_eq!(runtime.mode.as_deref(), Some("text"));
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 21);
    runtime.tick_at(80);
    assert_eq!(runtime.output.0.len(), 3);
    runtime.tick_at(81);
    runtime.tick_at(90);
    action(&mut runtime, "repeat_choice", vec![]);
    assert_eq!(&runtime.output.0[3..], ["t", "t"]);

    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, 0.5), 91);
    runtime.tick_at(151);
    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, -0.5), 152);
    runtime.tick_at(212);
    assert_eq!(&runtime.output.0[3..], ["t", "t", "h", "e"]);

    runtime.measure_at(Measurement::Button(Button::Lb, true), 213);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 214);
    runtime.tick_at(274);
    assert_eq!(runtime.output.0.len(), 7);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 275);
    runtime.measure_at(Measurement::Button(Button::Rb, true), 276);
    runtime.measure_at(Measurement::Button(Button::Rb, false), 277);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, -1.0), 278);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, -1.0), 279);
    runtime.tick_at(339);

    runtime.measure_at(Measurement::Button(Button::Lb, true), 340);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 341);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 342);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 343);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 344);
    runtime.tick_at(404);

    runtime.measure_at(Measurement::Button(Button::Lb, true), 405);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 406);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 407);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 408);
    runtime.tick_at(468);
    assert_eq!(&runtime.output.0[3..], ["t", "t", "h", "e", "Ё", "1", "u"]);

    let first_key_event = runtime.output.0.len();
    let mut time = 469;
    for button in [
        Button::DpadLeft,
        Button::DpadRight,
        Button::DpadUp,
        Button::DpadDown,
        Button::X,
    ] {
        runtime.measure_at(Measurement::Button(button, true), time);
        runtime.measure_at(Measurement::Button(button, false), time + 1);
        time += 2;
    }
    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, 0.5), time);
    runtime.measure_at(Measurement::Disconnected, time + 1);
    runtime.tick_at(time + 61);
    assert_eq!(
        &runtime.output.0[first_key_event..],
        [
            "key 123 true",
            "key 123 false",
            "key 124 true",
            "key 124 false",
            "key 126 true",
            "key 126 false",
            "key 125 true",
            "key 125 false",
            "key 51 true",
            "key 51 false"
        ]
    );
    assert_eq!(runtime.mode.as_deref(), Some("text"));
    runtime.shutdown();
}

#[test]
fn example_modifier_works_with_either_stick_and_resumes_letter_input() {
    let plugins = lua::PluginSources::discover(Path::new("example/plugins"))
        .unwrap()
        .instantiate()
        .unwrap();
    let mut runtime = setup(include_str!("../../example/config.yaml"), plugins);
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 0);

    runtime.measure_at(Measurement::Button(Button::B, true), 1);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 2);
    assert_eq!(runtime.mode.as_deref(), Some("text"));
    runtime.tick_at(61);
    assert!(runtime.output.0.is_empty());
    runtime.tick_at(62);

    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, -1.0), 63);
    runtime.tick_at(123);
    assert_eq!(runtime.output.0, [" "]);

    let upper_right_x = 3.0_f32.sqrt() / 2.0;
    runtime.measure_at(Measurement::Stick(Side::Left, upper_right_x, 0.5), 124);
    runtime.tick_at(184);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 0.0), 185);
    runtime.tick_at(245);
    assert_eq!(runtime.output.0, [" ", "."]);

    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 246);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, -1.0), 247);
    runtime.tick_at(307);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 308);
    runtime.measure_at(Measurement::Button(Button::B, false), 309);
    runtime.tick_at(369);
    assert_eq!(runtime.output.0, [" ", ".", "!"]);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 370);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 371);
    runtime.tick_at(431);
    assert_eq!(runtime.output.0.last().unwrap(), "u");

    runtime.measure_at(Measurement::Button(Button::Rb, true), 432);
    runtime.measure_at(Measurement::Button(Button::Rb, false), 433);
    runtime.measure_at(Measurement::Button(Button::B, true), 434);
    runtime.tick_at(494);
    assert_eq!(runtime.output.0, [" ", ".", "!", "u"]);
    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, 0.5), 495);
    runtime.tick_at(555);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 556);
    runtime.measure_at(Measurement::Button(Button::B, false), 557);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 558);
    runtime.tick_at(618);
    assert_eq!(runtime.output.0, [" ", ".", "!", "u", ".", "U"]);

    runtime.measure_at(Measurement::Button(Button::B, true), 619);
    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, 0.5), 620);
    runtime.measure_at(Measurement::Button(Button::B, false), 621);
    runtime.tick_at(681);
    assert_eq!(runtime.output.0, [" ", ".", "!", "u", ".", "U"]);

    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 0.0), 682);
    runtime.measure_at(Measurement::Button(Button::B, true), 683);
    runtime.measure_at(Measurement::Stick(Side::Right, upper_right_x, 0.5), 684);
    runtime.measure_at(Measurement::Disconnected, 685);
    runtime.tick_at(745);
    runtime.measure_at(Measurement::Connected, 746);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 747);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 748);
    runtime.tick_at(808);
    assert_eq!(runtime.output.0, [" ", ".", "!", "u", ".", "U", "u"]);
}

#[test]
fn modifiers_use_the_latest_held_button_and_choice_sectors() {
    let config = r#"
initial_mode: normal
modes:
  normal:
    mappings:
      - input: ls
        action: {plugin: builtin_primitives, type: change_mode, arguments: [text]}
  text:
    buttons:
      b: {behavior: {plugin: chord_text, type: modifier_key}}
      x: {behavior: {plugin: chord_text, type: modifier_key}}
    sticks:
      left: {behavior: {plugin: chord_text, type: group}}
      right: {behavior: {plugin: chord_text, type: choice}}
chord_text:
  sectors: {type: exact, groups: 2, choices: 4}
  modifier_groups:
    b: "%/ёй"
    x: " .?"
  confirm: {stable_ms: 0}
  keymaps:
    en: {groups: [ab, cd]}
"#;
    let chord = plugin(
        "chord_text",
        include_str!("../../example/plugins/chord_text.lua"),
    );
    let mut runtime = setup(config, Plugins { lua: vec![chord] });
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 0);
    runtime.measure_at(Measurement::Button(Button::X, true), 1);
    runtime.measure_at(Measurement::Button(Button::B, true), 2);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 3);
    runtime.tick_at(3);
    runtime.measure_at(Measurement::Stick(Side::Left, 1.0, 0.0), 4);
    runtime.tick_at(4);
    runtime.measure_at(Measurement::Button(Button::X, false), 5);
    runtime.tick_at(5);
    assert_eq!(runtime.output.0, ["%", "/"]);

    runtime.measure_at(Measurement::Button(Button::X, true), 6);
    runtime.tick_at(6);
    assert_eq!(runtime.output.0, ["%", "/"]);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 0.0), 7);
    runtime.measure_at(Measurement::Stick(Side::Left, 1.0, 0.0), 8);
    runtime.tick_at(8);
    runtime.measure_at(Measurement::Stick(Side::Left, -1.0, 0.0), 9);
    runtime.tick_at(9);
    assert_eq!(runtime.output.0, ["%", "/", "."]);

    runtime.measure_at(Measurement::Button(Button::X, false), 10);
    runtime.tick_at(10);
    assert_eq!(runtime.output.0, ["%", "/", "."]);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, -1.0), 11);
    runtime.tick_at(11);
    runtime.measure_at(Measurement::Button(Button::B, false), 12);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.0, 1.0), 13);
    runtime.tick_at(13);
    assert_eq!(runtime.output.0, ["%", "/", ".", "ё"]);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 0.0), 14);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 15);
    runtime.tick_at(15);
    assert_eq!(runtime.output.0, ["%", "/", ".", "ё", "a"]);
}

#[test]
fn mouse_holds_use_configured_buttons_and_clear_on_exit_and_disconnect() {
    let plugins = lua::PluginSources::discover(Path::new("example/plugins"))
        .unwrap()
        .instantiate()
        .unwrap();
    let mut runtime = setup(include_str!("../../example/config.yaml"), plugins);
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 0);
    runtime.measure_at(Measurement::Button(Button::Lb, true), 1);
    runtime.measure_at(Measurement::Button(Button::Lb, false), 2);
    runtime.measure_at(Measurement::Button(Button::Rb, true), 3);
    runtime.measure_at(Measurement::Button(Button::Lb, true), 4);
    runtime.measure_at(Measurement::Button(Button::Rb, false), 5);
    assert_eq!(
        runtime.output.0,
        ["mouse Right true", "mouse Left true", "mouse Right false"]
    );
    runtime.measure_at(Measurement::Trigger(Side::Right, 1.0), 6);
    assert_eq!(runtime.output.0.last().unwrap(), "mouse Left false");
    runtime.measure_at(Measurement::Button(Button::Lb, false), 7);
    runtime.measure_at(Measurement::Button(Button::Lb, true), 8);
    runtime.measure_at(Measurement::Button(Button::Rb, true), 9);
    runtime.measure_at(Measurement::Disconnected, 10);
    assert_eq!(
        &runtime.output.0[4..],
        ["mouse Right true", "mouse Right false"]
    );
}

#[test]
fn mouse_uses_motion_settings_and_per_behavior_overrides() {
    let config = r#"
initial_mode: mouse
modes:
  mouse:
    sticks:
      right: {behavior: {plugin: mouse, type: pointer, settings: {sensitivity: 200, invert_x: false}}}
      left: {behavior: {plugin: mouse, type: scroll}}
mouse:
  pointer: {sensitivity: 1000, acceleration: 2, dead_zone: 0.2, invert_x: true}
  scroll: {sensitivity: 40, acceleration: 1, dead_zone: 0, invert_y: true}
"#;
    let mouse = plugin("mouse", include_str!("../../example/plugins/mouse.lua"));
    let mut runtime = setup(config, Plugins { lua: vec![mouse] });
    runtime.start().unwrap();
    runtime.tick_at(0);
    runtime.measure_at(Measurement::Connected, 1);
    runtime.measure_at(Measurement::Stick(Side::Right, 0.19, 0.0), 2);
    runtime.tick_at(20);
    assert!(runtime.output.0.is_empty());
    runtime.measure_at(Measurement::Stick(Side::Right, 0.6, 0.0), 21);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 22);
    runtime.tick_at(120);
    assert_eq!(runtime.output.0, ["scroll 0 -4", "pointer 5.0 -0.0"]);
}

#[test]
fn uppercase_taps_arm_one_letter_and_holds_apply_only_until_release() {
    let config = r#"
initial_mode: text
modes:
  text:
    buttons:
      rb: {behavior: {plugin: chord_text, type: uppercase, settings: {hold_ms: 100}}}
    sticks:
      left: {behavior: {plugin: chord_text, type: group}}
      right: {behavior: {plugin: chord_text, type: choice}}
chord_text:
  confirm: {stable_ms: 0}
  uppercase: {hold_ms: 250}
  keymaps:
    en: {groups: ['a.bc']}
"#;
    let chord = plugin(
        "chord_text",
        include_str!("../../example/plugins/chord_text.lua"),
    );
    let mut runtime = setup(config, Plugins { lua: vec![chord] });
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 0);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 1);

    fn rb(runtime: &mut Runtime<TestOutput>, down: bool, time: u64) {
        runtime.measure_at(Measurement::Button(Button::Rb, down), time);
    }

    fn choose(runtime: &mut Runtime<TestOutput>, x: f32, y: f32, time: u64) {
        runtime.measure_at(Measurement::Stick(Side::Right, x, y), time);
        runtime.tick_at(time);
    }

    rb(&mut runtime, true, 5);
    rb(&mut runtime, false, 10);
    choose(&mut runtime, 1.0, 0.0, 11);
    choose(&mut runtime, 0.0, -1.0, 12);
    choose(&mut runtime, -1.0, 0.0, 13);
    assert_eq!(runtime.output.0, [".", "B", "c"]);

    rb(&mut runtime, true, 50);
    choose(&mut runtime, 0.0, -1.0, 149);
    choose(&mut runtime, 0.0, 1.0, 150);
    choose(&mut runtime, -1.0, 0.0, 151);
    rb(&mut runtime, false, 200);
    choose(&mut runtime, 0.0, -1.0, 201);
    assert_eq!(&runtime.output.0[3..], ["b", "A", "C", "b"]);

    rb(&mut runtime, true, 210);
    rb(&mut runtime, false, 310); // Долгое нажатие без тика между краями.
    choose(&mut runtime, 0.0, 1.0, 311);
    assert_eq!(runtime.output.0.last().unwrap(), "a");

    rb(&mut runtime, true, 320);
    runtime.measure_at(Measurement::Disconnected, 430);
    runtime.measure_at(Measurement::Connected, 440);
    rb(&mut runtime, false, 441);
    runtime.measure_at(Measurement::Stick(Side::Left, 0.0, 1.0), 442);
    choose(&mut runtime, -1.0, 0.0, 443);
    assert_eq!(runtime.output.0.last().unwrap(), "c");
}

#[test]
fn trigger_changes_mode_before_both_events_and_keeps_threshold_state() {
    let config = r#"
initial_mode: normal
modes:
  normal:
    mappings:
      - {input: rt, action: {plugin: builtin_primitives, type: change_mode, arguments: [next]}}
    buttons: {rt: {behavior: {plugin: observer, type: observe}}}
  next:
    triggers:
      right:
        behavior:
          plugin: observer
          type: observe
          arguments: [sample, 42, null, true, [false], {scale: 1.5}]
    buttons: {rt: {behavior: {plugin: observer, type: observe}}}
"#;
    let observer = plugin(
        "observer",
        r#"
local function text(value)
    return {{ plugin = "builtin_primitives", type = "type_text", arguments = { value } }}
end
local function transition_time(e, c)
    assert(e.time_ms == 3 and c.time.monotonic_ms == 3)
    assert(e.unix_ms == 1760000000003 and c.time.unix_ms == e.unix_ms)
end
local function check_trigger_arguments(e)
    local args = e.behavior.arguments
    assert(#args == 6 and args[1] == "sample" and args[2] == 42)
    assert(args[3] == yaml_null and args[4] == true)
    assert(args[5][1] == false and args[6].scale == 1.5)
end
return { event_handlers = {
    button_behavior_exited = function(e, c)
        transition_time(e, c)
        return text("exit " .. e.mode .. " now " .. c.active_mode)
    end,
    mode_changed = function(e, c)
        transition_time(e, c)
        return text(e.previous_mode .. " to " .. e.new_mode)
    end,
    trigger_behavior_entered = function(e)
        check_trigger_arguments(e)
        return text("trigger enters " .. e.mode)
    end,
    button_behavior_entered = function(e)
        assert(#e.behavior.arguments == 0)
        return text("button enters " .. e.mode)
    end,
    trigger_moved = function(e, c)
        check_trigger_arguments(e)
        if e.time_ms == 3 then
            transition_time(e, c)
            assert(e.input_mode == "normal")
        end
        return text(e.type .. " " .. e.time_ms .. " " .. e.mode)
    end,
    button_pressed = function(e, c)
        assert(e.button == "rt" and e.source.kind == "trigger" and e.source.side == "right")
        assert(c.controller.buttons.rt.pressed_at_ms == c.time.monotonic_ms)
        assert(e.input_mode == "normal")
        transition_time(e, c)
        return text(e.type .. " " .. e.time_ms .. " " .. e.mode)
    end,
    button_unpressed = function(e, c)
        assert(not c.controller.buttons.rt.pressed)
        return text(e.type .. " " .. e.time_ms .. " " .. e.mode)
    end,
} }
"#,
    );
    let mut runtime = setup(
        config,
        Plugins {
            lua: vec![observer],
        },
    );
    runtime.start().unwrap();
    runtime.output.0.clear();
    runtime.measure_at(Measurement::Connected, 1);
    runtime.measure_at(Measurement::Trigger(Side::Right, 0.30), 2);
    runtime.measure_timestamp(
        Measurement::Trigger(Side::Right, 0.31),
        Timestamp {
            monotonic_ms: 3,
            unix_ms: 1_760_000_000_003,
        },
    );
    runtime.measure_at(Measurement::Trigger(Side::Right, 0.30), 4);
    runtime.measure_at(Measurement::Trigger(Side::Right, 0.29), 5);
    assert_eq!(
        runtime.output.0,
        [
            "exit normal now next",
            "normal to next",
            "trigger enters next",
            "button enters next",
            "trigger_moved 3 next",
            "button_pressed 3 next",
            "trigger_moved 4 next",
            "trigger_moved 5 next",
            "button_unpressed 5 next",
        ]
    );
}

#[test]
fn shared_holds_are_released_once_on_mode_exit_or_disconnect() {
    let config = r#"
initial_mode: hold
modes:
  hold:
    mappings:
      - {input: rt, action: {plugin: builtin_primitives, type: change_mode, arguments: [empty]}}
    buttons:
      x: {behavior: {plugin: keyboard, type: hold}}
      y: {behavior: {plugin: keyboard, type: hold}}
  empty: {}
"#;
    let keyboard = plugin(
        "keyboard",
        r#"
local function key(action_type)
    return function()
        return {{ plugin = "builtin_primitives", type = action_type, arguments = { "shift" } }}
    end
end
return { event_handlers = { button_pressed = key("key_down"), button_unpressed = key("key_up") } }
"#,
    );
    let mut runtime = setup(
        config,
        Plugins {
            lua: vec![keyboard],
        },
    );
    runtime.start().unwrap();
    runtime.measure_at(Measurement::Connected, 1);
    runtime.measure_at(Measurement::Button(Button::X, true), 2);
    runtime.measure_at(Measurement::Button(Button::Y, true), 3);
    runtime.measure_at(Measurement::Button(Button::X, false), 4);
    assert_eq!(runtime.output.0, ["key 56 true"]);
    runtime.measure_at(Measurement::Disconnected, 5);
    assert_eq!(runtime.output.0, ["key 56 true", "key 56 false"]);
    runtime.measure_at(Measurement::Connected, 6);
    runtime.measure_at(Measurement::Button(Button::X, true), 7);
    runtime.measure_at(Measurement::Trigger(Side::Right, 0.5), 8);
    runtime.measure_at(Measurement::Button(Button::X, false), 9);
    runtime.shutdown();
    assert_eq!(
        runtime.output.0,
        ["key 56 true", "key 56 false", "key 56 true", "key 56 false"]
    );
}

#[test]
fn nested_actions_run_before_siblings_and_get_the_new_state() {
    let code = r#"
local function text(value)
    return { plugin = "builtin_primitives", type = "type_text", arguments = { value } }
end
return {
    event_handlers = {
        ready = function()
            return { state = { seen = true }, actions = {
                { plugin = "demo", type = "expand", arguments = {yaml_null, {answer=42}} },
                text("last"),
            }}
        end,
    },
    on_action = function(action, context)
        assert(context.state.seen and context.origin.plugin == "demo")
        assert(action.arguments[1] == yaml_null and action.arguments[2].answer == 42)
        return { text("first"), text("second") }
    end,
}
"#;
    let mut runtime = setup(
        "initial_mode: base\nmodes: {base: {}}",
        Plugins {
            lua: vec![plugin("demo", code)],
        },
    );
    runtime.start().unwrap();
    assert_eq!(runtime.output.0, ["first", "second", "last"]);

    let origin = Origin {
        plugin: "test".into(),
        activation_id: None,
    };
    let invalid = Action {
        plugin: PRIMITIVES.into(),
        kind: "does_not_exist".into(),
        arguments: vec![],
    };
    assert!(
        runtime
            .perform(
                vec![
                    (text_action("before"), origin.clone()),
                    (invalid, origin.clone()),
                    (text_action("after"), origin),
                ],
                &mut Chain::default()
            )
            .is_err()
    );
    assert_eq!(runtime.output.0, ["first", "second", "last", "before"]);
}

#[test]
fn startup_rolls_back_only_plugins_that_loaded_and_releases_holds() {
    let a = plugin(
        "a",
        r#"
return { event_handlers = {
    load = function() return {{ plugin = "builtin_primitives", type = "key_down", arguments = {"shift"} }} end,
    unload = function() return {{ plugin = "builtin_primitives", type = "type_text", arguments = {"unloaded a"} }} end,
} }
"#,
    );
    let b = plugin(
        "b",
        r#"
return { event_handlers = {
    load = function() error("bad settings") end,
    unload = function() error("never called") end,
} }
"#,
    );
    let mut runtime = setup(
        "initial_mode: base\nmodes: {base: {}}",
        Plugins { lua: vec![a, b] },
    );
    assert!(runtime.start().unwrap_err().contains("bad settings"));
    assert_eq!(
        runtime.output.0,
        ["key 56 true", "unloaded a", "key 56 false"]
    );
    assert_eq!(runtime.active_count, 0);
}

#[test]
fn mode_cycle_is_bounded_without_losing_already_queued_events() {
    let config = r#"
initial_mode: one
modes:
  one: {sticks: {left: {behavior: {plugin: cycle, type: watch}}}}
  two: {sticks: {left: {behavior: {plugin: cycle, type: watch}}}}
"#;
    let cycle = plugin(
        "cycle",
        r#"
return { event_handlers = {
    stick_behavior_entered = function(e)
        local next = e.mode == "one" and "two" or "one"
        return {
            { plugin = "builtin_primitives", type = "type_text", arguments = { e.mode } },
            { plugin = "builtin_primitives", type = "change_mode", arguments = { next } },
        }
    end,
} }
"#,
    );
    let mut runtime = setup(config, Plugins { lua: vec![cycle] });
    let error = runtime.start().unwrap_err();
    assert!(error.contains("превышен лимит цепочки"));
    assert!(runtime.output.0.len() > 20);
    assert!(runtime.output.0.len() < MAX_STEPS);
    assert!(runtime.events.is_empty());
    assert!(runtime.activations.is_empty());
    assert_eq!(runtime.active_count, 0);
}
