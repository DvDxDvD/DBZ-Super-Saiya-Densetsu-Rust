@echo off
rem Dragon Ball Z: Super Saiya Densetsu - the original game, as the cartridge plays it.
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
"%~dp0ssd-recompiled.exe" @HOOKS@
if errorlevel 1 pause
