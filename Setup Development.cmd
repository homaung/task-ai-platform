@echo off
title Task AI Platform - Development Setup
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\setup-windows-dev.ps1"
if errorlevel 1 (
  echo.
  echo Setup failed. Review the error above.
  pause
)

