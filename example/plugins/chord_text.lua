local dead_zone = 0.25
local stick_sides = { "left", "right" }
local keymaps = {}
local keymap_index = 1
local active = {}
local settings
local caps, uppercase_once = false, false
local uppercase_buttons = {}
local modifier_groups = {}
local held_modifiers, modifier = {}, nil
local wait_for_center = {}
local candidate, last_confirmed

local function hold_threshold(options, default)
  assert(options == nil or type(options) == "table", "chord_text.uppercase: настройки должны быть словарём")
  local hold_ms = options and options.hold_ms
  if hold_ms == nil then
    return default
  end
  assert(math.type(hold_ms) == "integer" and hold_ms > 0,
    "chord_text.uppercase.hold_ms должен быть положительным целым числом")
  return hold_ms
end

local function characters(text)
  local result = {}
  for _, codepoint in utf8.codes(text) do
    result[#result + 1] = utf8.char(codepoint)
  end
  return result
end

local function validate_usage(config)
  local used = false
  for name, mode in pairs(config.modes) do
    for _, source in ipairs({ "sticks", "triggers", "buttons" }) do
      for id, mapping in pairs(mode[source] or {}) do
        local behavior = mapping.behavior
        if behavior and behavior.plugin == "chord_text" then
          local expected = ({ left = "group", right = "choice" })[id]
          if source == "buttons" and behavior.type == "uppercase" then
            hold_threshold(behavior.settings, 250)
          elseif source == "buttons" and behavior.type == "modifier_key" then
            assert(modifier_groups[id] ~= nil,
              name .. ".buttons." .. id .. ": нужна группа chord_text.modifier_groups." .. id)
          else
            assert(source == "sticks" and behavior.type == expected,
              name .. "." .. source .. "." .. id
                .. ": chord_text ожидает group слева, choice справа, uppercase или modifier_key на кнопке")
          end
          used = true
        end
      end
    end
    for _, mapping in ipairs(mode.mappings or {}) do
      if mapping.action.plugin == "chord_text" then
        used = true
      end
    end
  end
  return used
end

local function load(_, context)
  local config = context.config.chord_text
  if config == nil then
    assert(not validate_usage(context.config), "нужна секция chord_text")
    return
  end
  assert(type(config) == "table", "нужна секция chord_text")

  local sectors = config.sectors == nil and { type = "adaptive" } or config.sectors
  local confirm = config.confirm == nil and {} or config.confirm
  local options = config.options == nil and {} or config.options
  local hold_ms = hold_threshold(config.uppercase, 250)
  assert(type(sectors) == "table" and type(confirm) == "table" and type(options) == "table",
    "chord_text: sectors, confirm и options должны быть словарями")
  assert(sectors.type == "exact" or sectors.type == "adaptive",
    "chord_text.sectors.type должен быть exact или adaptive")
  if sectors.type == "exact" then
    assert(math.type(sectors.groups) == "integer" and sectors.groups > 0,
      "chord_text.sectors.groups должен быть положительным целым числом")
    assert(math.type(sectors.choices) == "integer" and sectors.choices > 0,
      "chord_text.sectors.choices должен быть положительным целым числом")
  end
  local exact = sectors.type == "exact"
  local groups = config.modifier_groups
  if groups ~= nil then
    assert(type(groups) == "table", "chord_text.modifier_groups должен быть словарём")
    for button, text in pairs(groups) do
      assert(type(button) == "string" and button ~= "",
        "ключ chord_text.modifier_groups должен быть именем кнопки")
      local path = "chord_text.modifier_groups." .. button
      assert(type(text) == "string" and text ~= "", path .. " должна быть непустой строкой")
      local group = characters(text)
      assert(not exact or #group <= sectors.choices, path .. ": символов больше, чем секторов choices")
      modifier_groups[button] = group
    end
  end
  validate_usage(context.config)

  local stable_ms = confirm.stable_ms == nil and 60 or confirm.stable_ms
  local tolerance = confirm.angle_tolerance_deg == nil and 5 or confirm.angle_tolerance_deg
  local keymap_on_exit = options.keymap_on_exit == nil and "keep" or options.keymap_on_exit
  local caps_on_exit = options.caps_on_exit == nil and "reset" or options.caps_on_exit
  assert(type(stable_ms) == "number" and stable_ms >= 0 and stable_ms < math.huge,
    "chord_text.confirm.stable_ms должен быть конечным неотрицательным числом")
  assert(type(tolerance) == "number" and tolerance >= 0 and tolerance <= 180,
    "chord_text.confirm.angle_tolerance_deg должен быть числом от 0 до 180")
  assert(keymap_on_exit == "keep" or keymap_on_exit == "reset",
    "chord_text.options.keymap_on_exit должен быть keep или reset")
  assert(caps_on_exit == "keep" or caps_on_exit == "reset",
    "chord_text.options.caps_on_exit должен быть keep или reset")

  assert(type(config.keymaps) == "table", "chord_text.keymaps должен быть словарём")
  for name, map in pairs(config.keymaps) do
    assert(type(name) == "string" and name ~= "", "имя таблицы chord_text должно быть непустой строкой")
    assert(type(map) == "table" and type(map.groups) == "table" and #map.groups > 0,
      "chord_text.keymaps." .. name .. ".groups должен быть непустым списком")
    assert(not exact or #map.groups <= sectors.groups,
      "в таблице " .. name .. " групп больше, чем левых секторов")
    local groups = {}
    for _, text in ipairs(map.groups) do
      assert(type(text) == "string", "в таблице " .. name .. " каждая группа должна быть строкой")
      local group = characters(text)
      assert(not exact or #group <= sectors.choices,
        "в таблице " .. name .. " символов в группе больше, чем правых секторов")
      assert(exact or #group > 0, "в режиме adaptive группы должны содержать символы")
      groups[#groups + 1] = group
    end
    keymaps[#keymaps + 1] = { name = name, groups = groups }
  end
  assert(#keymaps > 0, "chord_text.keymaps должен содержать хотя бы одну таблицу")
  settings = {
    exact = exact, groups = sectors.groups, choices = sectors.choices,
    stable_ms = stable_ms, tolerance = tolerance,
    keymap_on_exit = keymap_on_exit, caps_on_exit = caps_on_exit, hold_ms = hold_ms,
  }
end

local function angle(stick)
  if stick.x * stick.x + stick.y * stick.y <= dead_zone * dead_zone then
    return nil
  end
  return math.deg(math.atan(stick.x, stick.y)) % 360
end

local function sector(degrees, count)
  return math.floor(degrees * count / 360 + 0.5) % count + 1
end

local function angle_difference(a, b)
  return math.abs((a - b + 180) % 360 - 180)
end

local function same_choice(a, b)
  return a and b and a.group == b.group and a.choice == b.choice and a.side == b.side
end

local function modifier_sector(degrees, group)
  return degrees and sector(degrees, settings.exact and settings.choices or #group)
end

local function read_modifier_choice(left_angle, right_angle)
  local group = modifier.binding.group
  local choices = { left = modifier_sector(left_angle, group), right = modifier_sector(right_angle, group) }
  for _, side in ipairs(stick_sides) do
    if modifier.blocked[side] ~= choices[side] then
      modifier.blocked[side] = nil
    end
  end
  if modifier.side and choices[modifier.side] == nil then
    modifier.side, last_confirmed = nil, nil
  end
  if not modifier.side then
    for _, side in ipairs(stick_sides) do
      if active[side] and choices[side] and not modifier.blocked[side] then
        modifier.side = side
        break
      end
    end
  end
  local side = modifier.side
  if not side then
    return nil
  end
  local other = side == "left" and "right" or "left"
  modifier.blocked[other] = choices[other]
  local choice = choices[side]
  local text = group[choice]
  if text then
    return { side = side, choice = choice, text = text,
      primary_angle = side == "left" and left_angle or right_angle }
  end
end

local function read_choice(context)
  if not context.controller.connected then
    return nil
  end
  local sticks = context.controller.sticks
  local left_angle = angle(sticks.left)
  local right_angle = angle(sticks.right)
  if not left_angle then
    wait_for_center.left = nil
  end
  if not right_angle then
    wait_for_center.right = nil
  end
  if modifier then
    return read_modifier_choice(left_angle, right_angle)
  end
  if not (active.left and active.right) then
    return nil
  end
  if not right_angle then
    last_confirmed = nil
  end
  if not (left_angle and right_angle) or next(wait_for_center) then
    return nil
  end

  local groups = keymaps[keymap_index].groups
  local group_index = sector(left_angle, settings.exact and settings.groups or #groups)
  local group = groups[group_index]
  if group == nil then
    return nil
  end
  local choice = sector(right_angle, settings.exact and settings.choices or #group)
  if group[choice] == nil then
    return nil
  end
  return {
    group = group_index, choice = choice, text = group[choice],
    primary_angle = left_angle, secondary_angle = right_angle,
  }
end

local function observe(time_ms, context)
  local choice = read_choice(context)
  if not choice or same_choice(choice, last_confirmed) then
    candidate = nil
  elseif not same_choice(choice, candidate)
    or angle_difference(choice.primary_angle, candidate.primary_angle) > settings.tolerance
    or (choice.secondary_angle
      and angle_difference(choice.secondary_angle, candidate.secondary_angle) > settings.tolerance) then
    candidate = choice
    candidate.started_at_ms = time_ms
  end
end

local function stick_entered(event, context)
  active[event.source.side] = true
  observe(event.time_ms, context)
end

local function stick_moved(event, context)
  if modifier and event.input_mode ~= event.mode then
    -- Это движение включило режим ввода при уже зажатом модификаторе.
    modifier.blocked[event.source.side] = nil
  end
  observe(event.time_ms, context)
end

local function reset_choice()
  candidate, last_confirmed = nil, nil
  wait_for_center, uppercase_once = {}, false
  uppercase_buttons = {}
  held_modifiers, modifier = {}, nil
end

local function stick_exited(event)
  local had_session = active.left and active.right
  active[event.source.side] = nil
  if had_session then
    reset_choice()
    if settings.keymap_on_exit == "reset" then
      keymap_index = 1
    end
    if settings.caps_on_exit == "reset" then
      caps = false
    end
  end
end

local function emit_text(text)
  return {{ plugin = "builtin_primitives", type = "type_text", arguments = { text } }}
end

local function uppercase(text)
  if string.unicode_upper then
    return string.unicode_upper(text)
  end
  local codepoint = utf8.codepoint(text)
  if codepoint >= 0x0430 and codepoint <= 0x044F then
    return utf8.char(codepoint - 32)
  end
  if codepoint == 0x0451 then
    return "Ё"
  end
  return string.upper(text)
end

local function caps_held(time_ms)
  for _, button in pairs(uppercase_buttons) do
    if time_ms - button.pressed_at_ms >= button.hold_ms then
      return true
    end
  end
  return false
end

local function set_modifier_button(event, context, pressed)
  local previous = held_modifiers[#held_modifiers]
  for index, binding in ipairs(held_modifiers) do
    if binding.button == event.button then
      table.remove(held_modifiers, index)
      break
    end
  end
  if pressed then
    local pressed_at = context.controller.buttons[event.button].pressed_at_ms
    local index = #held_modifiers + 1
    -- При входе в режим учитываем исходное время нажатия зажатых кнопок.
    while index > 1 and held_modifiers[index - 1].pressed_at > pressed_at do
      index = index - 1
    end
    table.insert(held_modifiers, index, {
      button = event.button,
      group = modifier_groups[event.button],
      pressed_at = pressed_at,
    })
  end

  local binding = held_modifiers[#held_modifiers]
  if binding == previous then
    return
  end

  local sticks = context.controller.sticks
  if binding then
    modifier = {
      binding = binding,
      blocked = {
        left = modifier_sector(angle(sticks.left), binding.group),
        right = modifier_sector(angle(sticks.right), binding.group),
      },
    }
  else
    local side = modifier.side or "right"
    wait_for_center[side] = angle(sticks[side]) and true
    modifier = nil
  end
  candidate, last_confirmed = nil, nil
end

local function button_entered(event, context)
  if event.behavior.type == "modifier_key" and context.controller.buttons[event.button].pressed then
    set_modifier_button(event, context, true)
  end
end

local function button_pressed(event, context)
  if event.input_mode ~= event.mode then
    return
  end
  if event.behavior.type == "modifier_key" then
    set_modifier_button(event, context, true)
  else
    uppercase_buttons[event.button] = {
      pressed_at_ms = event.time_ms,
      hold_ms = hold_threshold(event.behavior.settings, settings.hold_ms),
    }
  end
end

local function button_released(event, context)
  if event.behavior.type == "modifier_key" then
    set_modifier_button(event, context, false)
    return
  end
  local button = uppercase_buttons[event.button]
  if button then
    uppercase_buttons[event.button] = nil
    uppercase_once = event.time_ms - button.pressed_at_ms < button.hold_ms
      and not caps_held(event.time_ms)
  end
end

local function button_exited(event, context)
  if event.behavior.type == "modifier_key" then
    set_modifier_button(event, context, false)
  else
    uppercase_buttons[event.button] = nil
    uppercase_once = false
  end
end

local function tick(event, context)
  local held = caps_held(event.time_ms)
  if held then
    uppercase_once = false
  end
  observe(event.time_ms, context)
  if candidate and event.time_ms - candidate.started_at_ms >= settings.stable_ms then
    if (caps or held or uppercase_once) and string.is_cased(candidate.text) then
      candidate.text = uppercase(candidate.text)
      uppercase_once = false
    end
    last_confirmed, candidate = candidate, nil
    return emit_text(last_confirmed.text)
  end
end

local function select_keymap(index, context)
  if keymap_index ~= index then
    keymap_index = index
    if not modifier then
      candidate, last_confirmed = nil, nil
    end
    wait_for_center.right = angle(context.controller.sticks.right) and true
  end
end

local function on_action(action, context)
  assert(settings, "нужна секция chord_text")
  local args = action.arguments
  if action.type == "uppercase" then
    assert(#args == 1 and (args[1] == "once" or args[1] == "caps"), "uppercase ожидает once или caps")
    if args[1] == "once" then
      uppercase_once = true
    else
      caps = not caps
    end
  elseif action.type == "select_keymap" then
    assert(#args == 1 and type(args[1]) == "string", "select_keymap ожидает имя таблицы")
    for index, map in ipairs(keymaps) do
      if map.name == args[1] then
        select_keymap(index, context)
        return
      end
    end
    error("неизвестная таблица chord_text: " .. args[1])
  elseif action.type == "next_keymap" or action.type == "previous_keymap" then
    assert(#args == 0, action.type .. " не ожидает аргументы")
    local step = action.type == "next_keymap" and 1 or -1
    select_keymap((keymap_index - 1 + step) % #keymaps + 1, context)
  elseif action.type == "repeat_choice" then
    assert(#args == 0, "repeat_choice не ожидает аргументы")
    if same_choice(read_choice(context), last_confirmed) then
      return emit_text(last_confirmed.text)
    end
  else
    error("неизвестное действие chord_text: " .. action.type)
  end
end

return {
  event_handlers = {
    load = load,
    stick_behavior_entered = stick_entered,
    stick_behavior_exited = stick_exited,
    stick_moved = stick_moved,
    button_behavior_entered = button_entered,
    button_pressed = button_pressed,
    button_unpressed = button_released,
    button_behavior_exited = button_exited,
    controller_disconnected = reset_choice,
    tick = tick,
  },
  on_action = on_action,
}
