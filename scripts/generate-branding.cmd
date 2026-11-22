@echo off
setlocal
pushd "%~dp0.."
if errorlevel 1 exit /b 1

cargo xtask branding
set "result=%errorlevel%"

popd
exit /b %result%
