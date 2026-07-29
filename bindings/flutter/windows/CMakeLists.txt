# Flutter 3.32 FFI plugin packaging for a prebuilt Rust dynamic library.
cmake_minimum_required(VERSION 3.14)

set(PROJECT_NAME "mdstream_flutter")
project(${PROJECT_NAME} LANGUAGES CXX)

string(TOLOWER "${CMAKE_SYSTEM_PROCESSOR}" MDSTREAM_SYSTEM_PROCESSOR)
if(MDSTREAM_SYSTEM_PROCESSOR MATCHES "^(x86_64|amd64)$")
  set(MDSTREAM_ARCH "x64")
else()
  message(FATAL_ERROR
    "mdstream_flutter 0.4 supports Windows x64 only; got ${CMAKE_SYSTEM_PROCESSOR}")
endif()

set(MDSTREAM_FFI_LIBRARY
  "${CMAKE_CURRENT_SOURCE_DIR}/lib/${MDSTREAM_ARCH}/mdstream_ffi.dll")
if(NOT EXISTS "${MDSTREAM_FFI_LIBRARY}")
  message(FATAL_ERROR
    "Missing ${MDSTREAM_FFI_LIBRARY}; run tool/build_native.py windows first")
endif()

set(mdstream_flutter_bundled_libraries
  "${MDSTREAM_FFI_LIBRARY}"
  PARENT_SCOPE
)
