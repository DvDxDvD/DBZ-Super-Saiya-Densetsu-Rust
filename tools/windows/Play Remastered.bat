@echo off
rem Dragon Ball Z: Super Saiya Densetsu - Remastered: HD picture and art, 16:9, HD menus, cheats.
rem On the first start the game builds its HD data from your ROM and hd-pack.zip (under a minute, once).
cd /d "%~dp0"
rem The game's data folder: SSD_DATA_DIR when set, as the game itself reads it.
set "SSD_DATA=%SSD_DATA_DIR%"
if not defined SSD_DATA set "SSD_DATA=%LOCALAPPDATA%\dbz-ssd-recompiled"
if not exist "%SSD_DATA%\cache\rom\canonical.sfc" (
  echo Your ROM is not set up yet.
  echo Drag your game file onto "Set up my ROM.bat" once, then start this again.
  echo.
  pause
  exit /b 1
)
if not exist "%~dp0hd-pack.zip" if not exist "%~dp0hd-pack\hd-pack.toml" if not exist "%SSD_DATA%\cache\hd\" (
  echo hd-pack.zip is not next to this file: the Remastered mode will draw the game's own
  echo pictures, made bigger. Download hd-pack.zip from the same page as the game and put it here
  echo for the full HD art.
  echo.
)
"%~dp0ssd-recompiled.exe" --mode enhanced --hd-scale 4 @HOOKS@
if errorlevel 1 pause
