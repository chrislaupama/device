cmake_minimum_required(VERSION 3.25)
project(saine_engine VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
  set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

option(SAINE_ENGINE_BUILD_TESTS "Build engine tests" ${PROJECT_IS_TOP_LEVEL})
option(SAINE_ENGINE_BUILD_BENCH "Build engine benchmark" ${PROJECT_IS_TOP_LEVEL})
option(SAINE_ENGINE_BUILD_TOOLS "Build manual check tools" ${PROJECT_IS_TOP_LEVEL})
option(SAINE_ENGINE_SANITIZE "Build everything with AddressSanitizer + UndefinedBehaviorSanitizer" OFF)
option(SAINE_ENGINE_WERROR "Treat warnings as errors" OFF)
option(SAINE_ENGINE_BUILD_SCANNER "Build the out-of-process plugin scanner" ON)
option(SAINE_ENGINE_BUILD_TEST_PLUGINS "Build the test plugin bundles (SaineTest*.vst3/.clap)" ${PROJECT_IS_TOP_LEVEL})

if(APPLE)
  enable_language(OBJCXX)
  set(CMAKE_OBJCXX_STANDARD 23)
  set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
  set(CMAKE_OBJCXX_EXTENSIONS OFF)
endif()

# ---------------------------------------------------------------- third-party SDKs
# Fetched at configure time at pinned tags and never vendored into git
# (see third_party_licenses.md). Only headers/sources needed for hosting and
# for the test plugins are used; nothing is added via add_subdirectory.
# Set FETCHCONTENT_BASE_DIR to share one download between build trees, or
# FETCHCONTENT_SOURCE_DIR_<NAME> to use a local checkout (offline builds).
include(FetchContent)
set(SAINE_CLAP_TAG 1.2.10)
set(SAINE_VST3SDK_TAG v3.8.1_build_84)
FetchContent_Declare(saine_clap
  GIT_REPOSITORY https://github.com/free-audio/clap.git
  GIT_TAG ${SAINE_CLAP_TAG}
  GIT_SHALLOW TRUE
  SOURCE_SUBDIR _saine_headers_only)
FetchContent_Declare(saine_vst3sdk
  GIT_REPOSITORY https://github.com/steinbergmedia/vst3sdk.git
  GIT_TAG ${SAINE_VST3SDK_TAG}
  GIT_SHALLOW TRUE
  GIT_SUBMODULES pluginterfaces public.sdk
  SOURCE_SUBDIR _saine_sources_only)
FetchContent_MakeAvailable(saine_clap saine_vst3sdk)
set(SAINE_CLAP_INCLUDE "${saine_clap_SOURCE_DIR}/include")
set(SAINE_VST3SDK_DIR "${saine_vst3sdk_SOURCE_DIR}")
# IID definitions every VST3 module needs (host and test plugin).
set(SAINE_VST3_IID_SOURCES
  ${SAINE_VST3SDK_DIR}/pluginterfaces/base/funknown.cpp
  ${SAINE_VST3SDK_DIR}/pluginterfaces/base/coreiids.cpp
  ${SAINE_VST3SDK_DIR}/public.sdk/source/vst/vstinitiids.cpp
  ${SAINE_VST3SDK_DIR}/public.sdk/source/common/commoniids.cpp)
# SDK host helpers (IHostApplication / IMessage / IAttributeList).
set(SAINE_VST3_HOST_SOURCES
  ${SAINE_VST3_IID_SOURCES}
  ${SAINE_VST3SDK_DIR}/public.sdk/source/vst/hosting/hostclasses.cpp
  ${SAINE_VST3SDK_DIR}/public.sdk/source/vst/hosting/pluginterfacesupport.cpp
  ${SAINE_VST3SDK_DIR}/public.sdk/source/vst/utility/stringconvert.cpp
  ${SAINE_VST3SDK_DIR}/public.sdk/source/common/commonstringconvert.cpp)
# Third-party code is compiled without our warning set: the host helpers
# in their own object library (no /W4 to override), the IID files of the
# test plugins with warnings off.
set_source_files_properties(${SAINE_VST3_HOST_SOURCES} PROPERTIES COMPILE_OPTIONS
  "$<IF:$<CXX_COMPILER_ID:MSVC>,/w,-w>")
add_library(saine_vst3_host OBJECT ${SAINE_VST3_HOST_SOURCES})
target_include_directories(saine_vst3_host SYSTEM PRIVATE ${SAINE_VST3SDK_DIR})
target_compile_features(saine_vst3_host PRIVATE cxx_std_23)
set_target_properties(saine_vst3_host PROPERTIES POSITION_INDEPENDENT_CODE ON)

# ---------------------------------------------------------------- warnings
add_library(saine_engine_warnings INTERFACE)
if(MSVC)
  # C4324 "structure was padded due to alignment specifier" is the intended
  # effect of alignas(64) on the lock-free queues (false-sharing padding).
  target_compile_options(saine_engine_warnings INTERFACE /W4 /permissive- /wd4324)
  if(SAINE_ENGINE_WERROR)
    target_compile_options(saine_engine_warnings INTERFACE /WX)
  endif()
else()
  target_compile_options(saine_engine_warnings INTERFACE
    -Wall -Wextra -Wpedantic -Wshadow -Wconversion -Wno-sign-conversion
    -Wnon-virtual-dtor -Wold-style-cast -Wcast-align -Woverloaded-virtual
    -Wnull-dereference -Wimplicit-fallthrough)
  if(SAINE_ENGINE_WERROR)
    target_compile_options(saine_engine_warnings INTERFACE -Werror)
  endif()
endif()

if(SAINE_ENGINE_SANITIZE)
  if(MSVC)
    message(FATAL_ERROR "SAINE_ENGINE_SANITIZE is only wired up for clang/gcc")
  endif()
  add_compile_options(-fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer)
  add_link_options(-fsanitize=address,undefined)
endif()

# ---------------------------------------------------------------- library
add_library(saine_engine STATIC
  src/c_api.cpp
  src/engine.cpp
  src/processor.cpp
  src/recorder.cpp
  src/render_graph.cpp
  src/resampler.cpp
  src/synth.cpp
  src/wav.cpp
  src/plugins/plugin_common.cpp
  src/plugins/plugin_host.cpp
  src/plugins/plugin_modules.cpp
  src/plugins/plugin_scan.cpp
  src/plugins/au_instance.cpp
  src/plugins/vst3_instance.cpp
  src/plugins/clap_instance.cpp
  src/devices/builtin.cpp
  src/devices/compressor.cpp
  src/devices/dsp.cpp
  src/devices/autopanner.cpp
  src/devices/character_mod.cpp
  src/devices/chorus.cpp
  src/devices/flanger.cpp
  src/devices/phaser.cpp
  src/devices/ring_mod.cpp
  src/devices/tremolo.cpp
  src/devices/delay.cpp
  src/devices/echo.cpp
  src/devices/grains.cpp
  src/devices/pitch_shift.cpp
  src/devices/pitch_shifter.cpp
  src/devices/reverb.cpp
  $<TARGET_OBJECTS:saine_vst3_host>
  # Platform backends: each file is compile-guarded so exactly one is active.
  src/platform/audio_device.cpp
  src/platform/coreaudio_device.cpp
  src/platform/pipewire_device.cpp
  src/platform/wasapi_device.cpp
  src/platform/os.cpp
)
if(WIN32)
  target_sources(saine_engine PRIVATE src/platform/plugin_editor_win.cpp)
elseif(NOT APPLE)
  target_sources(saine_engine PRIVATE src/platform/plugin_editor_stub.cpp)
endif()
if(APPLE)
  target_sources(saine_engine PRIVATE src/platform/plugin_editor_mac.mm)
  # ARC; bridging casts are C-style by necessity.
  set_source_files_properties(src/platform/plugin_editor_mac.mm PROPERTIES
    COMPILE_OPTIONS "-fobjc-arc;-Wno-old-style-cast;-Wno-cast-align")
endif()
target_include_directories(saine_engine
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
  PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src)
# SDK headers are third-party: include them as system headers (no warnings).
target_include_directories(saine_engine SYSTEM PRIVATE ${SAINE_CLAP_INCLUDE} ${SAINE_VST3SDK_DIR})
target_compile_features(saine_engine PUBLIC cxx_std_23)
set_target_properties(saine_engine PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_link_libraries(saine_engine PRIVATE $<BUILD_INTERFACE:saine_engine_warnings>)
if(NOT MSVC)
  # DSP code: flag accidental float->double promotion (slow paths, surprises).
  # Not applied to tests/bench/tools, which compare in double on purpose.
  target_compile_options(saine_engine PRIVATE $<$<COMPILE_LANGUAGE:CXX>:-Wdouble-promotion>)
endif()

if(APPLE)
  target_link_libraries(saine_engine PUBLIC
    "-framework AudioToolbox" "-framework AudioUnit" "-framework CoreAudio" "-framework CoreFoundation"
    "-framework CoreAudioKit" "-framework AppKit" "-framework Foundation")
endif()
if(WIN32)
  # ole32: COM (WASAPI, MMDevice); shell32: CommandLineToArgvW; avrt: MMCSS;
  # user32: plugin editor windows.
  target_link_libraries(saine_engine PUBLIC ole32 shell32 avrt user32)
elseif(NOT APPLE)
  target_link_libraries(saine_engine PUBLIC ${CMAKE_DL_LIBS})
endif()
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
  # Audio backend (src/platform/pipewire_device.cpp). OFF builds an engine
  # without device I/O (offline rendering only).
  option(SAINE_ENGINE_PIPEWIRE "Build the PipeWire audio backend (needs libpipewire-0.3 development files)" ON)
  if(SAINE_ENGINE_PIPEWIRE)
    find_package(PkgConfig REQUIRED)
    pkg_check_modules(PIPEWIRE REQUIRED IMPORTED_TARGET libpipewire-0.3)
    target_link_libraries(saine_engine PUBLIC PkgConfig::PIPEWIRE)
    target_compile_definitions(saine_engine PRIVATE SAINE_HAVE_PIPEWIRE=1)
  endif()
endif()
find_package(Threads REQUIRED)
target_link_libraries(saine_engine PUBLIC Threads::Threads)

# Installed layout for the Rust `cmake` crate (it runs the install target):
#   <prefix>/lib/libsaine_engine.a, <prefix>/include/saine_engine.h
install(TARGETS saine_engine ARCHIVE DESTINATION lib)
install(FILES include/saine_engine.h DESTINATION include)

# ---------------------------------------------------------------- plugin scanner
# Out-of-process scanner (see README "Plugins > Scanning"). The Rust crate
# finds it at the path baked in by build.rs, or via SAINE_PLUGIN_SCANNER.
if(SAINE_ENGINE_BUILD_SCANNER)
  add_executable(saine_plugin_scanner scanner/scanner_main.cpp)
  target_include_directories(saine_plugin_scanner PRIVATE src)
  target_include_directories(saine_plugin_scanner SYSTEM PRIVATE ${SAINE_CLAP_INCLUDE} ${SAINE_VST3SDK_DIR})
  target_link_libraries(saine_plugin_scanner PRIVATE saine_engine saine_engine_warnings)
endif()

# ---------------------------------------------------------------- test plugins
# Minimal original plugins, built as proper bundles into <build>/test_plugins:
# SaineTestGain.vst3, SaineTestGain.clap, SaineTestSine.clap,
# SaineTestLatency.clap/.vst3 (delay by N frames, report latency N) and
# SaineTestCrash.clap (crashes in clap_entry.init; scanner blocklist test).
set(SAINE_TEST_PLUGIN_DIR "${CMAKE_CURRENT_BINARY_DIR}/test_plugins")
if(SAINE_ENGINE_BUILD_TEST_PLUGINS AND APPLE)
  function(saine_test_bundle target output ext)
    add_library(${target} MODULE ${ARGN})
    set_target_properties(${target} PROPERTIES
      BUNDLE TRUE
      BUNDLE_EXTENSION ${ext}
      OUTPUT_NAME ${output}
      LIBRARY_OUTPUT_DIRECTORY "${SAINE_TEST_PLUGIN_DIR}"
      MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/test_plugins/Info.plist.in"
      MACOSX_BUNDLE_BUNDLE_NAME ${output}
      MACOSX_BUNDLE_GUI_IDENTIFIER "studio.saine.test.${output}.${ext}"
      CXX_VISIBILITY_PRESET hidden
      VISIBILITY_INLINES_HIDDEN ON)
    target_link_libraries(${target} PRIVATE "-framework CoreFoundation")
  endfunction()
  saine_test_bundle(SaineTestGainClap SaineTestGain clap test_plugins/saine_test_clap.cpp)
  target_compile_definitions(SaineTestGainClap PRIVATE SAINE_TEST_CLAP_GAIN=1)
  saine_test_bundle(SaineTestSineClap SaineTestSine clap test_plugins/saine_test_clap.cpp)
  target_compile_definitions(SaineTestSineClap PRIVATE SAINE_TEST_CLAP_SINE=1)
  saine_test_bundle(SaineTestCrashClap SaineTestCrash clap test_plugins/saine_test_crash.cpp)
  saine_test_bundle(SaineTestLatencyClap SaineTestLatency clap test_plugins/saine_test_latency_clap.cpp)
  foreach(t SaineTestGainClap SaineTestSineClap SaineTestCrashClap SaineTestLatencyClap)
    target_include_directories(${t} SYSTEM PRIVATE ${SAINE_CLAP_INCLUDE})
  endforeach()
  saine_test_bundle(SaineTestGainVst3 SaineTestGain vst3 test_plugins/saine_test_vst3.cpp ${SAINE_VST3_IID_SOURCES})
  target_include_directories(SaineTestGainVst3 SYSTEM PRIVATE ${SAINE_VST3SDK_DIR})
  saine_test_bundle(SaineTestLatencyVst3 SaineTestLatency vst3 test_plugins/saine_test_latency_vst3.cpp
    ${SAINE_VST3_IID_SOURCES})
  target_include_directories(SaineTestLatencyVst3 SYSTEM PRIVATE ${SAINE_VST3SDK_DIR})
  add_custom_target(saine_test_plugins DEPENDS SaineTestGainClap SaineTestSineClap SaineTestCrashClap SaineTestGainVst3
    SaineTestLatencyClap SaineTestLatencyVst3)
elseif(SAINE_ENGINE_BUILD_TEST_PLUGINS)
  # Windows / Linux: a CLAP is a single shared library named *.clap; a VST3
  # is a bundle folder <name>.vst3/Contents/<arch>-win/<name>.vst3 (DLL) or
  # <name>.vst3/Contents/<arch>-linux/<name>.so. "$<1:...>" keeps
  # multi-config generators from appending a per-config subfolder.
  if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(AMD64|x86_64|amd64|x64)$")
    set(_saine_vst3_arch x86_64)
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(ARM64|arm64|aarch64)$")
    if(WIN32)
      set(_saine_vst3_arch arm64)
    else()
      set(_saine_vst3_arch aarch64)
    endif()
  else()
    set(_saine_vst3_arch ${CMAKE_SYSTEM_PROCESSOR})
  endif()
  function(saine_test_module target output dir suffix)
    add_library(${target} MODULE ${ARGN})
    set_target_properties(${target} PROPERTIES
      PREFIX ""
      SUFFIX ${suffix}
      OUTPUT_NAME ${output}
      LIBRARY_OUTPUT_DIRECTORY "$<1:${dir}>"
      RUNTIME_OUTPUT_DIRECTORY "$<1:${dir}>"
      CXX_VISIBILITY_PRESET hidden
      VISIBILITY_INLINES_HIDDEN ON)
    if(MSVC)
      target_link_options(${target} PRIVATE /NOIMPLIB /NOEXP)
    endif()
  endfunction()
  function(saine_test_clap target output)
    saine_test_module(${target} ${output} "${SAINE_TEST_PLUGIN_DIR}" ".clap" ${ARGN})
    target_include_directories(${target} SYSTEM PRIVATE ${SAINE_CLAP_INCLUDE})
  endfunction()
  function(saine_test_vst3 target output)
    if(WIN32)
      set(_dir "${SAINE_TEST_PLUGIN_DIR}/${output}.vst3/Contents/${_saine_vst3_arch}-win")
      set(_suffix ".vst3")
    else()
      set(_dir "${SAINE_TEST_PLUGIN_DIR}/${output}.vst3/Contents/${_saine_vst3_arch}-linux")
      set(_suffix ".so")
    endif()
    saine_test_module(${target} ${output} "${_dir}" "${_suffix}" ${ARGN} ${SAINE_VST3_IID_SOURCES})
    target_include_directories(${target} SYSTEM PRIVATE ${SAINE_VST3SDK_DIR})
  endfunction()
  saine_test_clap(SaineTestGainClap SaineTestGain test_plugins/saine_test_clap.cpp)
  target_compile_definitions(SaineTestGainClap PRIVATE SAINE_TEST_CLAP_GAIN=1)
  saine_test_clap(SaineTestSineClap SaineTestSine test_plugins/saine_test_clap.cpp)
  target_compile_definitions(SaineTestSineClap PRIVATE SAINE_TEST_CLAP_SINE=1)
  saine_test_clap(SaineTestCrashClap SaineTestCrash test_plugins/saine_test_crash.cpp)
  saine_test_clap(SaineTestLatencyClap SaineTestLatency test_plugins/saine_test_latency_clap.cpp)
  saine_test_vst3(SaineTestGainVst3 SaineTestGain test_plugins/saine_test_vst3.cpp)
  saine_test_vst3(SaineTestLatencyVst3 SaineTestLatency test_plugins/saine_test_latency_vst3.cpp)
  add_custom_target(saine_test_plugins DEPENDS SaineTestGainClap SaineTestSineClap SaineTestCrashClap SaineTestGainVst3
    SaineTestLatencyClap SaineTestLatencyVst3)
  if(WIN32)
    # Win32 clap.gui for the editor host test (plugin_editor_window_win32_*).
    saine_test_clap(SaineTestGuiClap SaineTestGui test_plugins/saine_test_gui_clap.cpp)
    target_link_libraries(SaineTestGuiClap PRIVATE user32)
    add_dependencies(saine_test_plugins SaineTestGuiClap)
  endif()
endif()

# What the Rust crate builds (build.rs): library, scanner, CLAP test gain.
add_custom_target(saine_engine_rust ALL DEPENDS saine_engine)
if(SAINE_ENGINE_BUILD_SCANNER)
  add_dependencies(saine_engine_rust saine_plugin_scanner)
endif()
if(TARGET saine_test_plugins)
  add_dependencies(saine_engine_rust saine_test_plugins)
endif()

# ---------------------------------------------------------------- tests
if(SAINE_ENGINE_BUILD_TESTS)
  enable_testing()
  add_executable(saine_engine_tests
    tests/test_main.cpp
    tests/test_util.cpp
    tests/test_wav.cpp
    tests/test_resampler.cpp
    tests/test_processing.cpp
    tests/test_transport.cpp
    tests/test_resources.cpp
    tests/test_concurrency.cpp
    tests/test_rt_safety.cpp
    tests/test_features.cpp
    tests/test_midi.cpp
    tests/test_synth.cpp
    tests/test_recording.cpp
    tests/test_plugins.cpp
    tests/test_plugin_scan.cpp
    tests/test_pdc.cpp
    tests/test_automation.cpp
    tests/test_frame_ring.cpp
    tests/devices/test_builtin.cpp
    tests/devices/test_compressor.cpp
    tests/devices/test_modulation.cpp
    tests/devices/test_character_mod.cpp
    tests/devices/test_time_fx.cpp)
  target_include_directories(saine_engine_tests PRIVATE src tests)
  target_link_libraries(saine_engine_tests PRIVATE saine_engine saine_engine_warnings)
  if(CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
    # GCC 13 -O2 false positive (free-nonheap-object) on std::vector range
    # construction in test_plugins.cpp after constant-folding a loop bound.
    target_compile_options(saine_engine_tests PRIVATE -Wno-free-nonheap-object)
  endif()
  target_compile_definitions(saine_engine_tests PRIVATE
    SAINE_TEST_PLUGIN_DIR="${SAINE_TEST_PLUGIN_DIR}"
    SAINE_PLUGIN_SCANNER_PATH="$<TARGET_FILE:saine_plugin_scanner>")
  if(TARGET saine_test_plugins)
    add_dependencies(saine_engine_tests saine_test_plugins saine_plugin_scanner)
  endif()

  # Each TEST_CASE becomes its own CTest test. The list is produced by the
  # test binary itself after every build (cf. gtest_discover_tests).
  set(_list "${CMAKE_CURRENT_BINARY_DIR}/saine_engine_tests_list.cmake")
  set(_wrapper "${CMAKE_CURRENT_BINARY_DIR}/saine_engine_tests_include.cmake")
  add_custom_command(TARGET saine_engine_tests POST_BUILD
    COMMAND saine_engine_tests --write-ctest "${_list}" "$<TARGET_FILE:saine_engine_tests>"
    BYPRODUCTS "${_list}"
    COMMENT "Discovering saine_engine_tests test cases")
  file(WRITE "${_wrapper}"
    "if(EXISTS \"${_list}\")\n  include(\"${_list}\")\nelse()\n"
    "  add_test(saine_engine_tests_NOT_BUILT saine_engine_tests_NOT_BUILT)\nendif()\n")
  set_property(DIRECTORY APPEND PROPERTY TEST_INCLUDE_FILES "${_wrapper}")
endif()

# ---------------------------------------------------------------- bench
if(SAINE_ENGINE_BUILD_BENCH)
  add_executable(saine_engine_bench bench/bench_render.cpp)
  target_include_directories(saine_engine_bench PRIVATE src)
  target_link_libraries(saine_engine_bench PRIVATE saine_engine saine_engine_warnings)
  target_compile_definitions(saine_engine_bench PRIVATE SAINE_TEST_PLUGIN_DIR="${SAINE_TEST_PLUGIN_DIR}")
  if(TARGET saine_test_plugins)
    add_dependencies(saine_engine_bench saine_test_plugins)
  endif()
  add_executable(saine_device_bench bench/bench_devices.cpp)
  target_include_directories(saine_device_bench PRIVATE src)
  target_link_libraries(saine_device_bench PRIVATE saine_engine saine_engine_warnings)
endif()

# ---------------------------------------------------------------- tools
if(SAINE_ENGINE_BUILD_TOOLS)
  add_executable(play_sine_or_wav tools/play_sine_or_wav.cpp)
  target_include_directories(play_sine_or_wav PRIVATE src)
  target_link_libraries(play_sine_or_wav PRIVATE saine_engine saine_engine_warnings)
  add_executable(record_check tools/record_check.cpp)
  target_include_directories(record_check PRIVATE src)
  target_link_libraries(record_check PRIVATE saine_engine saine_engine_warnings)
  if(APPLE)
    # Manual: opens and closes an AU editor window (needs a GUI session).
    add_executable(plugin_editor_check tools/plugin_editor_check.mm)
    set_source_files_properties(tools/plugin_editor_check.mm PROPERTIES COMPILE_OPTIONS "-fobjc-arc;-Wno-old-style-cast")
    target_include_directories(plugin_editor_check PRIVATE src)
    target_link_libraries(plugin_editor_check PRIVATE saine_engine saine_engine_warnings)
  endif()
endif()
