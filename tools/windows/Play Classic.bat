@echo off
rem Dragon Ball Z: Super Saiya Densetsu - the original game, as the cartridge plays it.
cd /d "%~dp0"
if not exist "%LOCALAPPDATA%\dbz-ssd-recompiled\cache\rom\canonical.sfc" (
  echo Your ROM is not set up yet.
  echo Drag your game file onto "Set up my ROM.bat" once, then start this again.
  echo.
  pause
  exit /b 1
)
"%~dp0ssd-recompiled.exe" @HOOKS@
if errorlevel 1 pause
