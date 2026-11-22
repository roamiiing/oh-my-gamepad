local active = {}
local defaults = {
  pointer = { sensitivity = 1000, acceleration = 1.5 },
  scroll = { sensitivity = 24, acceleration = 1 },
}
local hold_buttons = { left_hold = "left", right_hold = "right" }
local config

local function option(overrides, common, name, fallback)
  if overrides[name] ~= nil then
    return overrides[name]
  end
  if common[name] ~= nil then
    return common[name]
  end
  return fallback
end

local function motion_settings(kind, overrides)
  local common = config[kind] or {}
  local base = defaults[kind]
  local settings = {
    sensitivity = option(overrides, common, "sensitivity", base.sensitivity),
    acceleration = option(overrides, common, "acceleration", base.acceleration),
    dead_zone = option(overrides, common, "dead_zone", 0.25),
    invert_x = option(overrides, common, "invert_x", false),
    invert_y = option(overrides, common, "invert_y", false),
  }
  local speed, acceleration, dead_zone = settings.sensitivity, settings.acceleration, settings.dead_zone
  assert(type(speed) == "number" and speed >= 0 and speed < math.huge,
    "mouse." .. kind .. ".sensitivity должна быть конечным неотрицательным числом")
  assert(type(acceleration) == "number" and acceleration > 0 and acceleration < math.huge,
    "mouse." .. kind .. ".acceleration должно быть конечным положительным числом")
  assert(type(dead_zone) == "number" and dead_zone >= 0 and dead_zone < 1,
    "mouse." .. kind .. ".dead_zone должна быть числом от 0 до 1 (не включая 1)")
  assert(type(settings.invert_x) == "boolean" and type(settings.invert_y) == "boolean",
    "mouse." .. kind .. ": invert_x и invert_y должны быть true или false")
  return settings
end

local function settings_for(behavior)
  local settings = behavior.settings
  assert(settings == nil or type(settings) == "table", "mouse: settings должен быть словарём")
  return settings or {}
end

local function load(_, context)
  config = context.config.mouse
  assert(config == nil or type(config) == "table", "mouse должен быть словарём")
  config = config or {}
  for _, kind in ipairs({ "pointer", "scroll" }) do
    assert(config[kind] == nil or type(config[kind]) == "table", "mouse." .. kind .. " должен быть словарём")
    motion_settings(kind, {})
  end
  for name, mode in pairs(context.config.modes) do
    for _, source in ipairs({ "sticks", "triggers", "buttons" }) do
      for id, mapping in pairs(mode[source] or {}) do
        local behavior = mapping.behavior
        if behavior and behavior.plugin == "mouse" then
          local path = name .. "." .. source .. "." .. id
          if source == "sticks" and defaults[behavior.type] then
            motion_settings(behavior.type, settings_for(behavior))
          elseif not (source == "buttons" and hold_buttons[behavior.type]) then
            error(path .. ": mouse поддерживает pointer и scroll для стиков, left_hold и right_hold для кнопок")
          end
        end
      end
    end
  end
end

local function stick_entered(event)
  local kind = event.behavior.type
  active[event.source.side] = {
    type = kind, settings = motion_settings(kind, settings_for(event.behavior)), scroll_x = 0, scroll_y = 0,
  }
end

local function stick_exited(event)
  active[event.source.side] = nil
end

local function disconnect()
  for _, behavior in pairs(active) do
    behavior.scroll_x, behavior.scroll_y = 0, 0
  end
end

local function mouse_button(event, operation)
  local button = hold_buttons[event.behavior.type]
  -- Нажатие, которым открыли режим мыши, используется только для перехода.
  if button and (operation == "up" or event.input_mode == event.mode) then
    return {{
      plugin = "builtin_primitives", type = "mouse_key",
      arguments = { button, operation },
    }}
  end
end

local function whole(value)
  if value < 0 then
    return math.ceil(value)
  end
  return math.floor(value)
end

local function tick(event, context)
  if not context.controller.connected then
    return
  end

  local actions = {}
  local seconds = event.dt_ms / 1000
  for _, side in ipairs({ "left", "right" }) do
    local behavior = active[side]
    if behavior then
      local stick = context.controller.sticks[side]
      local length = math.sqrt(stick.x * stick.x + stick.y * stick.y)
      local settings = behavior.settings
      if length <= settings.dead_zone then
        behavior.scroll_x, behavior.scroll_y = 0, 0
      else
        local tilt = (math.min(length, 1) - settings.dead_zone) / (1 - settings.dead_zone)
        local speed = tilt ^ settings.acceleration * settings.sensitivity * seconds / length
        local dx, dy = stick.x * speed, stick.y * speed
        dx = settings.invert_x and -dx or dx
        dy = settings.invert_y and -dy or dy
        if behavior.type == "pointer" then
          actions[#actions + 1] = {
            plugin = "builtin_primitives", type = "move_pointer", arguments = { dx, -dy },
          }
        else
          behavior.scroll_x = behavior.scroll_x + dx
          behavior.scroll_y = behavior.scroll_y + dy
          local x, y = whole(behavior.scroll_x), whole(behavior.scroll_y)
          behavior.scroll_x, behavior.scroll_y = behavior.scroll_x - x, behavior.scroll_y - y
          if x ~= 0 or y ~= 0 then
            actions[#actions + 1] = {
              plugin = "builtin_primitives", type = "scroll", arguments = { x, y },
            }
          end
        end
      end
    end
  end
  return actions
end

return {
  event_handlers = {
    load = load,
    stick_behavior_entered = stick_entered,
    stick_behavior_exited = stick_exited,
    button_pressed = function(event) return mouse_button(event, "down") end,
    button_unpressed = function(event) return mouse_button(event, "up") end,
    controller_disconnected = disconnect,
    tick = tick,
  },
}
