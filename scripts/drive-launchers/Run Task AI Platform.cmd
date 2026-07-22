@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0source\scripts\drive-workspace.ps1" -Action Run -DriveRoot "%~dp0."
if errorlevel 1 pause
