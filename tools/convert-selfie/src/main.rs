//! `convert-selfie <model.tflite> [selfie.bin]`: MediaPipe's Selfie
//! Segmentation (landscape, 144x256) to Rustshot's compact model, which
//! the `selfie_model` feature embeds (`assets/selfie.bin` by default).
//! Weights are stored as f16 (float32 weights are rounded to it); the
//! operators and shapes are checked again by the app when it loads it
//! (`cargo test --features selfie_model embedded_model`).

mod flat;
#[path = "../../../src/cam/seg/format.rs"]
mod format;
mod tflite;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match &args[..] {
        [i] => (PathBuf::from(i), PathBuf::from("assets/selfie.bin")),
        [i, o] => (PathBuf::from(i), PathBuf::from(o)),
        _ => {
            eprintln!("usage: convert-selfie <selfie_segmentation_landscape.tflite> [out: assets/selfie.bin]");
            return ExitCode::from(2);
        }
    };
    let file = match std::fs::read(&input) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("convert-selfie: {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let (model, sum) = match tflite::convert(&file) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("convert-selfie: {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let bytes = model.to_bytes();
    // What the app will read back.
    if let Err(e) = format::Model::parse(&bytes) {
        eprintln!("convert-selfie: the written model does not read back: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&output, &bytes) {
        eprintln!("convert-selfie: {}: {e}", output.display());
        return ExitCode::FAILURE;
    }
    println!("{} -> {} ({} bytes)", input.display(), output.display(), bytes.len());
    println!("input {:?}, output {:?}, {} weights, {} DEQUANTIZE folded", sum.input, sum.output, sum.weights, sum.folded_dequantize);
    for (name, n) in &sum.ops {
        println!("  {name:<28} {n}");
    }
    ExitCode::SUCCESS
}
