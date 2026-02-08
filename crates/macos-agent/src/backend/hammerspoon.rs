use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::backend::process::{ProcessFailure, ProcessRequest, ProcessRunner};
use crate::error::CliError;
use crate::model::{
    AxActionPerformRequest, AxActionPerformResult, AxAttrGetRequest, AxAttrGetResult,
    AxAttrSetRequest, AxAttrSetResult, AxClickRequest, AxClickResult, AxListRequest, AxListResult,
    AxSelector, AxSessionListResult, AxSessionStartRequest, AxSessionStartResult,
    AxSessionStopRequest, AxSessionStopResult, AxTypeRequest, AxTypeResult, AxWatchPollRequest,
    AxWatchPollResult, AxWatchStartRequest, AxWatchStartResult, AxWatchStopRequest,
    AxWatchStopResult,
};
use crate::test_mode;

use super::AxBackendAdapter;

const AX_LIST_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_LIST_JSON";
const AX_CLICK_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_CLICK_JSON";
const AX_TYPE_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_TYPE_JSON";
const AX_ATTR_GET_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_ATTR_GET_JSON";
const AX_ATTR_SET_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_ATTR_SET_JSON";
const AX_ACTION_PERFORM_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_ACTION_PERFORM_JSON";
const AX_SESSION_START_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_SESSION_START_JSON";
const AX_SESSION_LIST_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_SESSION_LIST_JSON";
const AX_SESSION_STOP_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_SESSION_STOP_JSON";
const AX_WATCH_START_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_WATCH_START_JSON";
const AX_WATCH_POLL_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_WATCH_POLL_JSON";
const AX_WATCH_STOP_TEST_MODE_ENV: &str = "CODEX_MACOS_AGENT_AX_WATCH_STOP_JSON";
const BACKEND_UNAVAILABLE_HINT_PREFIX: &str = "Hammerspoon backend unavailable";

macro_rules! hs_ax_script_with_targeting_prelude {
    ($operation:literal, $body:literal) => {
        concat!(
            r#"
local json = hs.json
local appmod = hs.application
local ax = hs.axuielement

local function fail(message)
  error(message, 0)
end

local function safe(callable, fallback)
  local ok, value = pcall(callable)
  if ok then return value end
  return fallback
end

local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end

local function asTable(value)
  if type(value) == "table" then return value end
  return {}
end

local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end

local function resolveTarget(rawTarget)
  local target = rawTarget or {}
  local state = ensureState()
  if target.session_id and tostring(target.session_id) ~= "" then
    local session = state.sessions[tostring(target.session_id)]
    if not session then
      fail("session_id does not exist")
    end
    return {
      session_id = tostring(target.session_id),
      app = target.app or session.app,
      bundle_id = target.bundle_id or session.bundle_id,
      pid = session.pid,
      window_title_contains = target.window_title_contains or session.window_title_contains,
    }
  end
  return target
end

local function attr(element, name, fallback)
  local value = safe(function() return element:attributeValue(name) end, fallback)
  if value == nil then return fallback end
  return value
end

local function boolAttr(element, name, fallback)
  local value = attr(element, name, fallback)
  if type(value) == "boolean" then return value end
  if value == nil then return fallback end
  return tostring(value):lower() == "true"
end

local function children(element)
  return asTable(attr(element, "AXChildren", {}))
end

local function resolveApp(target)
  target = resolveTarget(target)

  if target.pid then
    local byPid = appmod.applicationForPID(tonumber(target.pid))
    if byPid then return byPid, target end
  end

  if target.app and tostring(target.app) ~= "" then
    local found = appmod.find(tostring(target.app))
    if found then return found, target end
  end

  if target.bundle_id and tostring(target.bundle_id) ~= "" then
    local apps = appmod.applicationsForBundleID(tostring(target.bundle_id))
    if type(apps) == "table" and #apps > 0 then
      return apps[1], target
    end
  end

  return appmod.frontmostApplication(), target
end

local function rootsForApp(app, target)
  local appElement = ax.applicationElement(app)
  if not appElement then
    fail("unable to resolve target app process for "#,
            $operation,
            r#")
  end

  local roots = asTable(attr(appElement, "AXWindows", {}))
  if #roots == 0 then
    roots = children(appElement)
  end
  local windowFilter = target and target.window_title_contains and string.lower(tostring(target.window_title_contains)) or nil
  if not windowFilter then
    return roots
  end

  local filtered = {}
  for _, root in ipairs(roots) do
    local title = string.lower(normalize(attr(root, "AXTitle", "")))
    if string.find(title, windowFilter, 1, true) then
      table.insert(filtered, root)
    end
  end
  return filtered
end

local function copyPath(path)
  local out = {}
  for i, value in ipairs(path) do
    out[i] = value
  end
  return out
end

"#,
            $body
        )
    };
}

const AX_LIST_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.list",
    r#"
local function frameFor(element)
  local pos = attr(element, "AXPosition", nil)
  local size = attr(element, "AXSize", nil)
  if type(pos) ~= "table" or type(size) ~= "table" then return nil end

  local x = tonumber(pos.x or pos[1])
  local y = tonumber(pos.y or pos[2])
  local width = tonumber(size.w or size.width or size[1])
  local height = tonumber(size.h or size.height or size[2])

  if not x or not y or not width or not height then return nil end
  return { x = x, y = y, width = width, height = height }
end

local function actionNames(element)
  local raw = safe(function() return element:actionNames() end, nil)
  if type(raw) ~= "table" then return {} end
  local out = {}
  for _, name in ipairs(raw) do
    local text = normalize(name)
    if text ~= "" then table.insert(out, text) end
  end
  return out
end

local function valuePreview(element)
  local value = attr(element, "AXValue", nil)
  if value == nil then return nil end
  local text = normalize(value)
  if #text > 160 then
    text = string.sub(text, 1, 160) .. "..."
  end
  return text
end

local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then
    fail("invalid payload JSON")
  end
  return payload
end

local payload = parsePayload()
local roleFilter = payload.role and string.lower(tostring(payload.role)) or nil
local titleFilter = payload.title_contains and string.lower(tostring(payload.title_contains)) or nil
local identifierFilter = payload.identifier_contains and string.lower(tostring(payload.identifier_contains)) or nil
local valueFilter = payload.value_contains and string.lower(tostring(payload.value_contains)) or nil
local subroleFilter = payload.subrole and string.lower(tostring(payload.subrole)) or nil
local maxDepth = payload.max_depth and tonumber(payload.max_depth) or nil
local limit = payload.limit and tonumber(payload.limit) or nil
local focusedFilter = payload.focused
local enabledFilter = payload.enabled

local app, resolvedTarget = resolveApp(payload.target)
if not app then
  fail("unable to resolve target app process for ax.list")
end

local roots = rootsForApp(app, resolvedTarget)
local nodes = {}

local function nodeFrom(element, path)
  local role = normalize(attr(element, "AXRole", ""))
  local title = normalize(attr(element, "AXTitle", ""))
  local identifier = normalize(attr(element, "AXIdentifier", ""))
  local subrole = normalize(attr(element, "AXSubrole", ""))

  local node = {
    node_id = table.concat(path, "."),
    role = role,
    subrole = subrole ~= "" and subrole or nil,
    title = title ~= "" and title or nil,
    identifier = identifier ~= "" and identifier or nil,
    value_preview = valuePreview(element),
    enabled = boolAttr(element, "AXEnabled", true),
    focused = boolAttr(element, "AXFocused", false),
    frame = frameFor(element),
    actions = actionNames(element),
    path = copyPath(path),
  }

  return node
end

local function matches(node)
  if roleFilter and string.lower(node.role or "") ~= roleFilter then
    return false
  end

  if titleFilter then
    local title = string.lower(node.title or "")
    local identifier = string.lower(node.identifier or "")
    if not string.find(title, titleFilter, 1, true) and not string.find(identifier, titleFilter, 1, true) then
      return false
    end
  end

  if identifierFilter and not string.find(string.lower(node.identifier or ""), identifierFilter, 1, true) then
    return false
  end

  if valueFilter and not string.find(string.lower(node.value_preview or ""), valueFilter, 1, true) then
    return false
  end

  if subroleFilter and string.lower(node.subrole or "") ~= subroleFilter then
    return false
  end

  if focusedFilter ~= nil and node.focused ~= focusedFilter then
    return false
  end

  if enabledFilter ~= nil and node.enabled ~= enabledFilter then
    return false
  end

  return true
end

local function visit(element, path, depth)
  if limit and #nodes >= limit then
    return
  end

  local node = nodeFrom(element, path)
  if matches(node) then
    table.insert(nodes, node)
  end

  if maxDepth and depth >= maxDepth then
    return
  end

  for index, child in ipairs(children(element)) do
    local childPath = copyPath(path)
    table.insert(childPath, tostring(index))
    visit(child, childPath, depth + 1)
    if limit and #nodes >= limit then
      return
    end
  end
end

for rootIndex, root in ipairs(roots) do
  visit(root, { tostring(rootIndex) }, 0)
  if limit and #nodes >= limit then
    break
  end
end

return json.encode({ nodes = nodes, warnings = {} })
"#
);

