@echo off
rem Builds Effect Forge (release) into target\release\effect_forge.exe.
rem   build.bat        build, then wait for a key
rem   build.bat run    build, then open Effect Forge
rem   build.bat quiet  build without waiting (for scripts)
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust isn't installed. Get it from https://rustup.rs, then run this again.
    if /i not "%~1"=="quiet" pause
    exit /b 1
)

tasklist /fi "imagename eq effect_forge.exe" 2>nul | find /i "effect_forge.exe" >nul
if not errorlevel 1 (
    echo Effect Forge is open. Close it first: the build can't replace a running exe.
    if /i not "%~1"=="quiet" pause
    exit /b 1
)

cargo build --release
if errorlevel 1 (
    echo.
    echo The build failed; the error is above.
    if /i not "%~1"=="quiet" pause
    exit /b 1
)

echo.
echo Built: %~dp0target\release\effect_forge.exe
if /i "%~1"=="run" (
    start "" "%~dp0target\release\effect_forge.exe"
    exit /b 0
)
if /i not "%~1"=="quiet" pause
exit /b 0
