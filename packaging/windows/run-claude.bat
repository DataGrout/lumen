@echo off
REM Lumen launcher for Claude Desktop (the app). Double-click to start Lumen and
REM launch Claude Desktop wired through it. Pass-through args work too.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lumen.ps1" -Client Claude %*
if errorlevel 1 (
  echo.
  echo ============================================================
  echo  Lumen launcher reported a problem - see the message above.
  echo ============================================================
  pause
)