const AX_CLICK_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.click",
    r#"
local function nodeFrom(element, path)
  local role = normalize(attr(element, "AXRole", ""))
  local title = normalize(attr(element, "AXTitle", ""))
  local identifier = normalize(attr(element, "AXIdentifier", ""))
  local subrole = normalize(attr(element, "AXSubrole", ""))
  local value = normalize(attr(element, "AXValue", ""))
  local focused = normalize(attr(element, "AXFocused", "false"))
  local enabled = normalize(attr(element, "AXEnabled", "true"))
  return {
    node_id = table.concat(path, "."),
    role = role,
    title = title,
    identifier = identifier,
    subrole = subrole,
    value_preview = value,
    focused = string.lower(focused) == "true",
    enabled = string.lower(enabled) == "true",
  }
end

local function frameCenter(element)
  local pos = attr(element, "AXPosition", nil)
  local size = attr(element, "AXSize", nil)
  if type(pos) ~= "table" or type(size) ~= "table" then return nil end

  local x = tonumber(pos.x or pos[1])
  local y = tonumber(pos.y or pos[2])
  local width = tonumber(size.w or size.width or size[1])
  local height = tonumber(size.h or size.height or size[2])

  if not x or not y or not width or not height then return nil end
  return {
    x = math.floor(x + width / 2),
    y = math.floor(y + height / 2),
  }
end

local function resolveByNodeId(roots, nodeId)
  local parts = {}
  for segment in string.gmatch(tostring(nodeId), "[^.]+") do
    table.insert(parts, tonumber(segment))
  end
  if #parts == 0 then return nil end

  local rootIndex = parts[1]
  if not rootIndex or rootIndex < 1 or rootIndex > #roots then
    return nil
  end

  local element = roots[rootIndex]
  local path = { tostring(rootIndex) }

  for i = 2, #parts do
    local childIndex = parts[i]
    local directChildren = children(element)
    if not childIndex or childIndex < 1 or childIndex > #directChildren then
      return nil
    end
    element = directChildren[childIndex]
    table.insert(path, tostring(childIndex))
  end

  return {
    element = element,
    node = nodeFrom(element, path),
  }
end

local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then
    fail("invalid payload JSON")
  end
  return payload
end

local payload = parsePayload()
local selector = payload.selector or {}
local roleFilter = selector.role and string.lower(tostring(selector.role)) or nil
local titleFilter = selector.title_contains and string.lower(tostring(selector.title_contains)) or nil
local identifierFilter = selector.identifier_contains and string.lower(tostring(selector.identifier_contains)) or nil
local valueFilter = selector.value_contains and string.lower(tostring(selector.value_contains)) or nil
local subroleFilter = selector.subrole and string.lower(tostring(selector.subrole)) or nil
local focusedFilter = selector.focused
local enabledFilter = selector.enabled
local nth = selector.nth and tonumber(selector.nth) or nil
local allowCoordinateFallback = payload.allow_coordinate_fallback and true or false

local app, resolvedTarget = resolveApp(payload.target)
if not app then
  fail("unable to resolve target app process for ax.click")
end

local roots = rootsForApp(app, resolvedTarget)
local matches = {}

if selector.node_id then
  local byId = resolveByNodeId(roots, selector.node_id)
  if byId then
    table.insert(matches, byId)
  end
else
  local function walk(element, path)
    local node = nodeFrom(element, path)
    local roleMatch = (not roleFilter) or (string.lower(node.role or "") == roleFilter)
    local title = string.lower(node.title or "")
    local identifier = string.lower(node.identifier or "")
    local value = string.lower(node.value_preview or "")
    local subrole = string.lower(node.subrole or "")
    local titleMatch = (not titleFilter) or string.find(title, titleFilter, 1, true) or string.find(identifier, titleFilter, 1, true)
    local identifierMatch = (not identifierFilter) or string.find(identifier, identifierFilter, 1, true)
    local valueMatch = (not valueFilter) or string.find(value, valueFilter, 1, true)
    local subroleMatch = (not subroleFilter) or subrole == subroleFilter
    local focusedMatch = (focusedFilter == nil) or (node.focused == focusedFilter)
    local enabledMatch = (enabledFilter == nil) or (node.enabled == enabledFilter)
    if roleMatch and titleMatch and identifierMatch and valueMatch and subroleMatch and focusedMatch and enabledMatch then
      table.insert(matches, { element = element, node = node })
    end

    for index, child in ipairs(children(element)) do
      local childPath = copyPath(path)
      table.insert(childPath, tostring(index))
      walk(child, childPath)
    end
  end

  for rootIndex, root in ipairs(roots) do
    walk(root, { tostring(rootIndex) })
  end
end

if #matches == 0 then
  fail("selector returned zero AX matches")
end

local selected
if selector.node_id then
  selected = matches[1]
elseif nth then
  if nth < 1 or nth > #matches then
    fail("selector nth is out of range")
  end
  selected = matches[nth]
else
  if #matches ~= 1 then
    fail("selector is ambiguous; add --nth or narrow role/title filters")
  end
  selected = matches[1]
end

local actions = asTable(safe(function() return selected.element:actionNames() end, {}))
local actionToRun = nil
for _, name in ipairs(actions) do
  local value = normalize(name)
  if value == "AXPress" or value == "AXConfirm" then
    actionToRun = value
    break
  end
end

local result = {
  node_id = selected.node.node_id,
  matched_count = #matches,
  action = "ax-press",
  used_coordinate_fallback = false,
}

local performOk = false
if actionToRun then
  performOk = safe(function()
    selected.element:performAction(actionToRun)
    return true
  end, false)
end

if not performOk then
  if not allowCoordinateFallback then
    fail("AXPress action unavailable")
  end
  local center = frameCenter(selected.element)
  if not center then
    fail("coordinate fallback requested but AXPosition/AXSize unavailable")
  end
  result.action = "ax-press-fallback"
  result.used_coordinate_fallback = true
  result.fallback_x = center.x
  result.fallback_y = center.y
end

return json.encode(result)
"#
);

const AX_TYPE_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.type",
    r#"
local eventtap = hs.eventtap
local pasteboard = hs.pasteboard

local function nodeFrom(element, path)
  local role = normalize(attr(element, "AXRole", ""))
  local title = normalize(attr(element, "AXTitle", ""))
  local identifier = normalize(attr(element, "AXIdentifier", ""))
  local subrole = normalize(attr(element, "AXSubrole", ""))
  local value = normalize(attr(element, "AXValue", ""))
  return {
    node_id = table.concat(path, "."),
    role = role,
    title = title,
    identifier = identifier,
    subrole = subrole,
    value_preview = value,
    focused = boolAttr(element, "AXFocused", false),
    enabled = boolAttr(element, "AXEnabled", true),
  }
end

local function resolveByNodeId(roots, nodeId)
  local parts = {}
  for segment in string.gmatch(tostring(nodeId), "[^.]+") do
    table.insert(parts, tonumber(segment))
  end
  if #parts == 0 then return nil end

  local rootIndex = parts[1]
  if not rootIndex or rootIndex < 1 or rootIndex > #roots then
    return nil
  end

  local element = roots[rootIndex]
  local path = { tostring(rootIndex) }

  for i = 2, #parts do
    local childIndex = parts[i]
    local directChildren = children(element)
    if not childIndex or childIndex < 1 or childIndex > #directChildren then
      return nil
    end
    element = directChildren[childIndex]
    table.insert(path, tostring(childIndex))
  end

  return {
    element = element,
    node = nodeFrom(element, path),
  }
end

local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then
    fail("invalid payload JSON")
  end
  return payload
end

