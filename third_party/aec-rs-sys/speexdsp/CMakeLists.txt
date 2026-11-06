cmake_minimum_required(VERSION 3.10)

project(speexdsp LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 14)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

option(BUILD_SHARED_LIBS "Build shared libraries" OFF)
# SimulVoice patch: floating point. smallft (used below instead of kiss_fft) only
# works in the floating-point build; fixed point requires speexdsp's own kiss_fft.
option(USE_FIXED_POINT "Use fixed-point arithmetic" OFF)

# Define compile-time options
if(USE_FIXED_POINT)
    add_compile_definitions(FIXED_POINT=1)
else()
    add_compile_definitions(FLOATING_POINT=1)
endif()

# 'M_PI': undeclared identifier
if(WIN32)
    add_compile_definitions(_USE_MATH_DEFINES=1)
endif()


# SimulVoice patch: use smallft instead of kiss_fft. sherpa-onnx links its own
# float kissfft with the same symbol names (kiss_fft, kiss_fftr, ...); with the
# fixed-point copy here the linker kept one of them and the echo canceller wrote
# past its buffers (heap corruption on duplex stop).
add_compile_definitions(EXPORT= USE_SMALLFT=1)

# Include directories
include_directories(
    ${CMAKE_SOURCE_DIR}/include
    ${CMAKE_SOURCE_DIR}/libspeexdsp
)

# Source files
file(GLOB SPEEXDSP_SOURCES libspeexdsp/*.c)
# Leave out kiss_fft (see above) and the test programs that define main()
list(FILTER SPEEXDSP_SOURCES EXCLUDE REGEX "(kiss_fft|kiss_fftr|test[a-z0-9]*)[.]c$")
add_library(speexdsp ${SPEEXDSP_SOURCES})

# Installation rules
install(TARGETS speexdsp
    EXPORT speexdspConfig
    RUNTIME DESTINATION bin
    LIBRARY DESTINATION lib
    ARCHIVE DESTINATION lib
)
install(DIRECTORY include/ DESTINATION include)
