-- Mesen2 headless tracer for DBZ Super Saiya Densetsu (SNES).
-- Use: run_trace.sh (it runs `xtask export-mlb` first and sets the variables below).
--   Mesen.exe --testRunner trace.lua <rom.sfc> --timeout=<sec>
-- Required variables:
--   DBZ_OUT=<dir>      output directory (with trailing slash); run_trace.sh uses .work/re/<run>/
--   DBZ_SYMBOLS=<file> symbols.lua written by `xtask export-mlb`. The game's names and addresses
--                      live only in re/symbols.toml; this script holds no game address.
-- Output in DBZ_OUT:
--   exec.csv    one row per executed PC: pc,m8,x8,e,d,dbr,hits,modes (sampled M/X/D/DBR)
--   calls.csv   observed JSR/JSL/JSR(abs,X) targets: target,caller,op
--   jumps.csv   observed indirect JMP/JML targets: target,source,op
--   prgrom.cdl  raw Mesen CDL of the PRG-ROM (1 flag byte per ROM byte, no ROM data)
--   shots/      PNG screenshots every SHOT_EVERY frames (only if DBZ_SHOT_DIR is not empty)
-- Optional variables:
--   DBZ_TRACE=0        no per-instruction exec callback (fast exploration, no CSV)
--   DBZ_LOAD=<file>    load a Mesen savestate at startup (before the first scripted frame)
--   DBZ_SAVE=<file>    save a savestate at the end of the run (to resume from there)
--   DBZ_INPUTS=<file>  input timeline: rows "<n> <keys>" = hold the keys for n frames (keys separated
--                      by commas: a,b,x,y,l,r,start,select,up,down,left,right; "-" = none), in order.
--                      "@mode <mash|walk|idle|battle>" sets the fallback mode used after the last row.
--                      "#" comments out the rest of the line.
--   DBZ_SHOT_EVERY=<n> screenshot interval (default 600)
--   DBZ_RAMDUMP=<file> at the end of the run save the WRAM (128 KB) to find state variables

local OUT_DIR = os.getenv("DBZ_OUT")
local SYMBOLS = os.getenv("DBZ_SYMBOLS")
if not OUT_DIR or OUT_DIR == "" then error("DBZ_OUT is required (output directory)") end
if not SYMBOLS or SYMBOLS == "" then error("DBZ_SYMBOLS is required (symbols.lua from xtask export-mlb)") end
local SYM = dofile(SYMBOLS)
local WRAM_BASE = 0x7E0000
-- WRAM offset of a variable of re/symbols.toml, by name.
local function var(name)
  local a = SYM[name]
  if not a then error("symbol " .. name .. " missing from " .. SYMBOLS) end
  return a - WRAM_BASE
end
local VAR_IN_BATTLE = var("in_battle")                -- $FF in battle
local VAR_TRAVEL_MODE = var("travel_mode")            -- 1 walk, 2 fly
local VAR_MAP_MENU_CURSOR = var("map_menu_cursor")    -- 0 Talk, 1 Look, 2 Fly, 3 Item, 4 Menu

local SHOT_DIR   = os.getenv("DBZ_SHOT_DIR") or ""
local MAX_FRAMES = tonumber(os.getenv("DBZ_FRAMES") or "6000")
local SHOT_EVERY = tonumber(os.getenv("DBZ_SHOT_EVERY") or "600")
local TRACE      = (os.getenv("DBZ_TRACE") or "1") ~= "0"
local LOAD       = os.getenv("DBZ_LOAD") or ""
local SAVE       = os.getenv("DBZ_SAVE") or ""
local INPUTS     = os.getenv("DBZ_INPUTS") or ""
local RAMDUMP    = os.getenv("DBZ_RAMDUMP") or ""