local payload = parsePayload()
local selector = payload.selector or {}
local roleFilter = selector.role and string.lower(tostring(selector.role)) or nil
local titleFilter = selector.title_contains and string.lower(tostring(selector.title_contains)) or nil
local identifierFilter = selector.identifier_contains and string.lower(tostring(selector.identifier_contains)) or nil
local valueFilter = selector.value_contains and string.lower(tostring(selector.value_contains)) or nil
local subroleFilter = selector.subrole and string.lower(tostring(selector.subrole)) or nil
local focusedFilter = selector.focused
local enabledFilter = selector.enabled
local nth = selector.nth and tonumber(selector.nth) or nil
local text = payload.text and tostring(payload.text) or ""
local allowKeyboardFallback = payload.allow_keyboard_fallback and true or false
local clearFirst = payload.clear_first and true or false
local submit = payload.submit and true or false
local paste = payload.paste and true or false

if text == "" then
  fail("text cannot be empty")
end

local app, resolvedTarget = resolveApp(payload.target)
if not app then
  fail("unable to resolve target app process for ax.type")
end
safe(function() app:activate() end, nil)

local roots = rootsForApp(app, resolvedTarget)
local matches = {}

if selector.node_id then
  local byId = resolveByNodeId(roots, selector.node_id)
  if byId then
    table.insert(matches, byId)
  end
else
  local function walk(element, path)
    local node = nodeFrom(element, path)
    local roleMatch = (not roleFilter) or (string.lower(node.role or "") == roleFilter)
    local title = string.lower(node.title or "")
    local identifier = string.lower(node.identifier or "")
    local value = string.lower(node.value_preview or "")
    local subrole = string.lower(node.subrole or "")
    local titleMatch = (not titleFilter) or string.find(title, titleFilter, 1, true) or string.find(identifier, titleFilter, 1, true)
    local identifierMatch = (not identifierFilter) or string.find(identifier, identifierFilter, 1, true)
    local valueMatch = (not valueFilter) or string.find(value, valueFilter, 1, true)
    local subroleMatch = (not subroleFilter) or subrole == subroleFilter
    local focusedMatch = (focusedFilter == nil) or (node.focused == focusedFilter)
    local enabledMatch = (enabledFilter == nil) or (node.enabled == enabledFilter)
    if roleMatch and titleMatch and identifierMatch and valueMatch and subroleMatch and focusedMatch and enabledMatch then
      table.insert(matches, { element = element, node = node })
    end

    for index, child in ipairs(children(element)) do
      local childPath = copyPath(path)
      table.insert(childPath, tostring(index))
      walk(child, childPath)
    end
  end

  for rootIndex, root in ipairs(roots) do
    walk(root, { tostring(rootIndex) })
  end
end

if #matches == 0 then
  fail("selector returned zero AX matches")
end

local selected
if selector.node_id then
  selected = matches[1]
elseif nth then
  if nth < 1 or nth > #matches then
    fail("selector nth is out of range")
  end
  selected = matches[nth]
else
  if #matches ~= 1 then
    fail("selector is ambiguous; add --nth or narrow selector filters")
  end
  selected = matches[1]
end

local appliedVia = "ax-set-value"
local usedKeyboardFallback = false

local function applyPaste()
  pasteboard.setContents(text)
  eventtap.keyStroke({"cmd"}, "v", 0)
end

local appliedOk = safe(function()
  safe(function() selected.element:setAttributeValue("AXFocused", true) end, nil)
  if clearFirst then
    safe(function() selected.element:setAttributeValue("AXValue", "") end, nil)
  end
  if paste then
    applyPaste()
    appliedVia = "ax-paste"
  else
    selected.element:setAttributeValue("AXValue", text)
    appliedVia = "ax-set-value"
  end
  return true
end, false)

if not appliedOk then
  if not allowKeyboardFallback then
    fail("AX value set failed")
  end

  usedKeyboardFallback = true
  if paste then
    applyPaste()
    appliedVia = "keyboard-paste-fallback"
  else
    eventtap.keyStrokes(text)
    appliedVia = "keyboard-keystroke-fallback"
  end
end

if submit then
  eventtap.keyStroke({}, "return", 0)
end

return json.encode({
  node_id = selected.node.node_id,
  matched_count = #matches,
  applied_via = appliedVia,
  text_length = string.len(text),
  submitted = submit,
  used_keyboard_fallback = usedKeyboardFallback,
})
"#
);

const AX_ATTR_GET_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.attr.get",
    r#"
local function resolveApp(target)
  target = resolveTarget(target)

  if target.pid then
    local byPid = appmod.applicationForPID(tonumber(target.pid))
    if byPid then return byPid end
  end

  if target.app and tostring(target.app) ~= "" then
    local found = appmod.find(tostring(target.app))
    if found then return found, target end
  end

  if target.bundle_id and tostring(target.bundle_id) ~= "" then
    local apps = appmod.applicationsForBundleID(tostring(target.bundle_id))
    if type(apps) == "table" and #apps > 0 then
      return apps[1], target
    end
  end

  return appmod.frontmostApplication(), target
end

local function nodeFrom(element, path)
  local role = normalize(attr(element, "AXRole", ""))
  local title = normalize(attr(element, "AXTitle", ""))
  local identifier = normalize(attr(element, "AXIdentifier", ""))
  local subrole = normalize(attr(element, "AXSubrole", ""))
  local value = normalize(attr(element, "AXValue", ""))
  return {
    node_id = table.concat(path, "."),
    role = role,
    title = title,
    identifier = identifier,
    subrole = subrole,
    value_preview = value,
    focused = boolAttr(element, "AXFocused", false),
    enabled = boolAttr(element, "AXEnabled", true),
  }
end

local function matches(node, selector)
  selector = selector or {}

  if selector.role and string.lower(node.role or "") ~= string.lower(tostring(selector.role)) then
    return false
  end
  if selector.title_contains and not string.find(string.lower(node.title or ""), string.lower(tostring(selector.title_contains)), 1, true) then
    return false
  end
  if selector.identifier_contains and not string.find(string.lower(node.identifier or ""), string.lower(tostring(selector.identifier_contains)), 1, true) then
    return false
  end
  if selector.value_contains and not string.find(string.lower(node.value_preview or ""), string.lower(tostring(selector.value_contains)), 1, true) then
    return false
  end
  if selector.subrole and string.lower(node.subrole or "") ~= string.lower(tostring(selector.subrole)) then
    return false
  end
  if selector.focused ~= nil and node.focused ~= selector.focused then
    return false
  end
  if selector.enabled ~= nil and node.enabled ~= selector.enabled then
    return false
  end
  return true
end

local function resolveByNodeId(roots, nodeId)
  local parts = {}
  for segment in string.gmatch(tostring(nodeId), "[^.]+") do
    table.insert(parts, tonumber(segment))
  end
  if #parts == 0 then return nil end

  local rootIndex = parts[1]
  if not rootIndex or rootIndex < 1 or rootIndex > #roots then
    return nil
  end

  local element = roots[rootIndex]
  local path = { tostring(rootIndex) }
  for i = 2, #parts do
    local childIndex = parts[i]
    local directChildren = children(element)
    if not childIndex or childIndex < 1 or childIndex > #directChildren then
      return nil
    end
    element = directChildren[childIndex]
    table.insert(path, tostring(childIndex))
  end
  return { element = element, node = nodeFrom(element, path) }
end

local function collectMatches(roots, selector)
  local matchesOut = {}

  if selector.node_id then
    local byId = resolveByNodeId(roots, selector.node_id)
    if byId then
      table.insert(matchesOut, byId)
    end
    return matchesOut
  end

  local function walk(element, path)
    local node = nodeFrom(element, path)
    if matches(node, selector) then
      table.insert(matchesOut, { element = element, node = node })
    end
    for index, child in ipairs(children(element)) do
      local childPath = copyPath(path)
      table.insert(childPath, tostring(index))
      walk(child, childPath)
    end
  end

  for rootIndex, root in ipairs(roots) do
    walk(root, { tostring(rootIndex) })
  end
  return matchesOut
end

local function selectOne(matchesOut, selector)
  if #matchesOut == 0 then
    fail("selector returned zero AX matches")
  end

  if selector.node_id then
    return matchesOut[1], #matchesOut
  end

  local nth = selector.nth and tonumber(selector.nth) or nil
  if nth then
    if nth < 1 or nth > #matchesOut then
      fail("selector nth is out of range")
    end
    return matchesOut[nth], #matchesOut
  end

  if #matchesOut ~= 1 then
    fail("selector is ambiguous; add --nth or narrow selector filters")
  end

  return matchesOut[1], #matchesOut
end

