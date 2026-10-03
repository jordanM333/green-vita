// Compile the production video module, including its real surface ownership,
// worker, timing and decoder adapter. Only the SDK functions are host fakes.
use std::{env, fs, path::PathBuf};
fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = here
        .join("../../src/streaming/video")
        .canonicalize()
        .unwrap();
    let mut module = String::new();
    for line in fs::read_to_string(source.join("mod.rs")).unwrap().lines() {
        if let Some(name) = line
            .strip_prefix("mod ")
            .or_else(|| line.strip_prefix("pub(crate) mod "))
            .and_then(|s| s.strip_suffix(';'))
        {
            module.push_str(&format!(
                "#[path = {:?}]\n",
                source.join(format!("{name}.rs"))
            ));
        }
        module.push_str(line);
        module.push('\n');
    }
    module.push_str(&format!(
        "#[cfg(test)]\n#[path = {:?}]\nmod pump_tests;\n",
        here.join("src/cases.rs")
    ));
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("video.rs"),
        module,
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=src/cases.rs");
    println!("cargo:rerun-if-changed=src/burst_replay.rs");
    println!("cargo:rerun-if-changed=src/codec_roundtrip.rs");
    let surface = here.join("../../src/shell/surface.rs").canonicalize().unwrap();
    let mut surface_source = fs::read_to_string(&surface).unwrap();
    surface_source.push_str(r#"
#[cfg(test)]
impl VitaSurface {
    pub(crate) fn software_fixture(video: &sdl2::VideoSubsystem) -> Self {
        let canvas = video.window("surface fixture", WIDTH, HEIGHT).hidden().build().unwrap()
            .into_canvas().software().build().unwrap();
        Self { canvas, video_textures: None, video_output_buffers: None,
            displayed_video_texture: None, displayed_video_timing: None, drew_video: false, held_video: false,
            pending_video_present: None, direct_video_output: None,
            video_width: 0, video_height: 0, egui_painter: SdlEguiPainter::default(), pending_probe: None }
    }
}
"#);
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("surface.rs"), surface_source).unwrap();
    println!("cargo:rerun-if-changed={}", surface.display());
}
