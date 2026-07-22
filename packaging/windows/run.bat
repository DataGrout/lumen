@echo off
REM One-click Lumen launcher for Windows.
REM Double-clicking this runs lumen.ps1 without changing any machine-wide
REM PowerShell settings (-ExecutionPolicy Bypass applies to this run only).
REM Pass-through args work too, e.g.:  run.bat -Client Cursor
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lumen.ps1" %*
if errorlevel 1 (
  echo.
  echo ============================================================
  echo  Lumen launcher reported a problem - see the message above.
  echo ============================================================
  pause
)
