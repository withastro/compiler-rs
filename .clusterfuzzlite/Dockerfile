FROM gcr.io/oss-fuzz-base/base-builder-rust
COPY . $SRC/compiler-rs
WORKDIR $SRC/compiler-rs
COPY .clusterfuzzlite/build.sh $SRC/
