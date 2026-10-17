//! Build script: compile the Win32 resource file (which embeds `logo.ico`)
//! and link it into the final executable. On MSVC this uses `rc.exe`; on GNU
//! it falls back to `windres`. The embedded group icon becomes the executable's
//! file icon and is also loadable at runtime via `LoadImageW(IDI_APP, ...)`.

fn main() {
    // Panics on failure (e.g. `rc.exe` not found). On success the compiled
    // resource is linked into the executable automatically.
    embed_resource::compile("res/app.rc", embed_resource::NONE);
}
