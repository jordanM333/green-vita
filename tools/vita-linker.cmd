@echo off
setlocal
rem Match the Unix linker wrapper: reserve space for SCE module metadata.
set "VITA_TOOL=arm-vita-eabi-gcc"
call "%~dp0vita-tool.cmd" -Wl,-T,"%~dp0vita-headroom.ld" %*
exit /b %ERRORLEVEL%
