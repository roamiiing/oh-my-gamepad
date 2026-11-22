use std::{
    collections::BTreeSet,
    ffi::c_void,
    ptr::{NonNull, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use block2::RcBlock;
use core_graphics::{
    event::{
        CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, KeyCode,
        ScrollEventUnit,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use objc2::{
    ClassType, MainThreadMarker,
    rc::{Retained, autoreleasepool},
    runtime::{NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use objc2_game_controller::{
    GCController, GCControllerButtonInput, GCControllerDidConnectNotification,
    GCControllerDidDisconnectNotification, GCControllerDirectionPad, GCExtendedGamepad,
    GCSystemGestureState, GCXboxGamepad,
};

use crate::{
    input::{Button, CapturedTime, Measurement, Side},
    output::{Key, MouseButton, Output},
};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

pub struct MacOutput {
    source: CGEventSource,
    modifiers: BTreeSet<u16>,
    mouse_buttons: [bool; 3],
}

impl MacOutput {
    pub fn new() -> Result<Self, String> {
        if !unsafe { CGPreflightPostEventAccess() } {
            return Err(
                "Нет доступа к системному вводу. Разрешите его в настройках macOS «Конфиденциальность и безопасность → Универсальный доступ» и перезапустите программу."
                    .into(),
            );
        }
        Ok(Self {
            source: CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|_| "Не удалось создать источник событий macOS")?,
            modifiers: BTreeSet::new(),
            mouse_buttons: [false; 3],
        })
    }

    fn location(&self) -> Result<CGPoint, String> {
        CGEvent::new(self.source.clone())
            .map(|event| event.location())
            .map_err(|_| "Не удалось получить положение указателя".into())
    }

    fn modifier_flags(&self) -> CGEventFlags {
        self.modifiers
            .iter()
            .map(|&code| modifier_flag(code))
            .fold(CGEventFlags::empty(), |flags, flag| flags | flag)
    }

    fn post(&self, event: &CGEvent) {
        event.set_flags(event.get_flags() | self.modifier_flags());
        event.post(CGEventTapLocation::HID);
    }
}

fn modifier_flag(code: u16) -> CGEventFlags {
    match code {
        KeyCode::SHIFT | KeyCode::RIGHT_SHIFT => CGEventFlags::CGEventFlagShift,
        KeyCode::CONTROL | KeyCode::RIGHT_CONTROL => CGEventFlags::CGEventFlagControl,
        KeyCode::OPTION | KeyCode::RIGHT_OPTION => CGEventFlags::CGEventFlagAlternate,
        KeyCode::COMMAND | KeyCode::RIGHT_COMMAND => CGEventFlags::CGEventFlagCommand,
        KeyCode::FUNCTION => CGEventFlags::CGEventFlagSecondaryFn,
        _ => CGEventFlags::empty(),
    }
}

impl Output for MacOutput {
    fn key(&mut self, key: Key, down: bool) -> Result<(), String> {
        let event = CGEvent::new_keyboard_event(self.source.clone(), key.0, down)
            .map_err(|_| "Не удалось создать событие клавиатуры")?;
        let modifier = modifier_flag(key.0);
        if !modifier.is_empty() {
            if down {
                self.modifiers.insert(key.0);
            } else {
                self.modifiers.remove(&key.0);
            }
            event.set_type(CGEventType::FlagsChanged);
            let modifiers = self.modifier_flags();
            event.set_flags((event.get_flags() - modifier) | modifiers);
        }
        self.post(&event);
        Ok(())
    }

    fn mouse(&mut self, button: MouseButton, down: bool) -> Result<(), String> {
        let (index, button, press, release) = match button {
            MouseButton::Left => (
                0,
                CGMouseButton::Left,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
            ),
            MouseButton::Right => (
                1,
                CGMouseButton::Right,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
            ),
            MouseButton::Middle => (
                2,
                CGMouseButton::Center,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
            ),
        };
        let event = CGEvent::new_mouse_event(
            self.source.clone(),
            if down { press } else { release },
            self.location()?,
            button,
        )
        .map_err(|_| "Не удалось создать событие кнопки мыши")?;
        event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
        self.post(&event);
        self.mouse_buttons[index] = down;
        Ok(())
    }

    fn move_pointer(&mut self, dx: f64, dy: f64) -> Result<(), String> {
        let position = self.location()?;
        let (kind, button) = match self.mouse_buttons {
            [true, _, _] => (CGEventType::LeftMouseDragged, CGMouseButton::Left),
            [_, true, _] => (CGEventType::RightMouseDragged, CGMouseButton::Right),
            [_, _, true] => (CGEventType::OtherMouseDragged, CGMouseButton::Center),
            _ => (CGEventType::MouseMoved, CGMouseButton::Left),
        };
        let event = CGEvent::new_mouse_event(
            self.source.clone(),
            kind,
            CGPoint::new(position.x + dx, position.y + dy),
            button,
        )
        .map_err(|_| "Не удалось создать событие движения мыши")?;
        self.post(&event);
        Ok(())
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let event =
            CGEvent::new_scroll_event(self.source.clone(), ScrollEventUnit::PIXEL, 2, dy, dx, 0)
                .map_err(|_| "Не удалось создать событие прокрутки")?;
        self.post(&event);
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<(), String> {
        for character in text.chars() {
            let mut utf16 = [0; 2];
            let character = character.encode_utf16(&mut utf16);
            let press = CGEvent::new_keyboard_event(self.source.clone(), 0, true)
                .map_err(|_| "Не удалось создать событие ввода текста")?;
            let release = CGEvent::new_keyboard_event(self.source.clone(), 0, false)
                .map_err(|_| "Не удалось создать событие ввода текста")?;
            for event in [press, release] {
                event.set_flags(CGEventFlags::empty());
                event.set_string_from_utf16_unchecked(character);
                event.post(CGEventTapLocation::HID);
            }
        }
        Ok(())
    }
}

enum GameControllerEvent {
    Connected,
    Disconnected(usize),
    Input(u64, Measurement),
}

type ControllerSender = Sender<(GameControllerEvent, CapturedTime)>;
type TimedMeasurement = (Measurement, CapturedTime);

fn send_event(sender: &ControllerSender, event: GameControllerEvent) {
    let _ = sender.send((event, CapturedTime::now()));
}

struct ActivePad {
    controller: Retained<GCController>,
    generation: u64,
    buttons: Vec<Retained<GCControllerButtonInput>>,
    triggers: [Retained<GCControllerButtonInput>; 2],
    sticks: [Retained<GCControllerDirectionPad>; 2],
    share_gesture: Option<(Retained<GCControllerButtonInput>, GCSystemGestureState)>,
}

impl ActivePad {
    fn attach(
        controller: Retained<GCController>,
        profile: &GCExtendedGamepad,
        generation: u64,
        sender: &ControllerSender,
    ) -> (Self, Vec<Measurement>) {
        // Game Controller копирует обработчики и передаёт им данные контроллера.
        unsafe {
            let dpad = profile.dpad();
            let share = profile
                .downcast_ref::<GCXboxGamepad>()
                .filter(|xbox| xbox.respondsToSelector(sel!(buttonShare)))
                .and_then(|xbox| xbox.buttonShare());
            let share_gesture = share.as_ref().map(|input| {
                let old_state = input.preferredSystemGestureState();
                input.setPreferredSystemGestureState(GCSystemGestureState::Disabled);
                (input.clone(), old_state)
            });
            let buttons = [
                (Button::A, Some(profile.buttonA())),
                (Button::B, Some(profile.buttonB())),
                (Button::X, Some(profile.buttonX())),
                (Button::Y, Some(profile.buttonY())),
                (Button::Lb, Some(profile.leftShoulder())),
                (Button::Rb, Some(profile.rightShoulder())),
                (Button::Lsb, profile.leftThumbstickButton()),
                (Button::Rsb, profile.rightThumbstickButton()),
                (Button::Menu, Some(profile.buttonMenu())),
                (Button::Options, profile.buttonOptions()),
                (Button::Home, profile.buttonHome()),
                (Button::DpadUp, Some(dpad.up())),
                (Button::DpadDown, Some(dpad.down())),
                (Button::DpadLeft, Some(dpad.left())),
                (Button::DpadRight, Some(dpad.right())),
                (Button::Share, share),
            ];
            let triggers = [profile.leftTrigger(), profile.rightTrigger()];
            let sticks = [profile.leftThumbstick(), profile.rightThumbstick()];
            let mut initial = Vec::new();
            let mut button_inputs = Vec::new();

            for (button, input) in buttons {
                if let Some(input) = input {
                    let sender = sender.clone();
                    let block = RcBlock::new(move |_, _: f32, pressed: objc2::runtime::Bool| {
                        let measurement = Measurement::Button(button, pressed.as_bool());
                        send_event(&sender, GameControllerEvent::Input(generation, measurement));
                    });
                    input.setPressedChangedHandler(RcBlock::as_ptr(&block));
                    initial.push(Measurement::Button(button, input.isPressed()));
                    button_inputs.push(input);
                }
            }

            for (side, input) in [Side::Left, Side::Right].into_iter().zip(&triggers) {
                let sender = sender.clone();
                let block = RcBlock::new(move |_, value: f32, _: objc2::runtime::Bool| {
                    let measurement = Measurement::Trigger(side, value);
                    send_event(&sender, GameControllerEvent::Input(generation, measurement));
                });
                input.setValueChangedHandler(RcBlock::as_ptr(&block));
                initial.push(Measurement::Trigger(side, input.value()));
            }

            for (side, input) in [Side::Left, Side::Right].into_iter().zip(&sticks) {
                let sender = sender.clone();
                let block = RcBlock::new(move |_, x: f32, y: f32| {
                    let measurement = Measurement::Stick(side, x, y);
                    send_event(&sender, GameControllerEvent::Input(generation, measurement));
                });
                input.setValueChangedHandler(RcBlock::as_ptr(&block));
                initial.push(Measurement::Stick(
                    side,
                    input.xAxis().value(),
                    input.yAxis().value(),
                ));
            }

            (
                Self {
                    controller,
                    generation,
                    buttons: button_inputs,
                    triggers,
                    sticks,
                    share_gesture,
                },
                initial,
            )
        }
    }

    fn identity(&self) -> usize {
        Retained::as_ptr(&self.controller) as usize
    }
}

impl Drop for ActivePad {
    fn drop(&mut self) {
        // Пустой обработчик отменяет подписку. События старого подключения отсеются в очереди.
        unsafe {
            for input in &self.buttons {
                input.setPressedChangedHandler(null_mut());
            }
            for input in &self.triggers {
                input.setValueChangedHandler(null_mut());
            }
            for input in &self.sticks {
                input.setValueChangedHandler(null_mut());
            }
            if let Some((input, state)) = &self.share_gesture {
                input.setPreferredSystemGestureState(*state);
            }
        }
    }
}

struct ConnectionObservers {
    center: Retained<NSNotificationCenter>,
    tokens: [Retained<ProtocolObject<dyn NSObjectProtocol>>; 2],
}

impl ConnectionObservers {
    fn new(sender: &ControllerSender) -> Self {
        let center = NSNotificationCenter::defaultCenter();
        let connect = {
            let sender = sender.clone();
            RcBlock::new(move |_: NonNull<NSNotification>| {
                send_event(&sender, GameControllerEvent::Connected);
            })
        };
        let disconnect = {
            let sender = sender.clone();
            RcBlock::new(move |note: NonNull<NSNotification>| {
                // Уведомление остаётся действительным до возврата из обработчика.
                if let Some(controller) = unsafe { note.as_ref() }.object() {
                    let identity = Retained::as_ptr(&controller) as usize;
                    send_event(&sender, GameControllerEvent::Disconnected(identity));
                }
            })
        };
        // Обработчики сохраняют только отправители канала, безопасные для разных потоков.
        let tokens = unsafe {
            [
                center.addObserverForName_object_queue_usingBlock(
                    Some(GCControllerDidConnectNotification),
                    None,
                    None,
                    &connect,
                ),
                center.addObserverForName_object_queue_usingBlock(
                    Some(GCControllerDidDisconnectNotification),
                    None,
                    None,
                    &disconnect,
                ),
            ]
        };
        Self { center, tokens }
    }
}

impl Drop for ConnectionObservers {
    fn drop(&mut self) {
        // Токены принадлежат этому центру уведомлений.
        unsafe {
            for token in &self.tokens {
                self.center.removeObserver((**token).as_ref());
            }
        }
    }
}

fn connect(
    generation: u64,
    excluded: Option<usize>,
    time: CapturedTime,
    sender: &ControllerSender,
    handle: &mut impl FnMut(Option<TimedMeasurement>),
) -> Option<ActivePad> {
    // Системный список удерживает ссылки на контроллеры до конца чтения.
    let controllers = unsafe { GCController::controllers() };
    for controller in controllers.iter() {
        if Some(Retained::as_ptr(&controller) as usize) == excluded {
            continue;
        }
        if let Some(profile) = unsafe { controller.extendedGamepad() } {
            let (pad, initial) = ActivePad::attach(controller, &profile, generation, sender);
            handle(Some((Measurement::Connected, time)));
            for measurement in initial {
                handle(Some((measurement, time)));
            }
            return Some(pad);
        }
    }
    None
}

pub fn shutdown_signal() -> Result<Arc<AtomicBool>, String> {
    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = Arc::clone(&stop);
        move || stop.store(true, Ordering::Relaxed)
    })
    .map_err(|err| format!("Не удалось настроить обработку завершения: {err}"))?;
    Ok(stop)
}

pub fn run(
    mut handle: impl FnMut(Option<TimedMeasurement>),
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    if MainThreadMarker::new().is_none() {
        return Err("Обработчик контроллера должен работать в главном потоке".into());
    }
    // До macOS 11.3 фоновый ввод был включён по умолчанию.
    if GCController::class()
        .class_method(sel!(setShouldMonitorBackgroundEvents:))
        .is_some()
    {
        unsafe { GCController::setShouldMonitorBackgroundEvents(true) };
    }

    let (sender, receiver) = mpsc::channel();
    let _observers = ConnectionObservers::new(&sender);
    let mut generation = 1;
    let mut active = connect(generation, None, CapturedTime::now(), &sender, &mut handle);
    let interval = Duration::from_millis(10);
    let mut next_tick = Instant::now() + interval;

    while !stop.load(Ordering::Relaxed) {
        // Через системный цикл приходят события Game Controller.
        let wait = next_tick.saturating_duration_since(Instant::now());
        let result = autoreleasepool(|_| unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, wait.as_secs_f64(), 1)
        });

        for (event, time) in receiver.try_iter().take(256) {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            autoreleasepool(|_| match event {
                GameControllerEvent::Connected if active.is_none() => {
                    generation += 1;
                    active = connect(generation, None, time, &sender, &mut handle);
                }
                GameControllerEvent::Disconnected(identity)
                    if active
                        .as_ref()
                        .is_some_and(|pad| pad.identity() == identity) =>
                {
                    drop(active.take());
                    handle(Some((Measurement::Disconnected, time)));
                    generation += 1;
                    active = connect(generation, Some(identity), time, &sender, &mut handle);
                }
                GameControllerEvent::Input(generation, input)
                    if active
                        .as_ref()
                        .is_some_and(|pad| pad.generation == generation) =>
                {
                    handle(Some((input, time)));
                }
                _ => {}
            });
        }

        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        if now >= next_tick {
            handle(None);
            next_tick = now + interval;
        } else if result == 1 {
            // Цикл без источников событий возвращается сразу.
            thread::sleep(next_tick.saturating_duration_since(now));
        }
    }

    Ok(())
}
