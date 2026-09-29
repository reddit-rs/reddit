# btls-sys 0.5.6 skips its no-assembly workaround on native Windows ARM64.
# MSBuild cannot assemble BoringSSL's .S files (upstream btls issue #151).
set(OPENSSL_NO_ASM ON CACHE BOOL "Disable unsupported Windows ARM64 assembly" FORCE)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreaded CACHE STRING "Match Rust's static CRT" FORCE)