local function sanitize(value, depth)
  depth = depth or 0
  if depth > 6 then
    return tostring(value)
  end

  local kind = type(value)
  if kind == "nil" then
    return json.null
  end
  if kind == "string" or kind == "number" or kind == "boolean" then
    return value
  end
  if kind == "table" then
    local out = {}
    local count = 0
    local maxIndex = 0
    local isArray = true
    for key, _ in pairs(value) do
      count = count + 1
      if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then
        isArray = false
        break
      end
      if key > maxIndex then maxIndex = key end
    end
    if isArray and maxIndex ~= count then
      isArray = false
    end
    if isArray then
      for i = 1, maxIndex do
        out[i] = sanitize(value[i], depth + 1)
      end
    else
      for key, child in pairs(value) do
        out[tostring(key)] = sanitize(child, depth + 1)
      end
    end
    return out
  end
  return tostring(value)
end

local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then
    fail("invalid payload JSON")
  end
  return payload
end

local payload = parsePayload()
local selector = payload.selector or {}
local app, target = resolveApp(payload.target)
if not app then
  fail("unable to resolve target app process for ax.attr.get")
end

local roots = rootsForApp(app, target)
local matchesOut = collectMatches(roots, selector)
local selected, matchedCount = selectOne(matchesOut, selector)
local name = normalize(payload.name)
if name == "" then
  fail("attribute name cannot be empty")
end
local value = sanitize(attr(selected.element, name, nil), 0)

return json.encode({
  node_id = selected.node.node_id,
  matched_count = matchedCount,
  name = name,
  value = value,
})
"#
);

const AX_ATTR_SET_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.attr.set",
    r#"
local function nodeFrom(element, path)
  return {
    node_id = table.concat(path, "."),
    role = normalize(attr(element, "AXRole", "")),
    title = normalize(attr(element, "AXTitle", "")),
    identifier = normalize(attr(element, "AXIdentifier", "")),
    subrole = normalize(attr(element, "AXSubrole", "")),
    value_preview = normalize(attr(element, "AXValue", "")),
    focused = boolAttr(element, "AXFocused", false),
    enabled = boolAttr(element, "AXEnabled", true),
  }
end

local function matches(node, selector)
  selector = selector or {}
  if selector.role and string.lower(node.role or "") ~= string.lower(tostring(selector.role)) then return false end
  if selector.title_contains and not string.find(string.lower(node.title or ""), string.lower(tostring(selector.title_contains)), 1, true) then return false end
  if selector.identifier_contains and not string.find(string.lower(node.identifier or ""), string.lower(tostring(selector.identifier_contains)), 1, true) then return false end
  if selector.value_contains and not string.find(string.lower(node.value_preview or ""), string.lower(tostring(selector.value_contains)), 1, true) then return false end
  if selector.subrole and string.lower(node.subrole or "") ~= string.lower(tostring(selector.subrole)) then return false end
  if selector.focused ~= nil and node.focused ~= selector.focused then return false end
  if selector.enabled ~= nil and node.enabled ~= selector.enabled then return false end
  return true
end

local function resolveByNodeId(roots, nodeId)
  local parts = {}
  for segment in string.gmatch(tostring(nodeId), "[^.]+") do table.insert(parts, tonumber(segment)) end
  if #parts == 0 then return nil end

  local rootIndex = parts[1]
  if not rootIndex or rootIndex < 1 or rootIndex > #roots then return nil end

  local element = roots[rootIndex]
  local path = { tostring(rootIndex) }
  for i = 2, #parts do
    local childIndex = parts[i]
    local directChildren = children(element)
    if not childIndex or childIndex < 1 or childIndex > #directChildren then return nil end
    element = directChildren[childIndex]
    table.insert(path, tostring(childIndex))
  end
  return { element = element, node = nodeFrom(element, path) }
end

local function collectMatches(roots, selector)
  local matchesOut = {}

  if selector.node_id then
    local byId = resolveByNodeId(roots, selector.node_id)
    if byId then table.insert(matchesOut, byId) end
    return matchesOut
  end

  local function walk(element, path)
    local node = nodeFrom(element, path)
    if matches(node, selector) then table.insert(matchesOut, { element = element, node = node }) end
    for index, child in ipairs(children(element)) do
      local childPath = copyPath(path)
      table.insert(childPath, tostring(index))
      walk(child, childPath)
    end
  end

  for rootIndex, root in ipairs(roots) do
    walk(root, { tostring(rootIndex) })
  end
  return matchesOut
end

local function selectOne(matchesOut, selector)
  if #matchesOut == 0 then fail("selector returned zero AX matches") end
  if selector.node_id then return matchesOut[1], #matchesOut end

  local nth = selector.nth and tonumber(selector.nth) or nil
  if nth then
    if nth < 1 or nth > #matchesOut then fail("selector nth is out of range") end
    return matchesOut[nth], #matchesOut
  end
  if #matchesOut ~= 1 then fail("selector is ambiguous; add --nth or narrow selector filters") end
  return matchesOut[1], #matchesOut
end

local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end

local payload = parsePayload()
local name = normalize(payload.name)
if name == "" then fail("attribute name cannot be empty") end

local app, target = resolveApp(payload.target)
if not app then fail("unable to resolve target app process for ax.attr.set") end

local roots = rootsForApp(app, target)
local matchesOut = collectMatches(roots, payload.selector or {})
local selected, matchedCount = selectOne(matchesOut, payload.selector or {})

local applied = safe(function()
  selected.element:setAttributeValue(name, payload.value)
  return true
end, false)
if not applied then
  fail("failed to set AX attribute value")
end

local valueType = type(payload.value)
if payload.value == json.null then
  valueType = "null"
end

return json.encode({
  node_id = selected.node.node_id,
  matched_count = matchedCount,
  name = name,
  applied = true,
  value_type = valueType,
})
"#
);

const AX_ACTION_PERFORM_HS_SCRIPT: &str = hs_ax_script_with_targeting_prelude!(
    "ax.action.perform",
    r#"
local function nodeFrom(element, path)
  return {
    node_id = table.concat(path, "."),
    role = normalize(attr(element, "AXRole", "")),
    title = normalize(attr(element, "AXTitle", "")),
    identifier = normalize(attr(element, "AXIdentifier", "")),
    subrole = normalize(attr(element, "AXSubrole", "")),
    value_preview = normalize(attr(element, "AXValue", "")),
    focused = boolAttr(element, "AXFocused", false),
    enabled = boolAttr(element, "AXEnabled", true),
  }
end
local function matches(node, selector)
  selector = selector or {}
  if selector.role and string.lower(node.role or "") ~= string.lower(tostring(selector.role)) then return false end
  if selector.title_contains and not string.find(string.lower(node.title or ""), string.lower(tostring(selector.title_contains)), 1, true) then return false end
  if selector.identifier_contains and not string.find(string.lower(node.identifier or ""), string.lower(tostring(selector.identifier_contains)), 1, true) then return false end
  if selector.value_contains and not string.find(string.lower(node.value_preview or ""), string.lower(tostring(selector.value_contains)), 1, true) then return false end
  if selector.subrole and string.lower(node.subrole or "") ~= string.lower(tostring(selector.subrole)) then return false end
  if selector.focused ~= nil and node.focused ~= selector.focused then return false end
  if selector.enabled ~= nil and node.enabled ~= selector.enabled then return false end
  return true
end
local function resolveByNodeId(roots, nodeId)
  local parts = {}
  for segment in string.gmatch(tostring(nodeId), "[^.]+") do table.insert(parts, tonumber(segment)) end
  if #parts == 0 then return nil end
  local rootIndex = parts[1]
  if not rootIndex or rootIndex < 1 or rootIndex > #roots then return nil end
  local element = roots[rootIndex]
  local path = { tostring(rootIndex) }
  for i = 2, #parts do
    local childIndex = parts[i]
    local directChildren = children(element)
    if not childIndex or childIndex < 1 or childIndex > #directChildren then return nil end
    element = directChildren[childIndex]
    table.insert(path, tostring(childIndex))
  end
  return { element = element, node = nodeFrom(element, path) }
end
local function collectMatches(roots, selector)
  local matchesOut = {}
  if selector.node_id then
    local byId = resolveByNodeId(roots, selector.node_id)
    if byId then table.insert(matchesOut, byId) end
    return matchesOut
  end
  local function walk(element, path)
    local node = nodeFrom(element, path)
    if matches(node, selector) then table.insert(matchesOut, { element = element, node = node }) end
    for index, child in ipairs(children(element)) do
      local childPath = copyPath(path)
      table.insert(childPath, tostring(index))
      walk(child, childPath)
    end
  end
  for rootIndex, root in ipairs(roots) do walk(root, { tostring(rootIndex) }) end
  return matchesOut
end
local function selectOne(matchesOut, selector)
  if #matchesOut == 0 then fail("selector returned zero AX matches") end
  if selector.node_id then return matchesOut[1], #matchesOut end
  local nth = selector.nth and tonumber(selector.nth) or nil
  if nth then
    if nth < 1 or nth > #matchesOut then fail("selector nth is out of range") end
    return matchesOut[nth], #matchesOut
  end
  if #matchesOut ~= 1 then fail("selector is ambiguous; add --nth or narrow selector filters") end
  return matchesOut[1], #matchesOut
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end

local payload = parsePayload()
local name = normalize(payload.name)
if name == "" then fail("action name cannot be empty") end

local app, target = resolveApp(payload.target)
if not app then fail("unable to resolve target app process for ax.action.perform") end

local roots = rootsForApp(app, target)
local matchesOut = collectMatches(roots, payload.selector or {})
local selected, matchedCount = selectOne(matchesOut, payload.selector or {})
local performed = safe(function()
  selected.element:performAction(name)
  return true
end, false)
if not performed then fail("failed to perform AX action") end

return json.encode({
  node_id = selected.node.node_id,
  matched_count = matchedCount,
  name = name,
  performed = true,
})
"#
);

