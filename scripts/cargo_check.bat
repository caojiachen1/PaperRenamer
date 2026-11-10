@echo off
call "D:\Softwares\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >/dev/null
cd /d "%~dp0.."
cargo check 2>&1
if errorlevel 1 exit /b 1
exit /b 0
