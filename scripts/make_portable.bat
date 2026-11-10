@echo off
set SRC=D:\Codebase\PaperRenamer
set DST=%SRC%\dist-portable
if exist "%DST%" rmdir /s /q "%DST%"
mkdir "%DST%"
copy /y "%SRC%\src-tauri\target\release\paper-renamer.exe" "%DST%\PaperRenamer.exe"
if errorlevel 1 exit /b 1
echo PORTABLE_OK
exit /b 0