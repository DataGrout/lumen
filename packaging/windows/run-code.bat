@echo off
REM Lumen launcher for Claude Code (the CLI). Double-click to start Lumen and
REM launch Claude Code wired through it. Pass-through args work too.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lumen.ps1" -Client Code %*
if errorlevel 1 (
  echo.
  echo ============================================================
  echo  Lumen launcher reported a problem - see the message above.
  echo ============================================================
  pause
)
