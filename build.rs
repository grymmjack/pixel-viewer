//! Build the vendored **libxmp** (MIT) into a static library, so pixelview can play the extra
//! tracker formats `xmrs` doesn't (669/FAR/OKT/MED/AMF/ULT/MTM/STM). libxmp is self-contained
//! (its own depackers; only links libm) and needs no generated config header — `common.h`
//! self-detects the platform and the version lives in `include/xmp.h`, so a plain `cc` compile
//! works with no autotools/cmake step.
//!
//! We compile exactly the files libxmp's own build lists in `cmake/libxmp-sources.cmake` (the
//! full build = everything before the `LITE` variant) rather than globbing `src/**.c` — the tree
//! ships a couple of disabled/leftover ProWizard files (`pm.c`, `pp30.c`) that don't compile and
//! aren't in the real build.

use std::path::Path;

fn main() {
    let base = Path::new("vendor/libxmp");
    let list_path = base.join("cmake/libxmp-sources.cmake");
    let sources = std::fs::read_to_string(&list_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", list_path.display()));

    // The full build is everything before the reduced "LITE" list (whose entries are bare
    // filenames without a `src/` prefix and are ignored by the filter below anyway).
    let full = sources
        .split("LIBXMP_SRC_LIST_LITE")
        .next()
        .unwrap_or(&sources);

    let mut build = cc::Build::new();
    build
        .include(base.join("src"))
        .include(base.join("include"))
        .warnings(false); // third-party C — don't flood the log

    let mut count = 0usize;
    for line in full.lines() {
        let tok = line.trim();
        // Each source entry is a lone `src/…/foo.c` token on its line.
        if tok.starts_with("src/") && tok.ends_with(".c") {
            let file = base.join(tok);
            if file.is_file() {
                build.file(&file);
                count += 1;
            }
        }
    }
    assert!(
        count > 100,
        "libxmp source list parsed to only {count} files — libxmp-sources.cmake format changed?"
    );

    build.compile("xmp");
    println!("cargo:rustc-link-lib=m"); // libxmp's oscillators use the math library
    println!("cargo:rerun-if-changed=vendor/libxmp/src");
    println!("cargo:rerun-if-changed=vendor/libxmp/cmake/libxmp-sources.cmake");
}
