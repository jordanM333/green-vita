cmake_minimum_required(VERSION 3.16)
if(NOT DEFINED CMAKE_TOOLCHAIN_FILE)
  set(CMAKE_TOOLCHAIN_FILE "$ENV{VITASDK}/share/vita.toolchain.cmake" CACHE PATH "Vita toolchain")
endif()
project(vita_tv_probe C)
include("${VITASDK}/share/vita.cmake" REQUIRED)
set(CMAKE_C_STANDARD 11)
add_executable(vita_tv_probe main.c)
target_compile_options(vita_tv_probe PRIVATE -Wall -Wextra -Werror -O2)
target_link_libraries(vita_tv_probe
  vita2d SceAvPlayer_stub SceAudio_stub SceAppUtil_stub SceAppMgr_stub
  SceDisplay_stub SceGxm_stub SceSysmodule_stub SceCtrl_stub ScePgf_stub
  ScePvf_stub SceCommonDialog_stub SceKernelModulemgr_stub
  freetype png jpeg z m c)
vita_create_self(eboot.bin vita_tv_probe UNSAFE)
vita_create_vpk(Vita-TV-Probe-0.1.vpk GVTVPRB01 eboot.bin
  VERSION 00.01 NAME "Vita TV Probe"
  FILE assets/icon0.png sce_sys/icon0.png
       assets/bg.png sce_sys/livearea/contents/bg.png
       assets/startup.png sce_sys/livearea/contents/startup.png
       assets/template.xml sce_sys/livearea/contents/template.xml
       assets/control.mp4 control.mp4
       assets/build-info.json build-info.json
       README.md README.md)
