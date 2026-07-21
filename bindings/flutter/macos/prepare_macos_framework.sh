#!/bin/sh
set -eu

# CocoaPods copies the shallow publish layout into its XCFramework intermediate
# directory. Restore Apple's versioned macOS framework topology there, before
# the aggregate application embeds and signs the framework.
framework=""
if [ -n "${PODS_XCFRAMEWORKS_BUILD_DIR:-}" ]; then
  for candidate in \
    "$PODS_XCFRAMEWORKS_BUILD_DIR/mdstream_flutter/MdstreamFFI.framework" \
    "$PODS_XCFRAMEWORKS_BUILD_DIR"/*/MdstreamFFI.framework
  do
    if [ -d "$candidate" ]; then
      framework="$candidate"
      break
    fi
  done
fi

if [ -z "$framework" ]; then
  source_root="${PODS_TARGET_SRCROOT:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}"
  for candidate in "$source_root"/MdstreamFFI.xcframework/*/*.framework; do
    if [ -d "$candidate" ]; then
      framework="$candidate"
      break
    fi
  done
fi

if [ -z "$framework" ]; then
  echo "mdstream_flutter: copied MdstreamFFI.framework was not found" >&2
  exit 1
fi

if [ -L "$framework/Versions/Current" ]; then
  exit 0
fi

if [ -e "$framework/Versions" ]; then
  echo "mdstream_flutter: unsupported partially versioned framework: $framework" >&2
  exit 1
fi

for required in MdstreamFFI Info.plist Headers Modules; do
  if [ ! -e "$framework/$required" ]; then
    echo "mdstream_flutter: shallow framework is missing $required" >&2
    exit 1
  fi
done

mkdir -p "$framework/Versions/A/Resources"
mv "$framework/MdstreamFFI" "$framework/Versions/A/MdstreamFFI"
mv "$framework/Headers" "$framework/Versions/A/Headers"
mv "$framework/Modules" "$framework/Versions/A/Modules"
mv "$framework/Info.plist" "$framework/Versions/A/Resources/Info.plist"

ln -s A "$framework/Versions/Current"
ln -s Versions/Current/MdstreamFFI "$framework/MdstreamFFI"
ln -s Versions/Current/Headers "$framework/Headers"
ln -s Versions/Current/Modules "$framework/Modules"
ln -s Versions/Current/Resources "$framework/Resources"
