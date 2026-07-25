@echo off
REM Lumen launcher for Cursor. Double-click to start Lumen and launch Cursor
REM wired through it. Pass-through args work too.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lumen.ps1" -Client Cursor %*
if errorlevel 1 (
  echo.
  echo ============================================================
  echo  Lumen launcher reported a problem - see the message above.
  echo ============================================================
  pause
)
