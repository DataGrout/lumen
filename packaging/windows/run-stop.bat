@echo off
REM Stop the Lumen daemon cleanly (it keeps running in the background otherwise;
REM this avoids having to end it in Task Manager).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lumen.ps1" -Stop %*
pause
