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

# ---------------------------------------------------------------- warnings
add_library(saine_engine_warnings INTERFACE)
if(MSVC)
  target_compile_options(saine_engine_warnings INTERFACE /W4 /permissive-)
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
  src/render_graph.cpp
  src/resampler.cpp
  src/synth.cpp
  src/wav.cpp
  # Platform backends: each file is compile-guarded so exactly one is active.
  src/platform/audio_device.cpp
  src/platform/coreaudio_device.cpp
  src/platform/pipewire_device.cpp
  src/platform/wasapi_device.cpp
)
target_include_directories(saine_engine
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
  PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src)
target_compile_features(saine_engine PUBLIC cxx_std_23)
set_target_properties(saine_engine PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_link_libraries(saine_engine PRIVATE $<BUILD_INTERFACE:saine_engine_warnings>)
if(NOT MSVC)
  # DSP code: flag accidental float->double promotion (slow paths, surprises).
  # Not applied to tests/bench/tools, which compare in double on purpose.
  target_compile_options(saine_engine PRIVATE -Wdouble-promotion)
endif()

if(APPLE)
  target_link_libraries(saine_engine PUBLIC
    "-framework AudioToolbox" "-framework CoreAudio" "-framework CoreFoundation")
endif()
find_package(Threads REQUIRED)
target_link_libraries(saine_engine PUBLIC Threads::Threads)

# Installed layout for the Rust `cmake` crate (it runs the install target):
#   <prefix>/lib/libsaine_engine.a, <prefix>/include/saine_engine.h
install(TARGETS saine_engine ARCHIVE DESTINATION lib)
install(FILES include/saine_engine.h DESTINATION include)

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
    tests/test_midi.cpp)
  target_include_directories(saine_engine_tests PRIVATE src tests)
  target_link_libraries(saine_engine_tests PRIVATE saine_engine saine_engine_warnings)

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
  target_link_libraries(saine_engine_bench PRIVATE saine_engine saine_engine_warnings)
endif()

# ---------------------------------------------------------------- tools
if(SAINE_ENGINE_BUILD_TOOLS)
  add_executable(play_sine_or_wav tools/play_sine_or_wav.cpp)
  target_link_libraries(play_sine_or_wav PRIVATE saine_engine saine_engine_warnings)
endif()