const AX_SESSION_START_HS_SCRIPT: &str = r#"
local json = hs.json
local appmod = hs.application
local timer = hs.timer

local function fail(message) error(message, 0) end
local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end
local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end
local function nowMs()
  return math.floor((timer.secondsSinceEpoch and timer.secondsSinceEpoch() or os.time()) * 1000)
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end
local function resolveApp(target)
  target = target or {}
  if target.app and normalize(target.app) ~= "" then
    local found = appmod.find(normalize(target.app))
    if found then return found end
  end
  if target.bundle_id and normalize(target.bundle_id) ~= "" then
    local apps = appmod.applicationsForBundleID(normalize(target.bundle_id))
    if type(apps) == "table" and #apps > 0 then return apps[1] end
  end
  return appmod.frontmostApplication()
end
local function generateSessionId()
  return string.format("axs-%d-%d", os.time(), math.random(1000, 999999))
end

local payload = parsePayload()
local target = payload.target or {}
local app = resolveApp(target)
if not app then fail("unable to resolve target app process for ax.session.start") end

local state = ensureState()
local requestedId = normalize(payload.session_id)
if requestedId == "" then requestedId = normalize(target.session_id) end
if requestedId == "" then requestedId = generateSessionId() end
local existing = state.sessions[requestedId]

local createdAt = existing and existing.created_at_ms or nowMs()
local info = {
  session_id = requestedId,
  app = normalize(target.app) ~= "" and normalize(target.app) or app:name(),
  bundle_id = normalize(target.bundle_id) ~= "" and normalize(target.bundle_id) or app:bundleID(),
  pid = app:pid(),
  window_title_contains = target.window_title_contains and tostring(target.window_title_contains) or nil,
  created_at_ms = createdAt,
}
state.sessions[requestedId] = info

return json.encode({
  session_id = info.session_id,
  app = info.app,
  bundle_id = info.bundle_id,
  pid = info.pid,
  window_title_contains = info.window_title_contains,
  created_at_ms = info.created_at_ms,
  created = existing == nil,
})
"#;

const AX_SESSION_LIST_HS_SCRIPT: &str = r#"
local json = hs.json

local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end

local state = ensureState()
local sessions = {}
for _, session in pairs(state.sessions) do
  table.insert(sessions, {
    session_id = session.session_id,
    app = session.app,
    bundle_id = session.bundle_id,
    pid = session.pid,
    window_title_contains = session.window_title_contains,
    created_at_ms = session.created_at_ms or 0,
  })
end
table.sort(sessions, function(a, b) return (a.session_id or "") < (b.session_id or "") end)
return json.encode({ sessions = sessions })
"#;

const AX_SESSION_STOP_HS_SCRIPT: &str = r#"
local json = hs.json

local function fail(message) error(message, 0) end
local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end
local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end

local payload = parsePayload()
local sessionId = normalize(payload.session_id)
if sessionId == "" then fail("session_id cannot be empty") end

local state = ensureState()
local removed = state.sessions[sessionId] ~= nil
state.sessions[sessionId] = nil

for watchId, slot in pairs(state.watchers) do
  if slot and slot.session_id == sessionId then
    if slot.observer and slot.observer.stop then pcall(function() slot.observer:stop() end) end
    state.watchers[watchId] = nil
  end
end

return json.encode({
  session_id = sessionId,
  removed = removed,
})
"#;

const AX_WATCH_START_HS_SCRIPT: &str = r#"
local json = hs.json
local appmod = hs.application
local ax = hs.axuielement
local observermod = hs.axuielement and hs.axuielement.observer or nil
local timer = hs.timer

local function fail(message) error(message, 0) end
local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end
local function asTable(value)
  if type(value) == "table" then return value end
  return {}
end
local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end
local function nowMs()
  return math.floor((timer.secondsSinceEpoch and timer.secondsSinceEpoch() or os.time()) * 1000)
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end
local function generateWatchId()
  return string.format("axw-%d-%d", os.time(), math.random(1000, 999999))
end
local function resolveAppFromSession(session)
  if session.pid then
    local byPid = appmod.applicationForPID(tonumber(session.pid))
    if byPid then return byPid end
  end
  if session.app and normalize(session.app) ~= "" then
    local byName = appmod.find(normalize(session.app))
    if byName then return byName end
  end
  if session.bundle_id and normalize(session.bundle_id) ~= "" then
    local apps = appmod.applicationsForBundleID(normalize(session.bundle_id))
    if type(apps) == "table" and #apps > 0 then return apps[1] end
  end
  return nil
end

local payload = parsePayload()
local sessionId = normalize(payload.session_id)
if sessionId == "" then fail("session_id cannot be empty") end
local state = ensureState()
local session = state.sessions[sessionId]
if not session then fail("session_id does not exist") end

local app = resolveAppFromSession(session)
if not app then fail("unable to resolve app from session") end
if not observermod or not observermod.new then
  fail("AX observer backend unavailable in Hammerspoon runtime")
end

local watchId = normalize(payload.watch_id)
if watchId == "" then watchId = generateWatchId() end

local events = asTable(payload.events)
if #events == 0 then
  events = { "AXFocusedUIElementChanged", "AXTitleChanged" }
end
local normalizedEvents = {}
for _, eventName in ipairs(events) do
  local value = normalize(eventName)
  if value ~= "" then
    table.insert(normalizedEvents, value)
  end
end
if #normalizedEvents == 0 then
  normalizedEvents = { "AXFocusedUIElementChanged", "AXTitleChanged" }
end

local maxBuffer = tonumber(payload.max_buffer) or 256
if maxBuffer < 1 then maxBuffer = 1 end

local slot = state.watchers[watchId]
if slot and slot.observer and slot.observer.stop then
  pcall(function() slot.observer:stop() end)
end

local appElement = ax.applicationElement(app)
if not appElement then fail("unable to resolve AX application element from session") end
local pid = tonumber(session.pid) or app:pid()
if not pid then fail("unable to resolve app pid from session") end

slot = {
  watch_id = watchId,
  session_id = sessionId,
  events = normalizedEvents,
  max_buffer = maxBuffer,
  dropped = 0,
  buffer = {},
  observed_pid = pid,
}

local function safeAttr(element, name)
  local ok, value = pcall(function() return element:attributeValue(name) end)
  if ok then return value end
  return nil
end

local function callback(_, element, eventName, details)
  local current = state.watchers[watchId]
  if not current then return end
  local evt = {
    watch_id = watchId,
    event = tostring(eventName),
    at_ms = nowMs(),
    pid = current.observed_pid,
  }
  if element then
    local role = safeAttr(element, "AXRole")
    local title = safeAttr(element, "AXTitle")
    local identifier = safeAttr(element, "AXIdentifier")
    evt.role = role and tostring(role) or nil
    evt.title = title and tostring(title) or nil
    evt.identifier = identifier and tostring(identifier) or nil
  end
  table.insert(current.buffer, evt)
  while #current.buffer > current.max_buffer do
    table.remove(current.buffer, 1)
    current.dropped = (current.dropped or 0) + 1
  end
end

local observer = observermod.new(pid)
if not observer then fail("failed to create AX observer") end
observer:callback(callback)

local registered = {}
for _, eventName in ipairs(normalizedEvents) do
  local ok = pcall(function()
    observer:addWatcher(appElement, eventName)
  end)
  if ok then
    table.insert(registered, eventName)
  end
end
if #registered == 0 then
  fail("failed to register AX notifications for observer")
end

local started = pcall(function()
  observer:start()
end)
if not started then
  fail("failed to start AX observer")
end

