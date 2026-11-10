@echo off
set SRC=%~dp0..
set DST=%SRC%\dist-portable
if exist "%DST%" rmdir /s /q "%DST%"
mkdir "%DST%"
copy /y "%SRC%\target\release\PaperRenamer.exe" "%DST%\PaperRenamer.exe"
if errorlevel 1 exit /b 1
echo PORTABLE_OK
exit /b 0
