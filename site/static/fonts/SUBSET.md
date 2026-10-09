# Font subsets

The `.woff2` files here are Latin subsets of the upstream fonts (SIL OFL 1.1,
see `OFL.txt` and `Inter-LICENSE.txt`), made with fontTools to cut ~430 KB to
~145 KB. Kept ranges: Basic Latin + Latin-1, General Punctuation, arrows,
box drawing, geometric shapes, ✓ ✗ and a few math signs. All OpenType layout
features are kept.

    pyftsubset Font.woff2 --flavor=woff2 --layout-features='*' \
      --unicodes="U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+2000-206F,U+2074,U+20AC,U+2122,U+2190-21FF,U+2212,U+2215,U+2260,U+2264,U+2265,U+2500-257F,U+25A0-25FF,U+2713,U+2717,U+FEFF,U+FFFD"

If a page needs a glyph outside these ranges, re-subset from the upstream
release rather than from these files.