-- Input timeline: {from_frame, to_frame, {buttons}}; after the last entry the AUTO cycle repeats.
local SCRIPT = {
  {0,    2400, {}},                 -- intro / attract mode
}
-- Automatic cycle (period 64 frames) to advance texts, menus and battles.
local AUTO = {
  {0,  4,  {start = true}},
  {16, 20, {a = true}},
  {32, 36, {a = true}},
  {48, 52, {down = true}},
  {56, 60, {a = true}},
}
local AUTO_ALT = {                  -- every 8 cycles: explore right / B to leave the menus
  {0,  4,  {right = true}},
  {16, 20, {a = true}},
  {32, 36, {b = true}},
  {48, 52, {up = true}},
}
local frame = 0
local states = {}    -- pc -> {m8,x8,e,d,dbr,hits,modes}
local calls = {}     -- "target,caller,op" -> true
local jumps = {}
local prevPc, prevOp = -1, -1

-- NMI/IRQ handlers read from the native vectors (SNES hardware vectors in bank 0)
local function rd(a) return emu.read(a & 0xFFFFFF, emu.memType.snesDebug) end
local INT_HANDLERS = {
  [rd(0xFFEA) | (rd(0xFFEB) << 8)] = true,
  [rd(0xFFEE) | (rd(0xFFEF) << 8)] = true,
}

local CALL_OPS = { [0x20] = "JSR", [0x22] = "JSL", [0xFC] = "JSRX" }
local JUMP_OPS = { [0x6C] = "JMPI", [0x7C] = "JMPIX", [0xDC] = "JMLI" }

local function sample(pc, rec)
  local s = emu.getState()
  local ps = s["cpu.ps"]
  local m8 = (ps & 0x20) ~= 0 and 1 or 0
  local x8 = (ps & 0x10) ~= 0 and 1 or 0
  local e = s["cpu.emulationMode"] and 1 or 0
  local key = m8 .. x8 .. e .. "/" .. s["cpu.d"] .. "/" .. s["cpu.dbr"]
  if rec == nil then
    rec = { hits = 0, modes = {}, first = key }
    states[pc] = rec
  end
  rec.modes[key] = true
end

local function onExec(address, value)
  local pc = address & 0xFFFFFF
  local rec = states[pc]
  if rec == nil then
    sample(pc, nil)
    rec = states[pc]
  end
  local h = rec.hits + 1
  rec.hits = h
  -- logarithmic sampling: re-check the flags at hits 16, 256, 4096, 65536...
  if h == 16 or h == 256 or h == 4096 or h == 65536 then sample(pc, rec) end

  if prevOp >= 0 then
    -- An interrupt may arrive right after the call: the target is accepted only if it matches
    -- the operand (JSR/JSL) or at least the bank, and is not an interrupt handler.
    local c = CALL_OPS[prevOp]
    if c then
      local ok
      if prevOp == 0x22 then
        ok = pc == (rd(prevPc + 1) | (rd(prevPc + 2) << 8) | (rd(prevPc + 3) << 16))
      elseif prevOp == 0x20 then
        ok = pc == ((prevPc & 0xFF0000) | rd(prevPc + 1) | (rd(prevPc + 2) << 8))
      else
        ok = (pc & 0xFF0000) == (prevPc & 0xFF0000) and not INT_HANDLERS[pc]
      end
      if ok then calls[string.format("%06X,%06X,%s", pc, prevPc, c)] = true end
    end
    local j = JUMP_OPS[prevOp]
    if j and not INT_HANDLERS[pc] then
      if prevOp == 0xDC or (pc & 0xFF0000) == (prevPc & 0xFF0000) then
        jumps[string.format("%06X,%06X,%s", pc, prevPc, j)] = true
      end
    end
  end
  prevPc, prevOp = pc, value
end

