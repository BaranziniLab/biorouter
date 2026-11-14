# Build rule 1 (DD-10). build.rs copies vendor/whisper.cpp/source into OUT_DIR and
# configures this project, never whisper.cpp itself. As a subdirectory, upstream sets
# WHISPER_STANDALONE OFF and skips the configure_file that would write into its own
# source tree (bindings/javascript/package.json), which the vendored subset does not have.
cmake_minimum_required(VERSION 3.14)
project(br_whisper C CXX)
add_subdirectory(${BR_WHISPER_SRC} whisper)
