# Image fixtures

`interlaced.png` is a generated two-color 32 × 24 image, encoded as an Adam7
palette PNG with ImageMagick. It exercises sparse passes and palette expansion.
It was generated for Plastext under MIT OR Apache-2.0 and is used here under MIT.

## Native image fixtures

These are generated 32 × 24 images containing two colored rectangles, under the
Plastext MIT OR Apache-2.0 licenses, used here under MIT. No photographs or
third-party artwork are included.

- `sample.avif`: AVIF encoded with Pillow/libavif.
- `sample.heic`: HEIC encoded with macOS ImageIO.
- `oriented.jpg`: ImageIO JPEG with EXIF orientation 6 (90° clockwise).
- `profile.png`: ImageIO PNG with a Display P3 color profile.

The decoder integration test compares cropped pixels with the overview, checks
orientation, and exercises the installed macOS codecs. Animation fixtures are
generated in a temporary directory during the test. Huge generated images and
raw benchmark logs stay in ignored `target/` directories.
