@echo off
call "D:\Softwares\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set JAVA_HOME=D:\Softwares\java-26.0.2
set PATH=%JAVA_HOME%\bin;%PATH%
cd /d D:\Codebase\PaperRenamer\src-tauri
cargo check 2>&1
if errorlevel 1 exit /b 1
exit /b 0