slot.observer = observer
slot.events = registered
state.watchers[watchId] = slot

return json.encode({
  watch_id = watchId,
  session_id = sessionId,
  events = registered,
  max_buffer = maxBuffer,
  started = true,
})
"#;

const AX_WATCH_POLL_HS_SCRIPT: &str = r#"
local json = hs.json

local function fail(message) error(message, 0) end
local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end
local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end

local payload = parsePayload()
local watchId = normalize(payload.watch_id)
if watchId == "" then fail("watch_id cannot be empty") end
local limit = tonumber(payload.limit) or 50
if limit < 1 then limit = 1 end
local drain = payload.drain
if drain == nil then drain = true end

local state = ensureState()
local slot = state.watchers[watchId]
if not slot then fail("watch_id does not exist") end

local events = {}
local available = #slot.buffer
local take = math.min(available, limit)
for i = 1, take do
  table.insert(events, slot.buffer[i])
end

if drain then
  for _ = 1, take do
    table.remove(slot.buffer, 1)
  end
end

local running = false
if slot.observer and slot.observer.isRunning then
  local ok, value = pcall(function() return slot.observer:isRunning() end)
  if ok then running = value and true or false end
end

return json.encode({
  watch_id = watchId,
  events = events,
  dropped = slot.dropped or 0,
  running = running,
})
"#;

const AX_WATCH_STOP_HS_SCRIPT: &str = r#"
local json = hs.json

local function fail(message) error(message, 0) end
local function normalize(value)
  if value == nil then return "" end
  return tostring(value)
end
local function ensureState()
  _G.__codex_macos_agent_ax = _G.__codex_macos_agent_ax or { sessions = {}, watchers = {} }
  return _G.__codex_macos_agent_ax
end
local function parsePayload()
  local raw = (_cli and _cli.args and _cli.args[1]) or "{}"
  local payload = json.decode(raw)
  if type(payload) ~= "table" then fail("invalid payload JSON") end
  return payload
end

local payload = parsePayload()
local watchId = normalize(payload.watch_id)
if watchId == "" then fail("watch_id cannot be empty") end
local state = ensureState()
local slot = state.watchers[watchId]
if not slot then
  return json.encode({ watch_id = watchId, stopped = false, drained = 0 })
end

local drained = #(slot.buffer or {})
if slot.observer and slot.observer.stop then
  pcall(function() slot.observer:stop() end)
end
state.watchers[watchId] = nil
return json.encode({
  watch_id = watchId,
  stopped = true,
  drained = drained,
})
"#;

#[derive(Debug, Default, Clone, Copy)]
pub struct HammerspoonAxBackend;

