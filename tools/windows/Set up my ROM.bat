@echo off
rem Drag your ROM file onto this file (once). If your ROM is the Japanese Rev 1 one, drag the
rem Klepto v1.02 patch (dbz102.ips) at the same time: select both files, then drag.
cd /d "%~dp0"
set "ROM="
set "PATCH="
:next
if "%~1"=="" goto run
if /i "%~x1"==".ips" (set "PATCH=%~1") else (set "ROM=%~1")
shift
goto next
:run
if "%ROM%"=="" (
  echo Drag your ROM file onto this file.
  echo.
  pause
  exit /b 1
)
if "%PATCH%"=="" (
  "%~dp0ssd-recompiled.exe" --rom "%ROM%" --headless-frames 1
) else (
  "%~dp0ssd-recompiled.exe" --rom "%ROM%" --ips "%PATCH%" --headless-frames 1
)
if errorlevel 1 (
  echo.
  echo The ROM was not accepted. See the message above, and the README for the accepted files.
) else (
  echo.
  echo Done. Your ROM is set up. Start "Play Classic.bat" or "Play Remastered.bat".
)
echo.
pause