-- Fallback modes (DBZ_MODE or "@mode" in the input file), after the timeline:
--   mash   AUTO cycle (Start/A/down) to advance texts and menus
--   walk   directions held for a long time (exploration, random encounters) interleaved with A
--   battle A every 12 frames with periodic cursor moves (picks different cards)
--   idle   no input
--   auto   seeded random pilot (DBZ_SEED) that reads the WRAM: in battle (in_battle = $FF) it presses
--          A/directions/B at random to explore commands, cards, items and escape; outside battle it
--          alternates long moves (also diagonal), A, B and pauses. Reproducible for a given seed.
--   explore long-range travel: in battle as auto; on the ground it opens the menu and picks Fly (cursor
--          read from map_menu_cursor); when flying (travel_mode = 2) it holds the DBZ_ROUTE directions
--          for a long time (e.g. "right:900,down:600"), cyclically; every ~5 s of flight it presses A then B
--          so as not to get stuck on open messages.
local MODE = os.getenv("DBZ_MODE") or "mash"
local WALK_DIRS = { "right", "down", "left", "up", "down", "right", "up", "left" }

local timeline = {}   -- {from, to, buttons}
local scriptEnd = 0
local walkDelay = 0   -- mash frames before walk (only without a timeline)
if INPUTS ~= "" then
  local t = 0
  for raw in io.lines(INPUTS) do
    local line = raw:gsub("#.*$", ""):match("^%s*(.-)%s*$")
    if line ~= "" then
      local m = line:match("^@mode%s+(%a+)$")
      if m then
        MODE = m
      else
        local n, keys = line:match("^(%d+)%s*(.*)$")
        local b = {}
        if keys ~= "" and keys ~= "-" then
          for k in keys:gmatch("[^,%s]+") do b[k] = true end
        end
        timeline[#timeline + 1] = { t, t + tonumber(n), b }
        t = t + tonumber(n)
      end
    end
  end
  scriptEnd = t
elseif LOAD == "" then
  timeline = SCRIPT
  scriptEnd = SCRIPT[#SCRIPT][2]
  walkDelay = 6000 - scriptEnd
end

local function cycleTable(tbl, t, period)
  local phase = t % period
  for _, e in ipairs(tbl) do
    if phase >= e[1] and phase < e[2] then return e[3] end
  end
  return {}
end

local function mash(t)
  return cycleTable(((t // 64) % 8 == 7) and AUTO_ALT or AUTO, t, 64)
end

local function walk(t)
  local phase = t % 160
  if phase < 96 then return { [WALK_DIRS[(t // 160) % #WALK_DIRS + 1]] = true } end
  if (phase >= 104 and phase < 108) or (phase >= 124 and phase < 128) or (phase >= 144 and phase < 148) then
    return { a = true }
  end
  return {}
end

local BATTLE_MOVES = { "right", "down", "left", "up", "right", "right", "down" }
local function battle(t)
  local phase = t % 96
  if phase < 3 or (phase >= 24 and phase < 27) or (phase >= 48 and phase < 51) then return { a = true } end
  if phase >= 72 and phase < 75 then return { [BATTLE_MOVES[(t // 96) % #BATTLE_MOVES + 1]] = true } end
  return {}
end

local rng = tonumber(os.getenv("DBZ_SEED") or "1")
local function rand(n)
  rng = (rng * 1103515245 + 12345) % 2147483648
  return (rng // 65536) % n
end
local DIRS = { "up", "down", "left", "right" }
local autoKeys, autoPressEnd, autoEnd = {}, -1, -1
local function auto(f)
  if f >= autoEnd then
    local inBattle = emu.read(VAR_IN_BATTLE, emu.memType.snesWorkRam) == 0xFF
    local r = rand(100)
    local press, gap = 3, 8 + rand(12)
    if inBattle then
      if r < 55 then autoKeys = { a = true }
      elseif r < 85 then autoKeys = { [DIRS[rand(4) + 1]] = true }
      elseif r < 95 then autoKeys = { b = true }
      else autoKeys, gap = {}, 30 end
    else
      if r < 40 then
        autoKeys = { [DIRS[rand(4) + 1]] = true }
        if rand(3) == 0 then autoKeys[DIRS[rand(4) + 1]] = true end
        press = 30 + rand(120)
      elseif r < 70 then autoKeys = { a = true }
      elseif r < 87 then autoKeys = { b = true }
      elseif r < 90 then autoKeys = { start = true }   -- esce dal titolo dopo un game over
      else autoKeys, gap = {}, 40 end
    end
    autoPressEnd, autoEnd = f + press, f + press + gap
  end
  if f < autoPressEnd then return autoKeys end
  return {}
end

local ROUTE = {}
for dir, n in (os.getenv("DBZ_ROUTE") or "right:900,down:900,left:900,up:900"):gmatch("([%a,]+):(%d+)") do
  local keys = {}
  for k in dir:gmatch("%a+") do keys[k] = true end
  ROUTE[#ROUTE + 1] = { keys, tonumber(n) }
end
local function wram(a) return emu.read(a, emu.memType.snesWorkRam) end
local exQueue, exKeys, exUntil, exCursorTries, exFlySteps = {}, {}, -1, 0, 0
local explore
local routeIdx, routeLeft = 1, ROUTE[1][2]
function explore(f)
  if f < exUntil then return exKeys end
  if wram(VAR_IN_BATTLE) == 0xFF then return auto(f) end
  if #exQueue == 0 then
    if wram(VAR_TRAVEL_MODE) == 2 then
      exFlySteps = exFlySteps + 1
      if exFlySteps % 5 == 0 then
        -- every ~5 s of flight: A to advance messages left open (e.g. "Enemies approach!" without a
        -- battle flag), then B to close the flight menu if A opened it
        exQueue = { { { a = true }, 3, 20 }, { { b = true }, 3, 20 } }
        return explore(f)
      end
      local seg = ROUTE[routeIdx]
      exKeys, exUntil = seg[1], f + 60
      routeLeft = routeLeft - 60
      if routeLeft <= 0 then
        routeIdx = routeIdx % #ROUTE + 1
        routeLeft = ROUTE[routeIdx][2]
      end
      return exKeys
    end
    -- on the ground: close texts/menus, open the menu and move the cursor to Fly
    exQueue = { { { b = true }, 3, 20 }, { { b = true }, 3, 20 }, { { a = true }, 3, 25 }, "cursor", { { a = true }, 3, 60 } }
  end
  local step = table.remove(exQueue, 1)
  if step == "cursor" then
    local c = wram(VAR_MAP_MENU_CURSOR)
    if c ~= 2 and exCursorTries < 6 then
      exCursorTries = exCursorTries + 1
      table.insert(exQueue, 1, "cursor")
      step = { { [c < 2 and "down" or "up"] = true }, 3, 12 }
    else
      exCursorTries = 0
      step = { {}, 1, 1 }
    end
  end
  exKeys, exUntil = step[1], f + step[2]
  table.insert(exQueue, 1, { {}, step[3], 0 })
  if step[3] == 0 then table.remove(exQueue, 1) end
  return exKeys
end

local function buttonsFor(f)
  for _, e in ipairs(timeline) do
    if f >= e[1] and f < e[2] then return e[3] end
  end
  local t = f - scriptEnd
  if MODE == "idle" then return {} end
  if MODE == "battle" then return battle(t) end
  if MODE == "auto" or MODE == "explore" then
    if t < walkDelay then return mash(t) end   -- as walk: first enter the game with the mash cycle
    if MODE == "explore" then return explore(t - walkDelay) end
    return auto(t - walkDelay)
  end
  if MODE == "walk" then
    if t < walkDelay then return mash(t) end
    return walk(t - walkDelay)
  end
  return mash(t)
end

local function writeRam()
  if RAMDUMP == "" then return end
  local size = emu.getMemorySize(emu.memType.snesWorkRam)
  local bytes = {}
  for i = 0, size - 1 do bytes[#bytes + 1] = string.char(emu.read(i, emu.memType.snesWorkRam)) end
  local g = io.open(RAMDUMP, "wb")
  g:write(table.concat(bytes))
  g:close()
end

local function writeAll()
  writeRam()
  if not TRACE then return end
  local f = io.open(OUT_DIR .. "exec.csv", "w")
  f:write("pc,m8,x8,e,d,dbr,hits,modes\n")
  local pcs = {}
  for pc in pairs(states) do pcs[#pcs + 1] = pc end
  table.sort(pcs)
  for _, pc in ipairs(pcs) do
    local r = states[pc]
    local m8, x8, e, d, dbr = r.first:match("^(%d)(%d)(%d)/(%d+)/(%d+)$")
    local n = 0
    local list = {}
    for k in pairs(r.modes) do n = n + 1; list[#list + 1] = k end
    table.sort(list)
    f:write(string.format("%06X,%s,%s,%s,%04X,%02X,%d,%s\n", pc, m8, x8, e, tonumber(d), tonumber(dbr), r.hits, table.concat(list, ";")))
  end
  f:close()

  local function dump(name, header, set)
    local keys = {}
    for k in pairs(set) do keys[#keys + 1] = k end
    table.sort(keys)
    local g = io.open(OUT_DIR .. name, "w")
    g:write(header, "\n")
    for _, k in ipairs(keys) do g:write(k, "\n") end
    g:close()
  end
  dump("calls.csv", "target,caller,op", calls)
  dump("jumps.csv", "target,source,op", jumps)

  local ok, cdl = pcall(emu.getCdlData, emu.memType.snesPrgRom)
  if ok and cdl then
    local size = emu.getMemorySize(emu.memType.snesPrgRom)
    local bytes = {}
    for i = 0, size - 1 do bytes[#bytes + 1] = string.char(cdl[i] or 0) end
    local g = io.open(OUT_DIR .. "prgrom.cdl", "wb")
    g:write(table.concat(bytes))
    g:close()
  end

  local s = io.open(OUT_DIR .. "summary.txt", "w")
  s:write("frames=", frame, "\npcs=", #pcs, "\n")
  s:close()
end

if TRACE then
  emu.addMemoryCallback(onExec, emu.callbackType.exec, 0x000000, 0xFFFFFF, emu.cpuType.snes, emu.memType.snesMemory)
end

-- Savestate: Mesen2 allows it only inside an exec callback of the main CPU. A temporary exec callback is
-- registered over the whole space (the first executed instruction triggers it) and removed at once.
-- The NMI handler is not used: NMI is disabled during the intro.
local function runInExec(fn)
  local ref
  ref = emu.addMemoryCallback(function()
    emu.removeMemoryCallback(ref, emu.callbackType.exec, 0x000000, 0xFFFFFF, emu.cpuType.snes, emu.memType.snesMemory)
    fn()
  end, emu.callbackType.exec, 0x000000, 0xFFFFFF, emu.cpuType.snes, emu.memType.snesMemory)
end

local finishing = false
local function finish()
  if finishing then return end
  finishing = true
  if SAVE == "" then
    writeAll()
    emu.stop(0)
    return
  end
  runInExec(function()
    local g = io.open(SAVE, "wb")
    g:write(emu.createSavestate())
    g:close()
    writeAll()
    emu.stop(0)
  end)
end

if LOAD ~= "" then
  runInExec(function()
    local g = io.open(LOAD, "rb")
    local st = g:read("*a")
    g:close()
    local ok, res = pcall(emu.loadSavestate, st)
    local lg = io.open(OUT_DIR .. "load.log", "w")
    lg:write("bytes=", #st, " ok=", tostring(ok), " result=", tostring(res), "\n")
    lg:close()
    frame = 0
  end)
end

emu.addEventCallback(function()
  emu.setInput(buttonsFor(frame), 0)
end, emu.eventType.inputPolled)

emu.addEventCallback(function()
  frame = frame + 1
  if SHOT_DIR ~= "" and frame % SHOT_EVERY == 0 then
    local png = emu.takeScreenshot()
    local g = io.open(string.format("%s/f%06d.png", SHOT_DIR, frame), "wb")
    g:write(png)
    g:close()
  end
  if frame >= MAX_FRAMES then finish() end
end, emu.eventType.endFrame)