impl AxBackendAdapter for HammerspoonAxBackend {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError> {
        run_hs_json(
            runner,
            "ax.list",
            request,
            AX_LIST_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError> {
        if selector_is_empty(&request.selector) {
            return Err(
                CliError::ax_contract_failure("ax.click", "selector is empty")
                    .with_operation("ax.click.hammerspoon")
                    .with_hint("Provide --node-id or selector filters (--role/--title-contains)."),
            );
        }

        run_hs_json(
            runner,
            "ax.click",
            request,
            AX_CLICK_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError> {
        if request.text.trim().is_empty() {
            return Err(
                CliError::usage("--text cannot be empty").with_operation("ax.type.hammerspoon")
            );
        }
        if selector_is_empty(&request.selector) {
            return Err(
                CliError::ax_contract_failure("ax.type", "selector is empty")
                    .with_operation("ax.type.hammerspoon")
                    .with_hint("Provide --node-id or selector filters (--role/--title-contains)."),
            );
        }

        run_hs_json(
            runner,
            "ax.type",
            request,
            AX_TYPE_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn attr_get(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxAttrGetRequest,
        timeout_ms: u64,
    ) -> Result<AxAttrGetResult, CliError> {
        if request.name.trim().is_empty() {
            return Err(
                CliError::usage("--name cannot be empty").with_operation("ax.attr.get.hammerspoon")
            );
        }
        if selector_is_empty(&request.selector) {
            return Err(
                CliError::ax_contract_failure("ax.attr.get", "selector is empty")
                    .with_operation("ax.attr.get.hammerspoon")
                    .with_hint("Provide --node-id or selector filters."),
            );
        }

        run_hs_json(
            runner,
            "ax.attr.get",
            request,
            AX_ATTR_GET_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn attr_set(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxAttrSetRequest,
        timeout_ms: u64,
    ) -> Result<AxAttrSetResult, CliError> {
        if request.name.trim().is_empty() {
            return Err(
                CliError::usage("--name cannot be empty").with_operation("ax.attr.set.hammerspoon")
            );
        }
        if selector_is_empty(&request.selector) {
            return Err(
                CliError::ax_contract_failure("ax.attr.set", "selector is empty")
                    .with_operation("ax.attr.set.hammerspoon")
                    .with_hint("Provide --node-id or selector filters."),
            );
        }

        run_hs_json(
            runner,
            "ax.attr.set",
            request,
            AX_ATTR_SET_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn action_perform(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxActionPerformRequest,
        timeout_ms: u64,
    ) -> Result<AxActionPerformResult, CliError> {
        if request.name.trim().is_empty() {
            return Err(CliError::usage("--name cannot be empty")
                .with_operation("ax.action.perform.hammerspoon"));
        }
        if selector_is_empty(&request.selector) {
            return Err(
                CliError::ax_contract_failure("ax.action.perform", "selector is empty")
                    .with_operation("ax.action.perform.hammerspoon")
                    .with_hint("Provide --node-id or selector filters."),
            );
        }

        run_hs_json(
            runner,
            "ax.action.perform",
            request,
            AX_ACTION_PERFORM_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn session_start(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxSessionStartRequest,
        timeout_ms: u64,
    ) -> Result<AxSessionStartResult, CliError> {
        run_hs_json(
            runner,
            "ax.session.start",
            request,
            AX_SESSION_START_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn session_list(
        &self,
        runner: &dyn ProcessRunner,
        timeout_ms: u64,
    ) -> Result<AxSessionListResult, CliError> {
        run_hs_json(
            runner,
            "ax.session.list",
            &serde_json::json!({}),
            AX_SESSION_LIST_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn session_stop(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxSessionStopRequest,
        timeout_ms: u64,
    ) -> Result<AxSessionStopResult, CliError> {
        if request.session_id.trim().is_empty() {
            return Err(CliError::usage("--session-id cannot be empty")
                .with_operation("ax.session.stop.hammerspoon"));
        }

        run_hs_json(
            runner,
            "ax.session.stop",
            request,
            AX_SESSION_STOP_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn watch_start(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchStartRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchStartResult, CliError> {
        if request.session_id.trim().is_empty() {
            return Err(CliError::usage("--session-id cannot be empty")
                .with_operation("ax.watch.start.hammerspoon"));
        }
        run_hs_json(
            runner,
            "ax.watch.start",
            request,
            AX_WATCH_START_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn watch_poll(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchPollRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchPollResult, CliError> {
        if request.watch_id.trim().is_empty() {
            return Err(CliError::usage("--watch-id cannot be empty")
                .with_operation("ax.watch.poll.hammerspoon"));
        }
        run_hs_json(
            runner,
            "ax.watch.poll",
            request,
            AX_WATCH_POLL_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }

    fn watch_stop(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchStopRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchStopResult, CliError> {
        if request.watch_id.trim().is_empty() {
            return Err(CliError::usage("--watch-id cannot be empty")
                .with_operation("ax.watch.stop.hammerspoon"));
        }
        run_hs_json(
            runner,
            "ax.watch.stop",
            request,
            AX_WATCH_STOP_HS_SCRIPT,
            timeout_ms.max(1),
        )
    }
}

pub fn is_backend_unavailable_error(error: &CliError) -> bool {
    if !error
        .operation()
        .map(|operation| operation.ends_with(".hammerspoon"))
        .unwrap_or(false)
    {
        return false;
    }

    error
        .hints()
        .iter()
        .any(|hint| hint.starts_with(BACKEND_UNAVAILABLE_HINT_PREFIX))
}

fn run_hs_json<Request, Response>(
    runner: &dyn ProcessRunner,
    operation: &'static str,
    payload: &Request,
    script: &'static str,
    timeout_ms: u64,
) -> Result<Response, CliError>
where
    Request: Serialize,
    Response: DeserializeOwned,
{
    if let Some(override_json) = test_mode_override_json(operation) {
        return parse_hs_output(operation, &override_json);
    }

    if test_mode::enabled() {
        if let Some(default_json) = test_mode_default_json(operation) {
            return parse_hs_output(operation, default_json);
        }
    }

    let payload_json = serde_json::to_string(payload).map_err(|err| {
        CliError::ax_payload_encode(&format!("{operation}.hammerspoon"), err.to_string())
    })?;
    let timeout_seconds = format!("{:.3}", (timeout_ms.max(1) as f64) / 1000.0);

    let request = ProcessRequest::new(
        "hs",
        vec![
            "-q".to_string(),
            "-t".to_string(),
            timeout_seconds,
            "-c".to_string(),
            script.to_string(),
            "--".to_string(),
            payload_json,
        ],
        timeout_ms.max(1),
    );

    let stdout = runner
        .run(&request)
        .map(|output| output.stdout)
        .map_err(|failure| map_hs_failure(operation, failure))?;

    parse_hs_output(operation, &stdout)
}

fn parse_hs_output<Response>(operation: &str, raw: &str) -> Result<Response, CliError>
where
    Response: DeserializeOwned,
{
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::ax_parse_failure(
            &format!("{operation}.hammerspoon"),
            "empty stdout",
        )
        .with_hint(
            "Ensure Hammerspoon is running and `hs.ipc` is enabled in ~/.hammerspoon/init.lua.",
        ));
    }

    serde_json::from_str(trimmed).map_err(|err| {
        CliError::ax_parse_failure(
            &format!("{operation}.hammerspoon"),
            format!("{err}; output preview: {}", output_preview(trimmed, 240)),
        )
        .with_hint("If backend mode is `auto`, macos-agent may fall back to JXA for AX commands.")
    })
}

fn map_hs_failure(operation: &str, failure: ProcessFailure) -> CliError {
    let operation_label = format!("{operation}.hammerspoon");

    match failure {
        ProcessFailure::NotFound { .. } => CliError::runtime(
            "hammerspoon AX backend is unavailable: missing dependency `hs` in PATH",
        )
        .with_operation(operation_label)
        .with_hint(
            "Hammerspoon backend unavailable; install Hammerspoon and ensure `hs` is in PATH.",
        )
        .with_hint("Auto mode will fall back to JXA AX backend."),
        ProcessFailure::Timeout { timeout_ms, .. } => CliError::timeout(
            &operation_label,
            timeout_ms,
        )
        .with_operation(operation_label)
        .with_hint("Hammerspoon backend unavailable; command timed out while connecting to hs IPC.")
        .with_hint(
            "Enable `require('hs.ipc')` in ~/.hammerspoon/init.lua and keep Hammerspoon running.",
        ),
        ProcessFailure::NonZero { code, stderr, .. } => {
            let lower = stderr.to_ascii_lowercase();
            let unavailable = lower.contains("message port")
                || lower.contains("ipc module")
                || lower.contains("is it running")
                || lower.contains("connection refused");

            let mut error = CliError::runtime(format!(
                "{operation_label} failed via `hs` (exit {code}): {stderr}"
            ))
            .with_operation(operation_label);

            if unavailable {
                error = error
                    .with_hint(
                        "Hammerspoon backend unavailable; hs cannot connect to Hammerspoon IPC.",
                    )
                    .with_hint(
                        "Enable `require('hs.ipc')` in ~/.hammerspoon/init.lua and reload config.",
                    )
                    .with_hint("Auto mode will fall back to JXA AX backend.");
            }

            error
        }
        ProcessFailure::Io { message, .. } => {
            CliError::runtime(format!("{operation_label} failed to run `hs`: {message}"))
                .with_operation(operation_label)
                .with_hint(
                    "Hammerspoon backend unavailable; check hs executable and local IPC state.",
                )
        }
    }
}

fn test_mode_override_json(operation: &str) -> Option<String> {
    if !test_mode::enabled() {
        return None;
    }

    let env_name = match operation {
        "ax.list" => AX_LIST_TEST_MODE_ENV,
        "ax.click" => AX_CLICK_TEST_MODE_ENV,
        "ax.type" => AX_TYPE_TEST_MODE_ENV,
        "ax.attr.get" => AX_ATTR_GET_TEST_MODE_ENV,
        "ax.attr.set" => AX_ATTR_SET_TEST_MODE_ENV,
        "ax.action.perform" => AX_ACTION_PERFORM_TEST_MODE_ENV,
        "ax.session.start" => AX_SESSION_START_TEST_MODE_ENV,
        "ax.session.list" => AX_SESSION_LIST_TEST_MODE_ENV,
        "ax.session.stop" => AX_SESSION_STOP_TEST_MODE_ENV,
        "ax.watch.start" => AX_WATCH_START_TEST_MODE_ENV,
        "ax.watch.poll" => AX_WATCH_POLL_TEST_MODE_ENV,
        "ax.watch.stop" => AX_WATCH_STOP_TEST_MODE_ENV,
        _ => return None,
    };

    std::env::var(env_name).ok().and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn test_mode_default_json(operation: &str) -> Option<&'static str> {
    match operation {
        "ax.list" => Some(r#"{"nodes":[],"warnings":[]}"#),
        "ax.click" => Some(
            r#"{"node_id":"test-node","matched_count":1,"action":"ax-press","used_coordinate_fallback":false}"#,
        ),
        "ax.type" => Some(
            r#"{"node_id":"test-node","matched_count":1,"applied_via":"ax-set-value","text_length":0,"submitted":false,"used_keyboard_fallback":false}"#,
        ),
        "ax.attr.get" => {
            Some(r#"{"node_id":"test-node","matched_count":1,"name":"AXRole","value":"AXButton"}"#)
        }
        "ax.attr.set" => Some(
            r#"{"node_id":"test-node","matched_count":1,"name":"AXValue","applied":true,"value_type":"string"}"#,
        ),
        "ax.action.perform" => {
            Some(r#"{"node_id":"test-node","matched_count":1,"name":"AXPress","performed":true}"#)
        }
        "ax.session.start" => Some(
            r#"{"session_id":"axs-test","app":"Arc","bundle_id":"company.thebrowser.Browser","pid":1001,"created_at_ms":1700000000000,"created":true}"#,
        ),
        "ax.session.list" => Some(r#"{"sessions":[]}"#),
        "ax.session.stop" => Some(r#"{"session_id":"axs-test","removed":true}"#),
        "ax.watch.start" => Some(
            r#"{"watch_id":"axw-test","session_id":"axs-test","events":["AXTitleChanged"],"max_buffer":64,"started":true}"#,
        ),
        "ax.watch.poll" => {
            Some(r#"{"watch_id":"axw-test","events":[],"dropped":0,"running":true}"#)
        }
        "ax.watch.stop" => Some(r#"{"watch_id":"axw-test","stopped":true,"drained":0}"#),
        _ => None,
    }
}

fn selector_is_empty(selector: &AxSelector) -> bool {
    selector.node_id.is_none()
        && selector.role.is_none()
        && selector.title_contains.is_none()
        && selector.identifier_contains.is_none()
        && selector.value_contains.is_none()
        && selector.subrole.is_none()
        && selector.focused.is_none()
        && selector.enabled.is_none()
}

fn output_preview(raw: &str, max_chars: usize) -> String {
    let mut preview = raw.chars().take(max_chars).collect::<String>();
    if raw.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use serde_json::json;

    use crate::backend::hammerspoon::{
        is_backend_unavailable_error, map_hs_failure, output_preview, selector_is_empty,
    };
    use crate::backend::process::ProcessFailure;
    use crate::backend::AxBackendAdapter;
    use crate::model::{
        AxActionPerformRequest, AxAttrGetRequest, AxAttrSetRequest, AxClickRequest, AxSelector,
        AxSessionStartRequest, AxSessionStopRequest, AxTarget, AxTypeRequest, AxWatchPollRequest,
        AxWatchStartRequest, AxWatchStopRequest,
    };

    fn node_selector() -> AxSelector {
        AxSelector {
            node_id: Some("1.1".to_string()),
            ..AxSelector::default()
        }
    }

    #[test]
    fn message_port_error_is_marked_backend_unavailable() {
        let error = map_hs_failure(
            "ax.list",
            ProcessFailure::NonZero {
                program: "hs".to_string(),
                code: 69,
                stderr: "can't access Hammerspoon message port Hammerspoon; is it running with the ipc module loaded?".to_string(),
            },
        );

        assert!(is_backend_unavailable_error(&error));
    }

    #[test]
    fn test_mode_override_is_honored_for_click() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _override = EnvGuard::set(
            &lock,
            "CODEX_MACOS_AGENT_AX_CLICK_JSON",
            r#"{"node_id":"1.1","matched_count":1,"action":"ax-press","used_coordinate_fallback":false}"#,
        );

        let runner = crate::backend::process::RealProcessRunner;
        let request = crate::model::AxClickRequest {
            target: crate::model::AxTarget::default(),
            selector: crate::model::AxSelector {
                node_id: Some("1.1".to_string()),
                role: None,
                title_contains: None,
                identifier_contains: None,
                value_contains: None,
                subrole: None,
                focused: None,
                enabled: None,
                nth: None,
            },
            allow_coordinate_fallback: false,
        };

        let result = super::HammerspoonAxBackend
            .click(&runner, &request, 1000)
            .expect("click should parse test override");
        assert_eq!(result.node_id.as_deref(), Some("1.1"));
        assert_eq!(result.matched_count, 1);
    }

    #[test]
    fn default_test_mode_fixtures_cover_all_hammerspoon_ax_operations() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let backend = super::HammerspoonAxBackend;
        let runner = crate::backend::process::RealProcessRunner;

        let list = backend
            .list(&runner, &crate::model::AxListRequest::default(), 1000)
            .expect("list default fixture");
        assert!(list.nodes.is_empty());

        let click = backend
            .click(
                &runner,
                &AxClickRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    allow_coordinate_fallback: false,
                },
                1000,
            )
            .expect("click default fixture");
        assert_eq!(click.matched_count, 1);

        let typ = backend
            .type_text(
                &runner,
                &AxTypeRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    text: "test".to_string(),
                    clear_first: false,
                    submit: false,
                    paste: false,
                    allow_keyboard_fallback: false,
                },
                1000,
            )
            .expect("type default fixture");
        assert_eq!(typ.applied_via, "ax-set-value");

        let attr_get = backend
            .attr_get(
                &runner,
                &AxAttrGetRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: "AXRole".to_string(),
                },
                1000,
            )
            .expect("attr get default fixture");
        assert_eq!(attr_get.name, "AXRole");

        let attr_set = backend
            .attr_set(
                &runner,
                &AxAttrSetRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: "AXValue".to_string(),
                    value: json!("hello"),
                },
                1000,
            )
            .expect("attr set default fixture");
        assert!(attr_set.applied);

        let action = backend
            .action_perform(
                &runner,
                &AxActionPerformRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: "AXPress".to_string(),
                },
                1000,
            )
            .expect("action default fixture");
        assert!(action.performed);

        let session_start = backend
            .session_start(
                &runner,
                &AxSessionStartRequest {
                    target: AxTarget::default(),
                    session_id: Some("axs-test".to_string()),
                },
                1000,
            )
            .expect("session start default fixture");
        assert_eq!(session_start.session.session_id, "axs-test");

        let session_list = backend
            .session_list(&runner, 1000)
            .expect("session list default fixture");
        assert!(session_list.sessions.is_empty());

        let session_stop = backend
            .session_stop(
                &runner,
                &AxSessionStopRequest {
                    session_id: "axs-test".to_string(),
                },
                1000,
            )
            .expect("session stop default fixture");
        assert!(session_stop.removed);

        let watch_start = backend
            .watch_start(
                &runner,
                &AxWatchStartRequest {
                    session_id: "axs-test".to_string(),
                    events: vec!["AXTitleChanged".to_string()],
                    max_buffer: 64,
                    watch_id: Some("axw-test".to_string()),
                },
                1000,
            )
            .expect("watch start default fixture");
        assert_eq!(watch_start.watch_id, "axw-test");

        let watch_poll = backend
            .watch_poll(
                &runner,
                &AxWatchPollRequest {
                    watch_id: "axw-test".to_string(),
                    limit: 10,
                    drain: true,
                },
                1000,
            )
            .expect("watch poll default fixture");
        assert!(watch_poll.running);

        let watch_stop = backend
            .watch_stop(
                &runner,
                &AxWatchStopRequest {
                    watch_id: "axw-test".to_string(),
                },
                1000,
            )
            .expect("watch stop default fixture");
        assert!(watch_stop.stopped);
    }

    #[test]
    fn validation_errors_are_reported_for_empty_hammerspoon_inputs() {
        let backend = super::HammerspoonAxBackend;
        let runner = crate::backend::process::RealProcessRunner;

        let click_err = backend
            .click(
                &runner,
                &AxClickRequest {
                    target: AxTarget::default(),
                    selector: AxSelector::default(),
                    allow_coordinate_fallback: false,
                },
                1000,
            )
            .expect_err("empty click selector should fail");
        assert!(click_err.to_string().contains("selector is empty"));

        let type_err = backend
            .type_text(
                &runner,
                &AxTypeRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    text: "   ".to_string(),
                    clear_first: false,
                    submit: false,
                    paste: false,
                    allow_keyboard_fallback: false,
                },
                1000,
            )
            .expect_err("empty text should fail");
        assert!(type_err.to_string().contains("--text cannot be empty"));

        let attr_get_err = backend
            .attr_get(
                &runner,
                &AxAttrGetRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: " ".to_string(),
                },
                1000,
            )
            .expect_err("empty attr get name should fail");
        assert!(attr_get_err.to_string().contains("--name cannot be empty"));

        let attr_set_err = backend
            .attr_set(
                &runner,
                &AxAttrSetRequest {
                    target: AxTarget::default(),
                    selector: AxSelector::default(),
                    name: "AXValue".to_string(),
                    value: json!("hello"),
                },
                1000,
            )
            .expect_err("empty attr set selector should fail");
        assert!(attr_set_err.to_string().contains("selector is empty"));

        let action_err = backend
            .action_perform(
                &runner,
                &AxActionPerformRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: " ".to_string(),
                },
                1000,
            )
            .expect_err("empty action name should fail");
        assert!(action_err.to_string().contains("--name cannot be empty"));

        let session_stop_err = backend
            .session_stop(
                &runner,
                &AxSessionStopRequest {
                    session_id: " ".to_string(),
                },
                1000,
            )
            .expect_err("empty session id should fail");
        assert!(session_stop_err
            .to_string()
            .contains("--session-id cannot be empty"));

        let watch_start_err = backend
            .watch_start(
                &runner,
                &AxWatchStartRequest {
                    session_id: " ".to_string(),
                    events: vec![],
                    max_buffer: 10,
                    watch_id: None,
                },
                1000,
            )
            .expect_err("empty watch session should fail");
        assert!(watch_start_err
            .to_string()
            .contains("--session-id cannot be empty"));

        let watch_poll_err = backend
            .watch_poll(
                &runner,
                &AxWatchPollRequest {
                    watch_id: " ".to_string(),
                    limit: 10,
                    drain: true,
                },
                1000,
            )
            .expect_err("empty watch id should fail");
        assert!(watch_poll_err
            .to_string()
            .contains("--watch-id cannot be empty"));

        let watch_stop_err = backend
            .watch_stop(
                &runner,
                &AxWatchStopRequest {
                    watch_id: " ".to_string(),
                },
                1000,
            )
            .expect_err("empty watch id should fail");
        assert!(watch_stop_err
            .to_string()
            .contains("--watch-id cannot be empty"));
    }

    #[test]
    fn invalid_override_json_reports_parse_hint() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _override = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_ATTR_GET_JSON", "not-json");

        let backend = super::HammerspoonAxBackend;
        let runner = crate::backend::process::RealProcessRunner;
        let err = backend
            .attr_get(
                &runner,
                &AxAttrGetRequest {
                    target: AxTarget::default(),
                    selector: node_selector(),
                    name: "AXRole".to_string(),
                },
                1000,
            )
            .expect_err("invalid json override should fail");
        let rendered = err.to_string();
        assert!(rendered.contains("output preview"));
    }

    #[test]
    fn empty_override_value_falls_back_to_default_fixture() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _override = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_SESSION_LIST_JSON", "   ");

        let backend = super::HammerspoonAxBackend;
        let runner = crate::backend::process::RealProcessRunner;
        let result = backend
            .session_list(&runner, 1000)
            .expect("default fixture should be used");
        assert!(result.sessions.is_empty());
    }

    #[test]
    fn not_found_failure_is_marked_backend_unavailable() {
        let error = map_hs_failure(
            "ax.list",
            ProcessFailure::NotFound {
                program: "hs".to_string(),
            },
        );
        assert!(is_backend_unavailable_error(&error));
    }

    #[test]
    fn selector_and_preview_helpers_cover_expected_cases() {
        assert!(selector_is_empty(&AxSelector::default()));
        assert!(!selector_is_empty(&AxSelector {
            title_contains: Some("Save".to_string()),
            ..AxSelector::default()
        }));

        let preview = output_preview("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(preview, "abcdefgh...");
    }
}
