@echo off
setlocal

echo Building Release Binaries...

:: Ensure we are in the script's directory
cd /d "%~dp0"

:: Build Runner
echo Building zexe-runner...
cd zexe-runner
call "%USERPROFILE%\.cargo\bin\cargo" build --release
if %ERRORLEVEL% NEQ 0 (
    echo Failed to build zexe-runner
    exit /b %ERRORLEVEL%
)
cd ..

:: Build Bundler
echo Building zexe-bundler...
cd zexe-bundler
call "%USERPROFILE%\.cargo\bin\cargo" build --release
if %ERRORLEVEL% NEQ 0 (
    echo Failed to build zexe-bundler
    exit /b %ERRORLEVEL%
)
cd ..

:: Create Dist
if not exist dist mkdir dist

:: Copy Binaries
echo Copying binaries to dist...
copy "zexe-runner\target\release\zexe-runner.exe" "dist\zexe-runner.exe"
copy "zexe-bundler\target\release\zexe-bundler.exe" "dist\zexe-bundler.exe"

:: Re-bundle test to verify packing
echo Re-bundling test.exe for verification...
if exist test.z80 (
    dist\zexe-bundler.exe test.z80 --output dist\test.exe --runner dist\zexe-runner.exe
)

echo.
echo Build Complete!
echo Binaries are in the 'dist' folder.
echo.
pause
